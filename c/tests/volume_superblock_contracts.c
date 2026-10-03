#include "ghostos/volume_superblock.h"
#include <assert.h>
#include <string.h>
static void superblock_checksum_rejects_a_flipped_tail_and_sequence_zero(void) {
    uint8_t block[GHOSTOS_VOLUME_SUPERBLOCK];
    uint16_t version = 0;
    uint64_t sequence = 0, generation = 0;
    uint32_t root = 9;
    assert(!ghostos_volume_superblock_encode(4, 2, 2, 0, 1, 1, 99, 64, block, sizeof block));
    assert(!memcmp(block, "SYNFSVOL", 8));
    assert(!ghostos_volume_superblock_decode(block, sizeof block, 64, &version, &sequence, &generation, &root));
    assert(version == 4 && sequence == 2 && generation == 2 && !root);
    block[GHOSTOS_VOLUME_SUPERBLOCK - 1] ^= 1;
    assert(ghostos_volume_superblock_decode(block, sizeof block, 64, &version, &sequence, &generation, &root) == 1);
    assert(!ghostos_volume_superblock_encode(4, 0, 2, 0, 1, 1, 99, 64, block, sizeof block));
    assert(ghostos_volume_superblock_decode(block, sizeof block, 64, &version, &sequence, &generation, &root) == 1);
    assert(!ghostos_volume_superblock_encode(2, 2, 2, 0, 1, 1, 99, 64, block, sizeof block));
    assert(ghostos_volume_superblock_decode(block, sizeof block, 64, &version, &sequence, &generation, &root) == 1);
}
int main(void) {
    superblock_checksum_rejects_a_flipped_tail_and_sequence_zero();
    return 0;
}
