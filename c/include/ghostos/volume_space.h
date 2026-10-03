#ifndef GHOSTOS_VOLUME_SPACE_H
#define GHOSTOS_VOLUME_SPACE_H
#include <stdint.h>
/* Free blocks are the smaller of the block limit and the volume capacity,
   minus the blocks already used. Free bytes are that count times 4096. */
void ghostos_volume_free_space(uint64_t max_blocks, uint64_t capacity_blocks, uint64_t used_blocks, uint64_t *free_blocks, uint64_t *free_bytes);
#endif
