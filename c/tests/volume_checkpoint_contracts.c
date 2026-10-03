#include "ghostos/volume_superblock.h"
#include <assert.h>
static void checkpoint_id_is_kept_and_duplicates_are_rejected(void) {
    ghostos_volume_checkpoint checkpoint = {1, 2, 4};
    ghostos_volume_checkpoint duplicate[2] = {{1, 2, 4}, {1, 1, 1}};
    ghostos_volume_checkpoint readback;
    uint8_t block[GHOSTOS_VOLUME_SUPERBLOCK];
    size_t count = 0;
    assert(!ghostos_volume_superblock_encode(4, 2, 2, 0, 3, 1, 99, 64, block, sizeof block));
    assert(!ghostos_volume_superblock_set_checkpoints(block, sizeof block, 64, &checkpoint, 1));
    assert(!ghostos_volume_superblock_checkpoints(block, sizeof block, 64, &readback, 1, &count));
    assert(count == 1 && readback.id == 1 && readback.generation == 2 && readback.root == 4);
    checkpoint.id = 0;
    assert(ghostos_volume_superblock_set_checkpoints(block, sizeof block, 64, &checkpoint, 1) == 1);
    assert(!ghostos_volume_superblock_checkpoints(block, sizeof block, 64, &readback, 1, &count));
    assert(readback.id == 1);
    assert(ghostos_volume_superblock_set_checkpoints(block, sizeof block, 64, duplicate, 2) == 1);
    checkpoint.id = 3;
    checkpoint.generation = 2;
    assert(ghostos_volume_superblock_set_checkpoints(block, sizeof block, 64, &checkpoint, 1) == 1);
    checkpoint.id = 1;
    checkpoint.generation = 3;
    assert(ghostos_volume_superblock_set_checkpoints(block, sizeof block, 64, &checkpoint, 1) == 1);
}
int main(void) {
    checkpoint_id_is_kept_and_duplicates_are_rejected();
    return 0;
}
