#include "ghostos/volume_mutation.h"
static void mark(ghostos_volume_mutation *volume, uint32_t id, size_t *count) {
    size_t index;
    if (!id || id > volume->tree.block_count) return;
    index = (size_t)id - 1;
    if (volume->marked[index]) return;
    volume->marked[index] = true;
    volume->pending[(*count)++] = id;
}
int ghostos_volume_mutation_collect(ghostos_volume_mutation *volume,
    size_t *live, size_t *freed) {
    size_t i, count = 0, kept = 0, released = 0;
    ghostos_volume_tree *tree;
    if (!volume || !live || !freed) return GHOSTOS_VOLUME_MUTATION_INVALID;
    tree = &volume->tree;
    if (tree->block_count > UINT32_MAX || tree->block_count > SIZE_MAX / GHOSTOS_VOLUME_BLOCK ||
        (tree->block_count && (!tree->blocks || !tree->kinds)) ||
        (volume->pin_capacity && !volume->pins)) return GHOSTOS_VOLUME_MUTATION_INVALID;
    if (volume->gc_capacity < tree->block_count ||
        (tree->block_count && (!volume->marked || !volume->pending)) ||
        !tree->payload || tree->payload_capacity < GHOSTOS_VOLUME_DATA) return GHOSTOS_VOLUME_MUTATION_SCRATCH;
    for (i = 0; i < tree->block_count; ++i) volume->marked[i] = false;
    mark(volume, volume->root, &count);
    for (i = 0; i < volume->pin_capacity; ++i)
        if (volume->pins[i].occupied) mark(volume, volume->pins[i].root, &count);
    while (count) {
        uint32_t id = volume->pending[--count];
        const uint8_t *raw = tree->blocks + (size_t)(id - 1) * GHOSTOS_VOLUME_BLOCK;
        uint8_t kind = tree->kinds[id - 1];
        if (!kind) continue;
        if (kind == 3) {
            uint32_t next;
            size_t length;
            if (ghostos_volume_decode_data(raw, GHOSTOS_VOLUME_BLOCK, &next,
                tree->payload, tree->payload_capacity, &length)) return 5;
            mark(volume, next, &count);
        } else if (kind == 1 || kind == 2) {
            ghostos_volume_tree_node node;
            if (ghostos_volume_tree_decode(raw, GHOSTOS_VOLUME_BLOCK,
                tree->payload, tree->payload_capacity, &node) || node.kind != kind) return 5;
            if (node.kind == 1) {
                for (i = 0; i < node.length; ++i)
                    if (!node.entries.records[i].deleted) mark(volume, node.entries.records[i].data, &count);
            } else {
                for (i = 0; i <= node.length; ++i) mark(volume, node.entries.branch.children[i], &count);
            }
        } else return 5;
    }
    for (i = 0; i < tree->block_count; ++i) {
        size_t byte;
        if (!tree->kinds[i]) continue;
        if (volume->marked[i]) { kept += 1; continue; }
        tree->kinds[i] = 0;
        for (byte = 0; byte < GHOSTOS_VOLUME_BLOCK; ++byte) tree->blocks[i * GHOSTOS_VOLUME_BLOCK + byte] = 0;
        released += 1;
    }
    *live = kept;
    *freed = released;
    return 0;
}
