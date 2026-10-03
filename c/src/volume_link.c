#include "ghostos/volume_link.h"
static bool same_name(const ghostos_volume_link_record *record, const uint8_t *name, size_t name_length) {
    size_t i;
    if (record->name_length != name_length) return false;
    for (i = 0; i < name_length; ++i) if (record->name[i] != name[i]) return false;
    return true;
}
static bool contains_semicolon(const uint8_t *name, size_t name_length) {
    size_t i;
    for (i = 0; i < name_length; ++i) if (name[i] == ';') return true;
    return false;
}
static int latest_index(const ghostos_volume_link_record *records, size_t count, const uint8_t *name, size_t name_length) {
    size_t i;
    int found = -1;
    for (i = 0; i < count; ++i) {
        if (records[i].deleted || !records[i].name_length || !same_name(&records[i], name, name_length)) continue;
        if (found < 0 || records[i].version > records[found].version) found = (int)i;
    }
    return found;
}
static int parent_directory(const ghostos_volume_link_record *records, size_t count, const uint8_t *target, size_t target_length) {
    size_t separator = target_length;
    while (separator > 0) {
        separator -= 1;
        if (target[separator] == '/') break;
    }
    if (separator == target_length || separator == 0) return 0;
    {
        int parent = latest_index(records, count, target, separator);
        if (parent < 0) return 1;
        if (records[parent].file_type != 2) return 2;
    }
    return 0;
}
int ghostos_volume_link(ghostos_volume_link_record *records, size_t count, const uint8_t *source, size_t source_length, const uint8_t *target, size_t target_length, uint64_t *generation) {
    int source_index = latest_index(records, count, source, source_length);
    size_t slot, i;
    uint32_t version = 1;
    int parent;
    if (source_index < 0) return 1;
    if (records[source_index].file_type != 1 || !records[source_index].object_id) return 2;
    if (contains_semicolon(target, target_length)) return 3;
    if (latest_index(records, count, target, target_length) >= 0) return 4;
    parent = parent_directory(records, count, target, target_length);
    if (parent) return parent;
    for (i = 0; i < count; ++i) if (same_name(&records[i], target, target_length) && records[i].version >= version) {
        if (records[i].version == UINT32_MAX) return 6;
        version = records[i].version + 1;
    }
    for (slot = 0; slot < count; ++slot) if (!records[slot].name_length) break;
    if (slot == count) return 5;
    records[slot].name = target;
    records[slot].name_length = (uint8_t)target_length;
    records[slot].file_type = 1;
    records[slot].version = version;
    records[slot].object_id = records[source_index].object_id;
    records[slot].deleted = false;
    if (*generation < UINT64_MAX) *generation += 1;
    return 0;
}
int ghostos_volume_link_count(const ghostos_volume_link_record *records, size_t count, const uint8_t *path, size_t path_length, uint32_t *link_count) {
    int selected = latest_index(records, count, path, path_length);
    size_t i;
    uint32_t total = 0;
    if (selected < 0) return 1;
    if (!records[selected].object_id) {
        *link_count = 1;
        return 0;
    }
    for (i = 0; i < count; ++i) {
        int latest;
        if (!records[i].name_length || records[i].deleted || records[i].object_id != records[selected].object_id) continue;
        latest = latest_index(records, count, records[i].name, records[i].name_length);
        if (latest == (int)i && total < UINT32_MAX) total += 1;
    }
    *link_count = total;
    return 0;
}
