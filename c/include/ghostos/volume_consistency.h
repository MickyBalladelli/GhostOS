#ifndef GHOSTOS_VOLUME_CONSISTENCY_H
#define GHOSTOS_VOLUME_CONSISTENCY_H
#include "ghostos/volume_superblock.h"
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 corrupt.
   Kind: empty=0, leaf=1, branch=2, data=3. File type: regular=1, directory=2, symlink=3.
   At most eight blocks are checked. */
#define GHOSTOS_VOLUME_CHECK_BLOCKS 8u
typedef struct {
    uint8_t kind, length, file_type[2];
    uint16_t name_length[2];
    uint32_t version[2], data[2], link_count[2], key_rank[2], child[3];
    uint64_t created_at[2], object_id[2], size[2];
    bool deleted[2];
} ghostos_volume_check_block;
int ghostos_volume_check(uint64_t generation, uint64_t next_checkpoint, uint64_t next_object_id, uint64_t max_blocks,
    uint32_t root, const ghostos_volume_checkpoint *checkpoints, size_t checkpoint_count,
    const ghostos_volume_check_block *blocks, size_t block_count);
#endif
