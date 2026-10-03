#ifndef GHOSTOS_FSD_GC_H
#define GHOSTOS_FSD_GC_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 process not registered, 2 invalid capability,
   3 access denied. Collection requires the admin right, bit 3.
   The reported freed count is capped by the work budget after the sweep. */
typedef struct {
    bool occupied;
    uint64_t process;
    uint32_t generation;
    uint16_t rights;
} ghostos_fsd_gc_process;
typedef struct {
    bool occupied;
    uint16_t child;
} ghostos_fsd_block;
int ghostos_fsd_collect(const ghostos_fsd_gc_process *processes, size_t process_count, uint64_t process, uint64_t authority,
    ghostos_fsd_block *blocks, size_t block_count, uint16_t root, uint16_t checkpoint, size_t work_budget, size_t *live_blocks,
    size_t *freed_blocks);
#endif
