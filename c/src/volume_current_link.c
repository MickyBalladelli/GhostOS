#include "ghostos/volume_current_link.h"
static bool same_prefix(const ghostos_volume_current_record *record, const uint8_t *path, size_t length) {
    size_t i;
    if (record->name_length != length) return false;
    for (i = 0; i < length; ++i) if (record->name[i] != path[i]) return false;
    return true;
}
static int extreme(const ghostos_volume_current_record *records, size_t count, const uint8_t *path, size_t length, bool live_only) {
    size_t i;
    int found = -1;
    for (i = 0; i < count; ++i) {
        if (!records[i].occupied || (live_only && records[i].deleted) || !same_prefix(&records[i], path, length)) continue;
        if (found < 0 || records[i].version > records[found].version) found = (int)i;
    }
    return found;
}
static int invalid_path(const uint8_t *path, size_t length) {
    size_t i = 0;
    if (!length || length > 192 || path[length - 1] == '/') return 1;
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
int ghostos_volume_current_link_count(const ghostos_volume_current_record *records, size_t count, const uint8_t *path, size_t path_length, uint32_t *link_count) {
    size_t name_length = 0, i;
    uint32_t version = 0, total = 0;
    bool latest = true;
    int selected = -1;
    int status = classify(path, path_length, &name_length, &version, &latest);
    if (status) return status;
    for (i = 0; i < count; ++i) {
        if (!records[i].occupied || records[i].deleted || !same_prefix(&records[i], path, name_length)) continue;
        if (!latest && records[i].version != version) continue;
        if (selected < 0 || records[i].version > records[selected].version) selected = (int)i;
    }
    if (selected < 0) return 1;
    for (i = 0; i < count; ++i) {
        if (!records[i].occupied || records[i].deleted || records[i].object_id != records[selected].object_id) continue;
        if (extreme(records, count, records[i].name, records[i].name_length, true) != (int)i) continue;
        if (extreme(records, count, records[i].name, records[i].name_length, false) != (int)i) continue;
        if (total < UINT32_MAX) total += 1;
    }
    *link_count = total;
    return 0;
}
