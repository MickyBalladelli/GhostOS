#include "ghostos/volume_write.h"
static bool same_prefix(const ghostos_volume_write_record *record, const uint8_t *path, size_t length) {
    size_t i;
    if (record->name_length != length) return false;
    for (i = 0; i < length; ++i) if (record->name[i] != path[i]) return false;
    return true;
}
static int last_index(const ghostos_volume_write_record *records, size_t capacity, const uint8_t *path, size_t length) {
    size_t i;
    int found = -1;
    for (i = 0; i < capacity; ++i) {
        if (!records[i].occupied || !same_prefix(&records[i], path, length)) continue;
        if (found < 0 || records[i].version > records[found].version) found = (int)i;
    }
    return found;
}
static int latest_live(const ghostos_volume_write_record *records, size_t capacity, const uint8_t *path, size_t length) {
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
static int classify(const uint8_t *path, size_t length, size_t *name_length) {
    size_t semi = length, i;
    while (semi > 0) {
        semi -= 1;
        if (path[semi] == ';') break;
    }
    if (semi < length && path[semi] == ';') {
        if (semi + 1 == length) return 3;
        for (i = semi + 1; i < length; ++i) if (path[i] < '0' || path[i] > '9') return 3;
        if (invalid_path(path, semi)) return 4;
        return 3;
    }
    if (invalid_path(path, length)) return 4;
    *name_length = length;
    return 0;
}
static uint64_t checksum(const uint8_t *bytes, size_t length) {
    uint64_t hash = 0xcbf29ce484222325ull;
    size_t i;
    for (i = 0; i < length; ++i) {
        hash ^= bytes[i];
        hash *= 0x100000001b3ull;
    }
    return hash;
}
static int parent_directory(const ghostos_volume_write_record *records, size_t capacity, const uint8_t *path, size_t length) {
    size_t separator = length;
    while (separator > 0) {
        separator -= 1;
        if (path[separator] == '/') break;
    }
    if (separator == length || separator == 0) return 0;
    {
        int parent = latest_live(records, capacity, path, separator);
        if (parent < 0) return 1;
        if (records[parent].file_type != 2) return 2;
    }
    return 0;
}
static void store(ghostos_volume_write_record *record, const uint8_t *path, size_t name_length, const uint8_t *contents, size_t contents_length, uint32_t version, uint64_t object_id) {
    record->name = path;
    record->data = contents;
    record->name_length = (uint8_t)name_length;
    record->file_type = 1;
    record->version = version;
    record->object_id = object_id;
    record->size = contents_length;
    record->checksum = checksum(contents, contents_length);
    record->occupied = true;
    record->deleted = false;
}
int ghostos_volume_write(ghostos_volume_write_record *records, size_t capacity, const uint8_t *path, size_t path_length, const uint8_t *contents, size_t contents_length, uint64_t *generation, uint64_t *next_object) {
    size_t name_length = 0, slot;
    int status = classify(path, path_length, &name_length);
    int previous, parent;
    uint32_t version = 1;
    uint64_t object_id;
    bool reuse = false;
    if (status) return status;
    previous = last_index(records, capacity, path, name_length);
    if (previous >= 0 && !records[previous].deleted && records[previous].file_type == 2) return 2;
    parent = parent_directory(records, capacity, path, name_length);
    if (parent) return parent;
    if (previous >= 0 && !records[previous].deleted && records[previous].version == 1 && !records[previous].size && !records[previous].data) reuse = true;
    if (!reuse && previous >= 0) {
        if (records[previous].version == UINT32_MAX) return 6;
        version = records[previous].version + 1;
    }
    if (previous >= 0 && !records[previous].deleted) object_id = records[previous].object_id;
    else {
        if (*next_object == UINT64_MAX) return 6;
        object_id = *next_object;
    }
    if (reuse) slot = (size_t)previous;
    else {
        for (slot = 0; slot < capacity; ++slot) if (!records[slot].occupied) break;
        if (slot == capacity) return 5;
        if (previous < 0 || records[previous].deleted) *next_object += 1;
    }
    store(&records[slot], path, name_length, contents, contents_length, version, object_id);
    if (*generation < UINT64_MAX) *generation += 1;
    return 0;
}
