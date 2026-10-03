#include "ghostos/path_pattern.h"
#include "ghostos/volume_wildcard.h"
enum { GHOSTOS_VOLUME_WILDCARD_PATH = 192 };
static int compare_name(const uint8_t *left, size_t left_length, const uint8_t *right, size_t right_length) {
    size_t i, length = left_length < right_length ? left_length : right_length;
    for (i = 0; i < length; ++i) {
        if (left[i] < right[i]) return -1;
        if (left[i] > right[i]) return 1;
    }
    if (left_length < right_length) return -1;
    if (left_length > right_length) return 1;
    return 0;
}
static int compare_record(const ghostos_volume_wildcard_record *left, const ghostos_volume_wildcard_record *right) {
    int name = compare_name(left->name, left->name_length, right->name, right->name_length);
    if (name) return name;
    if (left->version < right->version) return -1;
    if (left->version > right->version) return 1;
    return 0;
}
static int invalid_path(const uint8_t *path, size_t length) {
    size_t i = 0;
    if (!length || length > GHOSTOS_VOLUME_WILDCARD_PATH || path[length - 1] == '/') return 1;
    if (path[0] == '/') i = 1;
    while (i < length) {
        size_t start = i;
        while (i < length && path[i] != '/') {
            if (path[i] == 0 || path[i] == ';') return 1;
            i += 1;
        }
        if (i == start || (i - start == 1 && path[start] == '.') || (i - start == 2 && path[start] == '.' && path[start + 1] == '.')) return 1;
        if (i < length) i += 1;
    }
    return 0;
}
static int classify(const uint8_t *pattern, size_t length, size_t *name_length, uint32_t *version, bool *latest) {
    size_t semi = length, i;
    uint32_t parsed = 0;
    bool magic = false;
    if (semi > 0) {
        while (semi > 0) {
            semi -= 1;
            if (pattern[semi] == ';') break;
        }
    }
    if (length && semi < length && pattern[semi] == ';') {
        if (semi + 1 == length) return 3;
        for (i = semi + 1; i < length; ++i) {
            if (pattern[i] < '0' || pattern[i] > '9') return 3;
            if (parsed > (UINT32_MAX - (uint32_t)(pattern[i] - '0')) / 10) return 3;
            parsed = parsed * 10 + (uint32_t)(pattern[i] - '0');
        }
        if (invalid_path(pattern, semi)) return 4;
        if (ghostos_pattern_parse(pattern, semi, &magic)) return 6;
        *name_length = semi;
        *version = parsed;
        *latest = parsed == 0;
        return 0;
    }
    if (invalid_path(pattern, length)) return 4;
    if (ghostos_pattern_parse(pattern, length, &magic)) return 6;
    *name_length = length;
    *latest = true;
    return 0;
}
static bool same_file(const ghostos_volume_wildcard_record *record, const uint8_t *name, size_t name_length) {
    size_t i;
    if (record->name_length != name_length) return false;
    for (i = 0; i < name_length; ++i) if (record->name[i] != name[i]) return false;
    return true;
}
static int emit(const ghostos_volume_wildcard_record *record, size_t cursor, const uint8_t *pattern, size_t pattern_length, ghostos_volume_wildcard_entry *entries, size_t capacity, size_t *written, size_t *next, bool *has_next) {
    size_t i;
    if (!ghostos_pattern_matches(pattern, pattern_length, record->name, record->name_length)) return 0;
    if (*written == capacity) {
        *has_next = true;
        *next = cursor;
        return 1;
    }
    for (i = 0; i < record->name_length; ++i) entries[*written].name[i] = record->name[i];
    entries[*written].name_length = record->name_length;
    entries[*written].version = record->version;
    entries[*written].cursor = cursor;
    *written += 1;
    return 0;
}
int ghostos_volume_expand_page(const ghostos_volume_wildcard_record *records, size_t count, const uint8_t *pattern, size_t pattern_length, size_t continuation, ghostos_volume_wildcard_entry *entries, size_t capacity, size_t *written, size_t *next, bool *has_next) {
    size_t name_length = 0, cursor = 0;
    uint32_t version = 0;
    bool latest = true, have_previous = false, have_latest = false, have_file = false;
    int status = classify(pattern, pattern_length, &name_length, &version, &latest);
    int previous = -1, held = -1;
    size_t held_cursor = 0;
    uint8_t file_length = 0;
    *written = 0;
    *next = 0;
    *has_next = false;
    if (status) return status;
    if (!capacity) return 5;
    if (continuation > UINT32_MAX) return 4;
    for (;;) {
        size_t i;
        int best = -1;
        for (i = 0; i < count; ++i) {
            if (!records[i].occupied || records[i].deleted) continue;
            if (have_previous && compare_record(&records[i], &records[previous]) <= 0) continue;
            if (best >= 0 && compare_record(&records[i], &records[best]) >= 0) continue;
            best = (int)i;
        }
        if (best < 0) break;
        previous = best;
        have_previous = true;
        if (cursor >= continuation) {
            if (latest) {
                if (!have_file || !same_file(&records[best], records[held].name, file_length)) {
                    if (have_latest && emit(&records[held], held_cursor, pattern, name_length, entries, capacity, written, next, has_next)) return 0;
                    have_file = true;
                    file_length = records[best].name_length;
                    have_latest = false;
                }
                if (ghostos_pattern_matches(pattern, name_length, records[best].name, records[best].name_length) && (!have_latest || records[held].version < records[best].version)) {
                    held = best;
                    held_cursor = cursor;
                    have_latest = true;
                }
            } else if (records[best].version == version && emit(&records[best], cursor, pattern, name_length, entries, capacity, written, next, has_next)) return 0;
        }
        if (cursor == UINT32_MAX) break;
        cursor += 1;
    }
    if (latest && have_latest) emit(&records[held], held_cursor, pattern, name_length, entries, capacity, written, next, has_next);
    return 0;
}
int ghostos_volume_expand(const ghostos_volume_wildcard_record *records, size_t count, const uint8_t *pattern, size_t pattern_length, ghostos_volume_wildcard_entry *entries, size_t capacity, size_t *written) {
    size_t next = 0;
    bool has_next = false;
    int status = ghostos_volume_expand_page(records, count, pattern, pattern_length, 0, entries, capacity, written, &next, &has_next);
    if (status) return status;
    if (has_next) return 5;
    return 0;
}
