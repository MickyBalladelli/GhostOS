#include "ghostos/volume_symlink.h"
enum { GHOSTOS_VOLUME_SYMLINK_DEPTH = 40, GHOSTOS_VOLUME_SYMLINK_PATH = 192 };
static bool same_prefix(const ghostos_volume_symlink_record *record, const uint8_t *path, size_t length) {
    size_t i;
    if (record->name_length != length) return false;
    for (i = 0; i < length; ++i) if (record->name[i] != path[i]) return false;
    return true;
}
static int latest_live(const ghostos_volume_symlink_record *records, size_t capacity, const uint8_t *path, size_t length) {
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
    if (!length || length > GHOSTOS_VOLUME_SYMLINK_PATH || path[length - 1] == '/') return 1;
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
static int invalid_target(const uint8_t *target, size_t length) {
    size_t i;
    if (!length || length > GHOSTOS_VOLUME_SYMLINK_PATH) return 1;
    for (i = 0; i < length; ++i) if (target[i] == 0 || target[i] == ';') return 1;
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
static int parent_directory(const ghostos_volume_symlink_record *records, size_t capacity, const uint8_t *path, size_t length) {
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
static int append_components(const uint8_t *source, size_t source_length, uint8_t *output, size_t *length) {
    size_t i = 0;
    while (i <= source_length) {
        size_t start = i, component;
        while (i < source_length && source[i] != '/') i += 1;
        component = i - start;
        if (component == 0 || (component == 1 && source[start] == '.')) {
        } else if (component == 2 && source[start] == '.' && source[start + 1] == '.') {
            if (*length > 1) {
                size_t end = *length - 1, slash = 0;
                bool found = false;
                while (end > 0) {
                    end -= 1;
                    if (output[end] == '/') { slash = end; found = true; break; }
                }
                *length = found ? slash + 1 : 1;
            }
        } else {
            size_t required = component + (*length > 1 ? 1 : 0);
            size_t copy;
            if (*length + required > GHOSTOS_VOLUME_SYMLINK_PATH) return 4;
            if (*length > 1) { output[*length] = '/'; *length += 1; }
            for (copy = 0; copy < component; ++copy) output[*length + copy] = source[start + copy];
            *length += component;
        }
        if (i >= source_length) break;
        i += 1;
    }
    return 0;
}
static int join_target(const uint8_t *parent, size_t parent_length, const uint8_t *target, size_t target_length, const uint8_t *suffix, size_t suffix_length, uint8_t *output, size_t *length) {
    int status;
    output[0] = '/';
    *length = 1;
    if (!target_length || target[0] != '/') {
        status = append_components(parent, parent_length, output, length);
        if (status) return status;
    }
    status = append_components(target, target_length, output, length);
    if (status) return status;
    return append_components(suffix, suffix_length, output, length);
}
int ghostos_volume_symlink(ghostos_volume_symlink_record *records, size_t capacity, const uint8_t *target, size_t target_length, const uint8_t *link, size_t link_length, uint64_t *generation, uint64_t *next_object) {
    size_t slot;
    int parent;
    if (invalid_target(target, target_length) || invalid_path(link, link_length)) return 4;
    if (latest_live(records, capacity, link, link_length) >= 0) return 5;
    parent = parent_directory(records, capacity, link, link_length);
    if (parent) return parent;
    for (slot = 0; slot < capacity; ++slot) if (!records[slot].occupied) break;
    if (slot == capacity) return 6;
    if (*next_object == UINT64_MAX) return 7;
    records[slot].name = link;
    records[slot].data = target;
    records[slot].name_length = (uint8_t)link_length;
    records[slot].file_type = 3;
    records[slot].version = 1;
    records[slot].object_id = *next_object;
    records[slot].size = target_length;
    records[slot].checksum = checksum(target, target_length);
    records[slot].occupied = true;
    records[slot].deleted = false;
    *next_object += 1;
    if (*generation < UINT64_MAX) *generation += 1;
    return 0;
}
int ghostos_volume_read_link(const ghostos_volume_symlink_record *records, size_t capacity, const uint8_t *path, size_t path_length, uint8_t *output, size_t output_capacity, size_t *read) {
    int index = latest_live(records, capacity, path, path_length);
    size_t i;
    if (index < 0) return 1;
    if (records[index].file_type != 3) return 3;
    if (records[index].size > output_capacity) return 8;
    for (i = 0; i < records[index].size; ++i) output[i] = records[index].data[i];
    *read = records[index].size;
    return 0;
}
int ghostos_volume_follow(const ghostos_volume_symlink_record *records, size_t capacity, const uint8_t *path, size_t path_length, uint8_t *resolved, size_t resolved_capacity, size_t *resolved_length) {
    uint8_t current[GHOSTOS_VOLUME_SYMLINK_PATH], next[GHOSTOS_VOLUME_SYMLINK_PATH];
    size_t length, depth, i;
    if (path_length > GHOSTOS_VOLUME_SYMLINK_PATH) return 4;
    for (i = 0; i < path_length; ++i) current[i] = path[i];
    length = path_length;
    for (depth = 0; depth < GHOSTOS_VOLUME_SYMLINK_DEPTH; ++depth) {
        size_t component_end = 1;
        int link = -1;
        size_t link_end = 0;
        while (component_end <= length) {
            if (component_end == length || current[component_end] == '/') {
                if (component_end > 1) {
                    int index = latest_live(records, capacity, current, component_end);
                    if (index >= 0 && records[index].file_type == 3) { link = index; link_end = component_end; break; }
                }
            }
            component_end += 1;
        }
        if (link < 0) {
            if (length > resolved_capacity) return 8;
            if (latest_live(records, capacity, current, length) < 0) return 1;
            for (i = 0; i < length; ++i) resolved[i] = current[i];
            *resolved_length = length;
            return 0;
        }
        {
            size_t parent_end = 0, scan = link_end, joined = 0;
            const uint8_t *target = records[link].data;
            int status;
            if (records[link].size > GHOSTOS_VOLUME_SYMLINK_PATH || !target) return 4;
            while (scan > 0) {
                scan -= 1;
                if (current[scan] == '/') { parent_end = scan; break; }
            }
            status = join_target(current, parent_end == 0 ? 1 : parent_end, target, (size_t)records[link].size, current + link_end, length - link_end, next, &joined);
            if (status) return status;
            for (i = 0; i < joined; ++i) current[i] = next[i];
            length = joined;
        }
    }
    return 9;
}
