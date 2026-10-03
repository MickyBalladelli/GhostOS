#include "ghostos/volume_sync.h"
#include <assert.h>
static void peer_counter_advances_and_does_not_move_backward(void) {
    uint64_t generation = 4;
    assert(ghostos_volume_sync_generation(&generation, 0) == 1);
    assert(generation == 4);
    assert(ghostos_volume_sync_generation(&generation, UINT64_MAX) == 2);
    assert(generation == 4);
    assert(!ghostos_volume_sync_generation(&generation, 3));
    assert(generation == 4);
    assert(!ghostos_volume_sync_generation(&generation, 9));
    assert(generation == 9);
    assert(!ghostos_volume_sync_generation(&generation, 9));
    assert(generation == 9);
}
int main(void) {
    peer_counter_advances_and_does_not_move_backward();
    return 0;
}
