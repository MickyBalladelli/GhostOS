#include "ghostos/volume_remove.h"
static bool same_prefix(const ghostos_volume_remove_record *record, const uint8_t *path, size_t length) {
    size_t i;
    if (record->name_length != length) return false;
    for (i = 0; i < length; ++i) if (record->name[i] != path[i]) return false;
    return true;
}
static int latest_live(const ghostos_volume_remove_record *records, size_t count, const uint8_t *path, size_t length) {
    size_t i;
    int found = -1;
    for (i = 0; i < count; ++i) {
        if (!records[i].occupied || records[i].deleted || !same_prefix(&records[i], path, length)) continue;
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
static bool direct_child(const ghostos_volume_remove_record *record, const uint8_t *path, size_t length) {
    size_t i;
    if (record->name_length <= length + 1) return false;
    for (i = 0; i < length; ++i) if (record->name[i] != path[i]) return false;
    if (record->name[length] != '/') return false;
    for (i = length + 1; i < record->name_length; ++i) if (record->name[i] == '/') return false;
    return true;
}
int ghostos_volume_remove_directory(ghostos_volume_remove_record *records, size_t count, const uint8_t *path, size_t path_length, uint64_t *generation) {
    int directory;
    size_t i;
    if (invalid_path(path, path_length)) return 4;
    directory = latest_live(records, count, path, path_length);
    if (directory < 0) return 1;
    if (records[directory].file_type != 2) return 2;
    for (i = 0; i < count; ++i) {
        if (!records[i].occupied || records[i].deleted || !direct_child(&records[i], path, path_length)) continue;
        if (latest_live(records, count, records[i].name, records[i].name_length) == (int)i) return 3;
    }
    records[directory].deleted = true;
    records[directory].size = 0;
    records[directory].checksum = 0xcbf29ce484222325ull;
    if (*generation < UINT64_MAX) *generation += 1;
    return 0;
}
