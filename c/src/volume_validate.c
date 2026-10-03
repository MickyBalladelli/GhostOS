#include "ghostos/volume_validate.h"
static int order(const uint8_t *a, size_t al, uint32_t av,
    const uint8_t *b, size_t bl, uint32_t bv) {
    size_t i, length = al < bl ? al : bl;
    for (i = 0; i < length; ++i) if (a[i] != b[i]) return a[i] < b[i] ? -1 : 1;
    if (al != bl) return al < bl ? -1 : 1;
    return av < bv ? -1 : av > bv ? 1 : 0;
}
static int data(const ghostos_volume_mutation *volume, const ghostos_volume_record *record) {
    const ghostos_volume_tree *tree = &volume->tree;
    uint32_t id = record->data;
    uint64_t total = 0, checksum = 0xcbf29ce484222325ull;
    size_t steps = 0;
    while (id) {
        size_t length, i, index;
        uint32_t next;
        if (id > tree->block_count || steps++ == tree->block_count) return 1;
        index = (size_t)id - 1;
        if (tree->kinds[index] != 3 ||
            (volume->data_owners[index] && volume->data_owners[index] != record->object_id)) return 1;
        volume->data_owners[index] = record->object_id;
        if (ghostos_volume_decode_data(tree->blocks + index * GHOSTOS_VOLUME_BLOCK,
            GHOSTOS_VOLUME_BLOCK, &next, tree->payload, tree->payload_capacity, &length) ||
            !length || UINT64_MAX - total < length) return 1;
        total += length;
        for (i = 0; i < length; ++i) { checksum ^= tree->payload[i]; checksum *= 0x100000001b3ull; }
        id = next;
    }
    return total != record->size || checksum != record->checksum;
}
static int root(const ghostos_volume_mutation *volume, uint32_t id) {
    const ghostos_volume_tree *tree = &volume->tree;
    size_t i, pending = 0;
    if (!id) return 0;
    if (id > tree->block_count) return 1;
    for (i = 0; i < tree->block_count; ++i) {
        volume->marked[i] = false;
        volume->data_owners[i] = 0;
    }
    volume->marked[id - 1] = true;
    volume->pending[pending++] = id;
    while (pending) {
        ghostos_volume_tree_node node;
        uint8_t kind;
        id = volume->pending[--pending];
        kind = tree->kinds[id - 1];
        if ((kind != 1 && kind != 2) ||
            ghostos_volume_tree_decode(tree->blocks + (size_t)(id - 1) * GHOSTOS_VOLUME_BLOCK,
                GHOSTOS_VOLUME_BLOCK, tree->payload, tree->payload_capacity, &node) ||
            node.kind != kind || !node.length) return 1;
        if (kind == 2) {
            for (i = 1; i < node.length; ++i) {
                const ghostos_volume_tree_key *a = &node.entries.branch.keys[i - 1], *b = &node.entries.branch.keys[i];
                if (order(a->name, a->name_length, a->version, b->name, b->name_length, b->version) >= 0) return 1;
            }
            for (i = 0; i <= node.length; ++i) {
                uint32_t child = node.entries.branch.children[i];
                if (!child || child > tree->block_count || volume->marked[child - 1]) return 1;
                volume->marked[child - 1] = true;
                volume->pending[pending++] = child;
            }
        } else {
            for (i = 0; i < node.length; ++i) {
                const ghostos_volume_record *record = &node.entries.records[i];
                if (i) {
                    const ghostos_volume_record *previous = &node.entries.records[i - 1];
                    if (order(previous->name, previous->name_length, previous->version,
                        record->name, record->name_length, record->version) >= 0) return 1;
                }
                if (!record->version || !record->name_length || record->created_at > volume->generation) return 1;
                if (record->deleted) { if (record->size || record->data) return 1; continue; }
                if (!record->link_count) return 1;
                if (record->file_type == 2) {
                    if (record->object_id || record->size || record->data) return 1;
                } else if ((record->file_type != 1 && record->file_type != 3) ||
                    !record->object_id || data(volume, record)) return 1;
            }
        }
    }
    return 0;
}
int ghostos_volume_validate(const ghostos_volume_mutation *volume, uint64_t next_checkpoint) {
    size_t i, j;
    const ghostos_volume_tree *tree;
    if (!volume || !next_checkpoint || !volume->next_object ||
        (volume->root && !volume->generation) || (volume->pin_capacity && !volume->pins)) return 1;
    tree = &volume->tree;
    if (tree->block_count > UINT32_MAX || tree->block_count > SIZE_MAX / GHOSTOS_VOLUME_BLOCK ||
        (tree->block_count && (!tree->blocks || !tree->kinds)) ||
        (volume->limits.max_blocks != UINT64_MAX && volume->limits.max_blocks > tree->block_count)) return 1;
    if (volume->gc_capacity < tree->block_count || volume->owner_capacity < tree->block_count ||
        (tree->block_count && (!volume->marked || !volume->pending || !volume->data_owners)) ||
        !tree->payload || tree->payload_capacity < GHOSTOS_VOLUME_DATA) return 5;
    if (root(volume, volume->root)) return 1;
    for (i = 0; i < volume->pin_capacity; ++i) {
        const ghostos_volume_pin_slot *pin = &volume->pins[i];
        if (!pin->occupied) continue;
        if (!pin->id || pin->id >= next_checkpoint || pin->generation > volume->generation) return 1;
        for (j = 0; j < i; ++j) if (volume->pins[j].occupied && volume->pins[j].id == pin->id) return 1;
        if (root(volume, pin->root)) return 1;
    }
    return 0;
}
