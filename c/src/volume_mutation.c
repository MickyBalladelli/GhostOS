#include "ghostos/volume_mutation.h"
#include <limits.h>
static int rebuild(ghostos_volume_mutation *volume, ghostos_volume_reader *reader) {
    int status = ghostos_volume_reader_init(reader, volume->records, volume->record_count,
        volume->tree.blocks, volume->tree.kinds, volume->tree.block_count,
        volume->files, volume->file_capacity, volume->blocks, volume->block_capacity);
    if (status == 6) return GHOSTOS_VOLUME_MUTATION_SCRATCH;
    return status;
}
int ghostos_volume_mutation_init(ghostos_volume_mutation *volume, ghostos_volume_reader *reader) {
    size_t i;
    if (!volume || !reader || !volume->next_object || volume->record_count > INT_MAX ||
        volume->record_capacity > INT_MAX || volume->record_count > volume->record_capacity ||
        (volume->record_capacity && !volume->records) ||
        volume->tree.block_count > UINT32_MAX ||
        volume->tree.block_count > SIZE_MAX / GHOSTOS_VOLUME_BLOCK ||
        (volume->tree.block_count && (!volume->tree.blocks || !volume->tree.kinds)) ||
        volume->root > volume->tree.block_count || (!volume->root && volume->record_count))
        return GHOSTOS_VOLUME_MUTATION_INVALID;
    if (volume->quota_capacity < volume->record_capacity ||
        volume->file_capacity < volume->record_capacity ||
        volume->block_capacity < volume->tree.block_count ||
        (volume->record_capacity && (!volume->quota_records || !volume->files)) ||
        (volume->tree.block_count && !volume->blocks) ||
        !volume->tree.payload || volume->tree.payload_capacity < GHOSTOS_VOLUME_DATA ||
        (volume->tree.frame_capacity && !volume->tree.frames)) return GHOSTOS_VOLUME_MUTATION_SCRATCH;
    for (i = 0; i < volume->record_count; ++i) if (!volume->records[i].name_length) return 5;
    if (volume->root && (!volume->tree.kinds[volume->root - 1] || volume->tree.kinds[volume->root - 1] > 2)) return 5;
    return rebuild(volume, reader);
}
static bool same_name(const ghostos_volume_record *record, const uint8_t *name, size_t length) {
    size_t i;
    if (record->name_length != length) return false;
    for (i = 0; i < length; ++i) if (record->name[i] != name[i]) return false;
    for (i = length; i < GHOSTOS_VOLUME_NAME; ++i) if (record->name[i]) return false;
    return true;
}
static int last_record(const ghostos_volume_mutation *volume, const uint8_t *name, size_t length) {
    size_t i;
    int last = -1;
    for (i = 0; i < volume->record_count; ++i) {
        if (!same_name(&volume->records[i], name, length)) continue;
        if (last < 0 || volume->records[i].version > volume->records[last].version) last = (int)i;
    }
    return last;
}
static int quota(ghostos_volume_mutation *volume, bool new_file) {
    size_t i;
    uint64_t used = 0;
    for (i = 0; i < volume->record_count; ++i) {
        volume->quota_records[i] = (ghostos_volume_quota_record){
            .name = volume->records[i].name,
            .name_length = (uint8_t)volume->records[i].name_length,
            .size = volume->records[i].size, .occupied = true
        };
    }
    for (i = 0; i < volume->tree.block_count; ++i) if (volume->tree.kinds[i]) used += 1;
    return ghostos_volume_quota_enforce(volume->quota_records, volume->record_count,
        &volume->limits, used, volume->tree.block_count, 0, new_file) ? GHOSTOS_VOLUME_MUTATION_QUOTA : 0;
}
static int tree_result(int status) {
    if (status == 1) return 5;
    if (status == 2) return GHOSTOS_VOLUME_MUTATION_SCRATCH;
    if (status == 3) return GHOSTOS_VOLUME_MUTATION_ARENA_FULL;
    if (status == 4) return 5;
    if (status == 5) return 1;
    return status;
}
int ghostos_volume_mutation_write_empty(ghostos_volume_mutation *volume,
    ghostos_volume_reader *reader, const uint8_t *path, size_t length) {
    const ghostos_volume_record *found;
    ghostos_volume_record record = {0};
    uint8_t name[GHOSTOS_VOLUME_NAME];
    size_t name_length = length, i, parent = 0, slot;
    int previous, status;
    uint32_t root;
    bool replace = false;
    status = ghostos_volume_reader_lookup(reader, path, length, &found);
    if (status && status != 1) return status;
    if (!status && found->file_type == 3) {
        status = ghostos_volume_reader_lookup_following(reader, path, length, &found);
        if (status) return status;
        path = found->name;
        name_length = found->name_length;
    }
    if (!name_length || name_length > GHOSTOS_VOLUME_NAME) return 4;
    for (i = 0; i < name_length; ++i) {
        if (path[i] == ';') return 3;
        name[i] = path[i];
        if (path[i] == '/') parent = i;
    }
    previous = last_record(volume, name, name_length);
    if (previous >= 0 && !volume->records[previous].deleted && volume->records[previous].file_type == 2) return 2;
    if (parent) {
        status = ghostos_volume_reader_lookup_following(reader, name, parent, &found);
        if (status) return status;
        if (found->file_type != 2) return 2;
    }
    status = quota(volume, previous < 0);
    if (status) return status;
    record.version = 1;
    if (previous >= 0) {
        const ghostos_volume_record *old = &volume->records[previous];
        replace = !old->deleted && old->version == 1 && !old->size && !old->data;
        if (replace) record.version = old->version;
        else {
            if (old->version == UINT32_MAX) return GHOSTOS_VOLUME_MUTATION_OVERFLOW;
            record.version = old->version + 1;
        }
    }
    slot = replace ? (size_t)previous : volume->record_count;
    if (slot == volume->record_capacity) return GHOSTOS_VOLUME_MUTATION_SCRATCH;
    record.name_length = (uint16_t)name_length;
    for (i = 0; i < name_length; ++i) record.name[i] = name[i];
    record.created_at = volume->generation == UINT64_MAX ? UINT64_MAX : volume->generation + 1;
    record.checksum = 0xcbf29ce484222325ull;
    record.file_type = 1;
    record.link_count = 1;
    record.mode = 0666;
    if (previous >= 0 && !volume->records[previous].deleted) record.object_id = volume->records[previous].object_id;
    else {
        if (volume->next_object == UINT64_MAX) return GHOSTOS_VOLUME_MUTATION_OVERFLOW;
        record.object_id = volume->next_object++;
    }
    if (replace) status = ghostos_volume_tree_replace(&volume->tree, volume->root, &record, &root);
    else status = ghostos_volume_tree_insert(&volume->tree, volume->root, &record, &root);
    if (status) return tree_result(status);
    volume->records[slot] = record;
    if (!replace) volume->record_count += 1;
    volume->root = root;
    volume->generation = record.created_at;
    return rebuild(volume, reader);
}
