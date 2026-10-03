#include "ghostos/volume_rename.h"
static bool same_name(const ghostos_volume_rename_record *record, const uint8_t *path, size_t length) {
    size_t i;
    if (record->name_length != length) return false;
    for (i = 0; i < length; ++i) if (record->name[i] != path[i]) return false;
    return true;
}
static bool has_prefix(const uint8_t *path, size_t path_length, const uint8_t *prefix, size_t prefix_length) {
    size_t i;
    if (path_length <= prefix_length || path[prefix_length] != '/') return false;
    for (i = 0; i < prefix_length; ++i) if (path[i] != prefix[i]) return false;
    return true;
}
static int latest_live(const ghostos_volume_rename_record *records, size_t count, const uint8_t *path, size_t length) {
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
    if (!length || length > GHOSTOS_VOLUME_RENAME_PATH || path[length - 1] == '/') return 1;
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
static int parent_directory(const ghostos_volume_rename_record *records, size_t count, const uint8_t *path, size_t length) {
    size_t separator = length;
    while (separator > 0) {
        separator -= 1;
        if (path[separator] == '/') break;
    }
    if (separator == length || separator == 0) return 0;
    {
        int parent = latest_live(records, count, path, separator);
        if (parent < 0) return 1;
        if (records[parent].file_type != 2) return 2;
    }
    return 0;
}
static void copy_name(ghostos_volume_rename_record *record, const uint8_t *path, size_t length) {
    size_t i;
    for (i = 0; i < length; ++i) record->name[i] = path[i];
    record->name_length = (uint8_t)length;
}
static int target_name(const uint8_t *old_path, size_t old_length, const uint8_t *new_path, size_t new_length, const uint8_t *current, size_t current_length, uint8_t *output, size_t *output_length) {
    size_t i, suffix;
    if (current_length == old_length) {
        for (i = 0; i < new_length; ++i) output[i] = new_path[i];
        *output_length = new_length;
        return 0;
    }
    suffix = old_length + 1;
    if (new_length + 1 + (current_length - suffix) > GHOSTOS_VOLUME_RENAME_PATH) return 4;
    for (i = 0; i < new_length; ++i) output[i] = new_path[i];
    output[new_length] = '/';
    for (i = 0; i < current_length - suffix; ++i) output[new_length + 1 + i] = current[suffix + i];
    *output_length = new_length + 1 + (current_length - suffix);
    return 0;
}
static void tombstone(ghostos_volume_rename_record *record) {
    record->deleted = true;
    record->data = 0;
    record->size = 0;
    record->checksum = 0xcbf29ce484222325ull;
}
int ghostos_volume_rename(ghostos_volume_rename_record *records, size_t count, const uint8_t *old_path, size_t old_length, const uint8_t *new_path, size_t new_length, uint64_t *generation) {
    int source, parent;
    uint8_t source_type;
    size_t moved = 0;
    if (invalid_path(old_path, old_length) || invalid_path(new_path, new_length)) return 4;
    source = latest_live(records, count, old_path, old_length);
    if (source < 0) return 1;
    if (latest_live(records, count, new_path, new_length) >= 0) return 3;
    if (has_prefix(new_path, new_length, old_path, old_length)) return 4;
    parent = parent_directory(records, count, new_path, new_length);
    if (parent) return parent;
    source_type = records[source].file_type;
    for (;;) {
        size_t i;
        int candidate = -1;
        for (i = 0; i < count; ++i) {
            bool named;
            if (!records[i].occupied || records[i].deleted) continue;
            named = same_name(&records[i], old_path, old_length) || (source_type == 2 && has_prefix(records[i].name, records[i].name_length, old_path, old_length));
            if (!named || latest_live(records, count, records[i].name, records[i].name_length) != (int)i) continue;
            candidate = (int)i;
            break;
        }
        if (candidate < 0) break;
        {
            uint8_t renamed[GHOSTOS_VOLUME_RENAME_PATH];
            size_t renamed_length = 0, slot;
            int existing = -1;
            uint32_t version = 1;
            int status = target_name(old_path, old_length, new_path, new_length, records[candidate].name, records[candidate].name_length, renamed, &renamed_length);
            if (status) return status;
            if (latest_live(records, count, renamed, renamed_length) >= 0) return 3;
            for (slot = 0; slot < count; ++slot) if (records[slot].occupied && same_name(&records[slot], renamed, renamed_length) && records[slot].version == version) existing = (int)slot;
            if (existing < 0) {
                for (slot = 0; slot < count; ++slot) if (!records[slot].occupied) { existing = (int)slot; break; }
            }
            if (existing < 0) return 5;
            copy_name(&records[existing], renamed, renamed_length);
            records[existing].data = records[candidate].data;
            records[existing].file_type = records[candidate].file_type;
            records[existing].version = version;
            records[existing].object_id = records[candidate].object_id;
            records[existing].size = records[candidate].size;
            records[existing].checksum = records[candidate].checksum;
            records[existing].occupied = true;
            records[existing].deleted = false;
            tombstone(&records[candidate]);
            if (*generation < UINT64_MAX) *generation += 1;
            moved += 1;
        }
    }
    return moved ? 0 : 1;
}
