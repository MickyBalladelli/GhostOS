#include "ghostos/volume_consistency.h"
static int block_index(uint32_t id, size_t block_count, size_t *index) {
    if (!id || id > block_count || block_count > GHOSTOS_VOLUME_CHECK_BLOCKS) return 1;
    *index = (size_t)id - 1;
    return 0;
}
static int validate_data(const ghostos_volume_check_block *blocks, size_t block_count, uint32_t id, uint64_t object_id, uint64_t *owners, bool *owner_set) {
    bool seen[GHOSTOS_VOLUME_CHECK_BLOCKS];
    size_t i;
    for (i = 0; i < block_count; ++i) seen[i] = false;
    while (id) {
        size_t index = 0;
        if (block_index(id, block_count, &index) || seen[index] || blocks[index].kind != 3) return 1;
        if (owner_set[index] && owners[index] != object_id) return 1;
        seen[index] = true;
        owner_set[index] = true;
        owners[index] = object_id;
        id = blocks[index].child[0];
    }
    return 0;
}
static int validate_tree(const ghostos_volume_check_block *blocks, size_t block_count, uint32_t id, uint64_t generation, bool *tree_seen, uint64_t *owners, bool *owner_set) {
    size_t index = 0, record;
    if (!id) return 0;
    if (block_index(id, block_count, &index) || tree_seen[index] || (blocks[index].kind != 1 && blocks[index].kind != 2)) return 1;
    tree_seen[index] = true;
    if (!blocks[index].length || blocks[index].length > 2) return 1;
    if (blocks[index].kind == 2) {
        uint8_t child;
        if (blocks[index].length == 2 && blocks[index].key_rank[0] >= blocks[index].key_rank[1]) return 1;
        for (child = 0; child <= blocks[index].length; ++child)
            if (validate_tree(blocks, block_count, blocks[index].child[child], generation, tree_seen, owners, owner_set)) return 1;
        return 0;
    }
    if (blocks[index].length == 2 && blocks[index].key_rank[0] >= blocks[index].key_rank[1]) return 1;
    for (record = 0; record < blocks[index].length; ++record) {
        if (!blocks[index].version[record] || !blocks[index].name_length[record] || blocks[index].name_length[record] > 192 ||
            blocks[index].created_at[record] > generation)
            return 1;
        if (blocks[index].deleted[record]) {
            if (blocks[index].size[record] || blocks[index].data[record]) return 1;
            continue;
        }
        if (!blocks[index].link_count[record]) return 1;
        if (blocks[index].file_type[record] == 2) {
            if (blocks[index].object_id[record] || blocks[index].size[record] || blocks[index].data[record]) return 1;
            continue;
        }
        if (blocks[index].file_type[record] != 1 && blocks[index].file_type[record] != 3) return 1;
        if (!blocks[index].object_id[record]) return 1;
        if (validate_data(blocks, block_count, blocks[index].data[record], blocks[index].object_id[record], owners, owner_set)) return 1;
    }
    return 0;
}
static int validate_root(const ghostos_volume_check_block *blocks, size_t block_count, uint32_t root, uint64_t generation) {
    bool tree_seen[GHOSTOS_VOLUME_CHECK_BLOCKS], owner_set[GHOSTOS_VOLUME_CHECK_BLOCKS];
    uint64_t owners[GHOSTOS_VOLUME_CHECK_BLOCKS];
    size_t i;
    for (i = 0; i < block_count; ++i) { tree_seen[i] = false; owner_set[i] = false; owners[i] = 0; }
    return validate_tree(blocks, block_count, root, generation, tree_seen, owners, owner_set);
}
int ghostos_volume_check(uint64_t generation, uint64_t next_checkpoint, uint64_t next_object_id, uint64_t max_blocks,
    uint32_t root, const ghostos_volume_checkpoint *checkpoints, size_t checkpoint_count,
    const ghostos_volume_check_block *blocks, size_t block_count) {
    size_t i, j;
    if (!next_checkpoint || !next_object_id || (max_blocks != UINT64_MAX && max_blocks > block_count) || (root && !generation)) return 1;
    for (i = 0; i < checkpoint_count; ++i) {
        if (!checkpoints[i].id || checkpoints[i].id >= next_checkpoint || checkpoints[i].generation > generation) return 1;
        for (j = 0; j < i; ++j) if (checkpoints[j].id == checkpoints[i].id) return 1;
        if (validate_root(blocks, block_count, checkpoints[i].root, generation)) return 1;
    }
    return validate_root(blocks, block_count, root, generation);
}
