#include "ghostos/frame_allocator.h"

#include <assert.h>
#include <stdint.h>

int main(void) {
    const ghostos_memory_region regions[] = {
        {0x1000, 0x1000, GHOSTOS_MEMORY_RESERVED, 0},
        {16 * 1024 * 1024 + 1, 0x3000, GHOSTOS_MEMORY_USABLE, 0},
    };
    ghostos_early_frame_allocator allocator;
    uint64_t frame = 0;
    ghostos_frame_allocator_init(&allocator, regions, 2);
    assert(ghostos_frame_allocate(&allocator, &frame) == GHOSTOS_ALLOC_OK);
    assert(frame == 16 * 1024 * 1024 + 0x1000);
    assert(ghostos_frame_allocate(&allocator, &frame) == GHOSTOS_ALLOC_OK);
    assert(frame == 16 * 1024 * 1024 + 0x2000);
    assert(ghostos_frame_allocate(&allocator, &frame) == GHOSTOS_ALLOC_EXHAUSTED);
    return 0;
}
