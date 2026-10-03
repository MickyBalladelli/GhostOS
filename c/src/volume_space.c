#include "ghostos/volume_space.h"
enum { GHOSTOS_VOLUME_BLOCK_BYTES = 4096 };
void ghostos_volume_free_space(uint64_t max_blocks, uint64_t capacity_blocks, uint64_t used_blocks, uint64_t *free_blocks, uint64_t *free_bytes) {
    uint64_t available = max_blocks < capacity_blocks ? max_blocks : capacity_blocks;
    *free_blocks = used_blocks >= available ? 0 : available - used_blocks;
    if (*free_blocks > UINT64_MAX / GHOSTOS_VOLUME_BLOCK_BYTES) *free_bytes = UINT64_MAX;
    else *free_bytes = *free_blocks * GHOSTOS_VOLUME_BLOCK_BYTES;
}
