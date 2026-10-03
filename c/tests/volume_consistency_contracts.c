#include "ghostos/volume_consistency.h"
#include <assert.h>
#include <string.h>
static void empty_root_is_consistent_and_shared_data_is_not(void) {
    ghostos_volume_check_block blocks[2];
    ghostos_volume_checkpoint checkpoints[2];
    memset(blocks, 0, sizeof blocks);
    memset(checkpoints, 0, sizeof checkpoints);
    assert(!ghostos_volume_check(5, 3, 1, UINT64_MAX, 0, 0, 0, blocks, 2));
    assert(ghostos_volume_check(5, 0, 1, UINT64_MAX, 0, 0, 0, blocks, 2) == 1);
    assert(ghostos_volume_check(0, 3, 1, UINT64_MAX, 1, 0, 0, blocks, 2) == 1);
    blocks[0].kind = 1;
    blocks[0].length = 1;
    blocks[0].version[0] = 1;
    blocks[0].name_length[0] = 6;
    blocks[0].created_at[0] = 5;
    blocks[0].object_id[0] = 7;
    blocks[0].data[0] = 2;
    blocks[0].link_count[0] = 1;
    blocks[0].file_type[0] = 1;
    blocks[1].kind = 3;
    assert(!ghostos_volume_check(5, 3, 1, UINT64_MAX, 1, 0, 0, blocks, 2));
    blocks[0].length = 2;
    blocks[0].version[1] = 1;
    blocks[0].name_length[1] = 4;
    blocks[0].created_at[1] = 5;
    blocks[0].object_id[1] = 8;
    blocks[0].data[1] = 2;
    blocks[0].link_count[1] = 1;
    blocks[0].file_type[1] = 1;
    blocks[0].key_rank[0] = 1;
    blocks[0].key_rank[1] = 2;
    assert(ghostos_volume_check(5, 3, 1, UINT64_MAX, 1, 0, 0, blocks, 2) == 1);
    checkpoints[0].id = 1;
    checkpoints[0].generation = 5;
    checkpoints[1].id = 1;
    checkpoints[1].generation = 5;
    blocks[0].length = 1;
    assert(ghostos_volume_check(5, 3, 1, UINT64_MAX, 0, checkpoints, 2, blocks, 2) == 1);
}
int main(void) {
    empty_root_is_consistent_and_shared_data_is_not();
    return 0;
}
