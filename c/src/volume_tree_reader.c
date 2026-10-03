#include "ghostos/volume_tree_reader.h"
#include <limits.h>
int ghostos_volume_tree_reader_init(const ghostos_volume_tree *tree, uint32_t root,
    const ghostos_volume_tree_reader_buffers *buffers,
    ghostos_volume_reader *reader, size_t *record_count) {
    size_t i, pending = 0, records = 0;
    int status;
    if (!tree || !buffers || !reader || !record_count ||
        tree->block_count > UINT32_MAX || tree->block_count > SIZE_MAX / GHOSTOS_VOLUME_BLOCK ||
        root > tree->block_count ||
        (tree->block_count && (!tree->blocks || !tree->kinds))) return 5;
    if (buffers->traversal_capacity < tree->block_count ||
        buffers->block_capacity < tree->block_count ||
        !tree->payload || tree->payload_capacity < GHOSTOS_VOLUME_DATA) return 6;
    if ((tree->block_count && (!buffers->pending || !buffers->visited || !buffers->blocks)) ||
        (buffers->record_capacity && !buffers->records) ||
        (buffers->file_capacity && !buffers->files)) return 5;
    for (i = 0; i < tree->block_count; ++i) buffers->visited[i] = false;
    if (root) {
        buffers->visited[root - 1] = true;
        buffers->pending[pending++] = root;
    }
    while (pending) {
        uint32_t id = buffers->pending[--pending];
        uint8_t kind = tree->kinds[id - 1];
        ghostos_volume_tree_node node;
        if ((kind != 1 && kind != 2) ||
            ghostos_volume_tree_decode(tree->blocks + (size_t)(id - 1) * GHOSTOS_VOLUME_BLOCK,
                GHOSTOS_VOLUME_BLOCK, tree->payload, tree->payload_capacity, &node) ||
            node.kind != kind) return 5;
        if (kind == 1) {
            if (node.length > buffers->record_capacity - records ||
                records > INT_MAX - node.length) return 6;
            for (i = 0; i < node.length; ++i) buffers->records[records++] = node.entries.records[i];
        } else {
            for (i = (size_t)node.length + 1; i; --i) {
                uint32_t child = node.entries.branch.children[i - 1];
                if (!child || child > tree->block_count || buffers->visited[child - 1]) return 5;
                buffers->visited[child - 1] = true;
                buffers->pending[pending++] = child;
            }
        }
    }
    status = ghostos_volume_reader_init(reader, buffers->records, records,
        tree->blocks, tree->kinds, tree->block_count, buffers->files,
        buffers->file_capacity, buffers->blocks, buffers->block_capacity);
    if (!status) *record_count = records;
    return status;
}
int ghostos_volume_tree_checkpoint_reader(const ghostos_volume_tree *tree,
    const ghostos_volume_pin_slot *pins, size_t pin_count, uint64_t checkpoint,
    const ghostos_volume_tree_reader_buffers *buffers,
    ghostos_volume_reader *reader, size_t *record_count) {
    size_t i;
    if (pin_count && !pins) return 5;
    for (i = 0; i < pin_count; ++i)
        if (pins[i].occupied && pins[i].id == checkpoint)
            return ghostos_volume_tree_reader_init(tree, pins[i].root, buffers, reader, record_count);
    return 1;
}
