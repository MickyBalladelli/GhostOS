#include "ghostos/fsd_wildcard.h"
#include "ghostos/path_pattern.h"
static bool same_name(const ghostos_fsd_record *left, const ghostos_fsd_record *right) {
    size_t i;
    if (left->name_length != right->name_length) return false;
    for (i = 0; i < left->name_length; ++i) if (left->name[i] != right->name[i]) return false;
    return true;
}
static int split_version(const uint8_t *pattern, size_t length, size_t *file_length, bool *latest, uint32_t *version) {
    size_t mark = length, i;
    uint32_t parsed = 0;
    bool digits = false;
    for (i = 0; i < length; ++i) if (pattern[i] == ';') mark = i;
    if (mark == length) { *file_length = length; *latest = true; return 0; }
    if (mark + 1 == length) return 1;
    for (i = mark + 1; i < length; ++i) {
        uint32_t digit;
        if (pattern[i] < '0' || pattern[i] > '9') return 1;
        digit = (uint32_t)(pattern[i] - '0');
        if (parsed > (UINT32_MAX - digit) / 10) return 1;
        parsed = parsed * 10 + digit;
        digits = true;
    }
    if (!digits) return 1;
    *file_length = mark;
    *latest = parsed == 0;
    *version = parsed;
    return 0;
}
static bool emit(size_t *matches, size_t capacity, size_t *written, size_t *next, bool *has_next, size_t index, uint32_t cursor) {
    if (*written == capacity) { *next = cursor; *has_next = true; return false; }
    matches[*written] = index;
    *written += 1;
    return true;
}
int ghostos_fsd_expand(const uint8_t *pattern, size_t pattern_length, const ghostos_fsd_record *records,
    size_t record_count, size_t continuation, size_t *matches, size_t capacity, size_t *count, size_t *next, bool *has_next) {
    size_t file_length = 0, written = 0, i;
    bool latest = true, magic = false, have_latest = false;
    uint32_t version = 0;
    size_t latest_index = 0;
    int status = split_version(pattern, pattern_length, &file_length, &latest, &version);
    if (status) return status;
    if (ghostos_pattern_parse(pattern, file_length, &magic) != GHOSTOS_PATTERN_OK) return 2;
    if (!capacity) return 3;
    *has_next = false;
    *next = 0;
    for (i = 0; i < record_count; ++i) {
        bool matches_name;
        if (records[i].deleted || records[i].cursor < continuation) continue;
        matches_name = ghostos_pattern_matches(pattern, file_length, records[i].name, records[i].name_length);
        if (!latest) {
            if (records[i].version == version && matches_name && !emit(matches, capacity, &written, next, has_next, i, records[i].cursor))
                break;
            continue;
        }
        if (have_latest && !same_name(&records[latest_index], &records[i])) {
            if (!emit(matches, capacity, &written, next, has_next, latest_index, records[latest_index].cursor)) {
                have_latest = false;
                break;
            }
            have_latest = false;
        }
        if (matches_name && (!have_latest || records[latest_index].version < records[i].version)) {
            latest_index = i;
            have_latest = true;
        }
    }
    if (latest && have_latest) emit(matches, capacity, &written, next, has_next, latest_index, records[latest_index].cursor);
    *count = written;
    if (!written && !continuation && !*has_next) return 4;
    return 0;
}
int ghostos_fsd_expand_delete(uint16_t rights, const uint8_t *pattern, size_t pattern_length, const ghostos_fsd_record *records,
    size_t record_count, size_t continuation, size_t *matches, size_t capacity, size_t *count, size_t *next, bool *has_next) {
    if ((rights & 14u) != 14u) return 5;
    return ghostos_fsd_expand(pattern, pattern_length, records, record_count, continuation, matches, capacity, count, next, has_next);
}
