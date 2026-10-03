#include "ghostos/volume_directory.h"
static bool same_prefix(const ghostos_volume_directory_record *record, const uint8_t *path, size_t length) {
    size_t i;
    if (record->name_length != length) return false;
    for (i = 0; i < length; ++i) if (record->name[i] != path[i]) return false;
    return true;
}
static int latest_index(const ghostos_volume_directory_record *records, size_t capacity, const uint8_t *path, size_t length) {
    size_t i;
    int found = -1;
    for (i = 0; i < capacity; ++i) {
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
static int create_one(ghostos_volume_directory_record *records, size_t capacity, const uint8_t *path, size_t length, uint64_t *generation) {
    size_t separator = length, slot, i;
    uint32_t version = 1;
    while (separator > 0) {
        separator -= 1;
        if (path[separator] == '/') break;
    }
    if (separator > 0) {
        int parent = latest_index(records, capacity, path, separator);
        if (parent < 0) return 1;
        if (records[parent].file_type != 2) return 2;
    }
    for (i = 0; i < capacity; ++i) if (records[i].occupied && same_prefix(&records[i], path, length) && records[i].version >= version) {
        if (records[i].version == UINT32_MAX) return 6;
        version = records[i].version + 1;
    }
    for (slot = 0; slot < capacity; ++slot) if (!records[slot].occupied) break;
    if (slot == capacity) return 5;
    records[slot].name = path;
    records[slot].name_length = (uint8_t)length;
    records[slot].file_type = 2;
    records[slot].version = version;
    records[slot].occupied = true;
    records[slot].deleted = false;
    if (*generation < UINT64_MAX) *generation += 1;
    return 0;
}
int ghostos_volume_create_directory(ghostos_volume_directory_record *records, size_t capacity, const uint8_t *path, size_t path_length, bool recursive, uint64_t *generation, size_t *created) {
    size_t i;
    int status;
    *created = 0;
    if (invalid_path(path, path_length)) return 3;
    if (latest_index(records, capacity, path, path_length) >= 0) return 4;
    if (recursive) {
        for (i = 1; i < path_length; ++i) if (path[i] == '/') {
            if (latest_index(records, capacity, path, i) >= 0) continue;
            status = create_one(records, capacity, path, i, generation);
            if (status) return status;
            *created += 1;
        }
    }
    status = create_one(records, capacity, path, path_length, generation);
    if (!status) *created += 1;
    return status;
}
