#include "ghostos/volume_space.h"
#include <assert.h>
static void free_space_uses_the_tighter_block_limit(void) {
    uint64_t blocks = 9, bytes = 9;
    ghostos_volume_free_space(4, 8, 1, &blocks, &bytes);
    assert(blocks == 3 && bytes == 3 * 4096);
    ghostos_volume_free_space(100, 8, 1, &blocks, &bytes);
    assert(blocks == 7 && bytes == 7 * 4096);
    ghostos_volume_free_space(4, 8, 5, &blocks, &bytes);
    assert(!blocks && !bytes);
    ghostos_volume_free_space(UINT64_MAX, 8, 2, &blocks, &bytes);
    assert(blocks == 6 && bytes == 6 * 4096);
    ghostos_volume_free_space(UINT64_MAX, UINT64_MAX, 0, &blocks, &bytes);
    assert(blocks == UINT64_MAX && bytes == UINT64_MAX);
}
int main(void) {
    free_space_uses_the_tighter_block_limit();
    return 0;
}
