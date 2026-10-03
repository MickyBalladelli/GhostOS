#include "ghostos/volume_link_list.h"
enum { GHOSTOS_VOLUME_LINK_LIST_PATH = 192 };
static bool same_name(const ghostos_volume_link_list_record *record, const uint8_t *path, size_t length) {
    size_t i;
    if (record->name_length != length) return false;
    for (i = 0; i < length; ++i) if (record->name[i] != path[i]) return false;
    return true;
}
static int compare_record(const ghostos_volume_link_list_record *left, const ghostos_volume_link_list_record *right) {
    size_t i, length = left->name_length < right->name_length ? left->name_length : right->name_length;
    for (i = 0; i < length; ++i) {
        if (left->name[i] < right->name[i]) return -1;
        if (left->name[i] > right->name[i]) return 1;
    }
    if (left->name_length < right->name_length) return -1;
    if (left->name_length > right->name_length) return 1;
    if (left->version < right->version) return -1;
    if (left->version > right->version) return 1;
    return 0;
}
static int latest_live(const ghostos_volume_link_list_record *records, size_t count, const uint8_t *path, size_t length) {
    size_t i;
    int found = -1;
    for (i = 0; i < count; ++i) {
        if (!records[i].occupied || records[i].deleted || !same_name(&records[i], path, length)) continue;
        if (found < 0 || records[i].version > records[found].version) found = (int)i;
    }
    return found;
}
static int invalid_path(const uint8_t *path, size_t length) {
    size_t i = 0;
    if (!length || length > GHOSTOS_VOLUME_LINK_LIST_PATH || path[length - 1] == '/') return 1;
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
static int classify(const uint8_t *path, size_t length, size_t *name_length, uint32_t *version, bool *latest) {
    size_t semi = length, i;
    uint32_t parsed = 0;
    while (semi > 0) {
        semi -= 1;
        if (path[semi] == ';') break;
    }
    if (semi < length && path[semi] == ';') {
        if (semi + 1 == length) return 3;
        for (i = semi + 1; i < length; ++i) {
            if (path[i] < '0' || path[i] > '9') return 3;
            if (parsed > (UINT32_MAX - (uint32_t)(path[i] - '0')) / 10) return 3;
            parsed = parsed * 10 + (uint32_t)(path[i] - '0');
        }
        if (invalid_path(path, semi)) return 4;
        *name_length = semi;
        *version = parsed;
        *latest = parsed == 0;
        return 0;
    }
    if (invalid_path(path, length)) return 4;
    *name_length = length;
    *latest = true;
    return 0;
}
int ghostos_volume_list_links(const ghostos_volume_link_list_record *records, size_t count, const uint8_t *path, size_t path_length, ghostos_volume_link_list_entry *entries, size_t capacity, size_t *written) {
    size_t name_length = 0, i;
    uint32_t version = 0;
    bool latest = true, have_previous = false;
    int status = classify(path, path_length, &name_length, &version, &latest);
    int selected = -1, previous = -1;
    uint64_t object_id;
    if (status) return status;
    for (i = 0; i < count; ++i) {
        if (!records[i].occupied || records[i].deleted || !same_name(&records[i], path, name_length)) continue;
        if (!latest && records[i].version != version) continue;
        if (selected < 0 || records[i].version > records[selected].version) selected = (int)i;
    }
    if (selected < 0) return 1;
    object_id = records[selected].object_id;
    *written = 0;
    for (;;) {
        int best = -1;
        for (i = 0; i < count; ++i) {
            if (!records[i].occupied || records[i].deleted || records[i].object_id != object_id) continue;
            if (latest_live(records, count, records[i].name, records[i].name_length) != (int)i) continue;
            if (have_previous && compare_record(&records[i], &records[previous]) <= 0) continue;
            if (best >= 0 && compare_record(&records[i], &records[best]) >= 0) continue;
            best = (int)i;
        }
        if (best < 0) return 0;
        if (*written == capacity) return 5;
        for (i = 0; i < records[best].name_length; ++i) entries[*written].name[i] = records[best].name[i];
        entries[*written].name_length = records[best].name_length;
        entries[*written].version = 0;
        *written += 1;
        previous = best;
        have_previous = true;
    }
}
