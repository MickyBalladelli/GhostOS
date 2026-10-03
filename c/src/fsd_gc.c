#include "ghostos/fsd_gc.h"
static void mark(ghostos_fsd_block *blocks, size_t count, uint16_t id, bool *marked) {
    while (id && id <= count) {
        size_t index = (size_t)id - 1;
        if (marked[index] || !blocks[index].occupied) return;
        marked[index] = true;
        id = blocks[index].child;
    }
}
static void sweep(ghostos_fsd_block *blocks, size_t count, uint16_t root, uint16_t checkpoint, size_t *live_blocks, size_t *freed_blocks) {
    bool marked[64];
    size_t i;
    if (count > 64) count = 64;
    for (i = 0; i < count; ++i) marked[i] = false;
    mark(blocks, count, root, marked);
    mark(blocks, count, checkpoint, marked);
    *live_blocks = 0;
    *freed_blocks = 0;
    for (i = 0; i < count; ++i) {
        if (!blocks[i].occupied) continue;
        if (marked[i]) *live_blocks += 1;
        else {
            blocks[i].occupied = false;
            blocks[i].child = 0;
            *freed_blocks += 1;
        }
    }
}
int ghostos_fsd_collect(const ghostos_fsd_gc_process *processes, size_t process_count, uint64_t process, uint64_t authority,
    ghostos_fsd_block *blocks, size_t block_count, uint16_t root, uint16_t checkpoint, size_t work_budget, size_t *live_blocks,
    size_t *freed_blocks) {
    size_t index = 0, i;
    uint32_t generation, raw;
    bool found = false;
    size_t live = 0, freed = 0;
    for (i = 0; i < process_count; ++i) if (processes[i].occupied && processes[i].process == process) { index = i; found = true; break; }
    if (!found) return 1;
    generation = (uint32_t)(authority >> 32);
    raw = (uint32_t)authority;
    if (!raw || (size_t)raw - 1 != index || generation != processes[index].generation) return 2;
    if ((processes[index].rights & 8u) == 0) return 3;
    sweep(blocks, block_count, root, checkpoint, &live, &freed);
    *live_blocks = live;
    *freed_blocks = freed < work_budget ? freed : work_budget;
    return 0;
}
