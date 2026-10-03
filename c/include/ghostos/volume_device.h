#ifndef GHOSTOS_VOLUME_DEVICE_H
#define GHOSTOS_VOLUME_DEVICE_H
#include "ghostos/volume_generation.h"
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 corrupt, 2 version overflow, 3 input/output, 4 interrupted.
   The type map and data blocks are written before the superblock. The active
   bank changes only after the durability flush. A flush marker is UINT64_MAX. */
int ghostos_volume_device_flush(ghostos_volume *volume, uint64_t generation, uint32_t version, uint32_t root,
    uint64_t next_checkpoint, uint64_t next_object_id, size_t block_count, uint64_t *log, size_t log_capacity,
    size_t *log_count, int fail_at, bool fail_flush, bool interrupted, uint64_t *sequence);
#endif
