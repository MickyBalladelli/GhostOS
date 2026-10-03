#include "ghostos/volume_block.h"
#include <assert.h>
#include <string.h>
static void data_and_tree_blocks_reject_a_flipped_checksum(void) {
    const uint8_t text[] = {'h', 'i'};
    const uint8_t leaf[] = {1, 0, 0, 0};
    uint8_t block[GHOSTOS_VOLUME_BLOCK];
    uint8_t payload[8];
    uint32_t next = 9;
    uint8_t kind = 0;
    size_t length = 0;
    assert(!ghostos_volume_encode_data(0, text, sizeof text, block, sizeof block));
    assert(!ghostos_volume_decode_data(block, sizeof block, &next, payload, sizeof payload, &length));
    assert(!next && length == 2 && !memcmp(payload, "hi", 2));
    block[16] ^= 1;
    assert(ghostos_volume_decode_data(block, sizeof block, &next, payload, sizeof payload, &length) == 1);
    assert(!ghostos_volume_encode_tree(1, leaf, sizeof leaf, block, sizeof block));
    assert(block[0] == 'S' && block[3] == 'T' && block[4] == 1 && block[5] == 1);
    assert(!ghostos_volume_decode_tree(block, sizeof block, &kind, payload, sizeof payload, &length));
    assert(kind == 1 && length == 4 && payload[0] == 1);
    block[8] ^= 1;
    assert(ghostos_volume_decode_tree(block, sizeof block, &kind, payload, sizeof payload, &length) == 1);
    memset(block, 0, sizeof block);
    assert(!ghostos_volume_decode_empty(block, sizeof block));
    block[GHOSTOS_VOLUME_BLOCK - 1] = 1;
    assert(ghostos_volume_decode_empty(block, sizeof block) == 1);
}
int main(void) {
    data_and_tree_blocks_reject_a_flipped_checksum();
    return 0;
}
