#include "ghostos/volume_device.h"
#include <assert.h>
static void superblock_is_written_after_data_and_before_activation(void) {
    ghostos_volume volume;
    uint64_t log[8], sequence = 0;
    size_t count = 0;
    ghostos_volume_format(&volume);
    assert(ghostos_volume_device_flush(&volume, 0, 0, 1, 1, 1, 1, log, 8, &count, -1, false, false, &sequence) == 1);
    assert(!count && volume.sequence == 1);
    assert(!ghostos_volume_device_flush(&volume, 1, 1, 1, 1, 1, 1, log, 8, &count, -1, false, false, &sequence));
    assert(sequence == 2 && volume.active == 1);
    assert(count == 4 && log[0] == 4 && log[1] == 5 && log[2] == 3 && log[3] == UINT64_MAX);
    assert(ghostos_volume_device_flush(&volume, 1, 1, 1, 1, 1, 1, log, 8, &count, 0, false, false, &sequence) == 3);
    assert(!count && volume.sequence == 2);
    assert(ghostos_volume_device_flush(&volume, 1, 1, 1, 1, 1, 1, log, 8, &count, -1, true, false, &sequence) == 3);
    assert(count == 3 && log[2] == 0 && volume.sequence == 2 && volume.active == 1);
    assert(ghostos_volume_device_flush(&volume, 2, 2, 1, 1, 1, 1, log, 8, &count, -1, false, true, &sequence) == 4);
    assert(sequence == 3 && volume.sequence == 3 && volume.active == 0);
    assert(!ghostos_volume_device_discard(1, log, 8, &count, -1, false, false, false));
    assert(count == 8 && log[0] == 0 && log[5] == 5 && log[6] == UINT64_MAX && log[7] == UINT64_MAX - 1u);
    assert(ghostos_volume_device_discard(1, log, 8, &count, 2, false, false, false) == 3);
    assert(count == 2 && log[1] == 1);
    assert(ghostos_volume_device_discard(1, log, 8, &count, -1, true, false, false) == 3);
    assert(count == 6 && log[5] == 5);
    assert(ghostos_volume_device_discard(1, log, 8, &count, -1, false, true, false) == 4);
    assert(count == 7 && log[6] == UINT64_MAX);
}
int main(void) {
    superblock_is_written_after_data_and_before_activation();
    return 0;
}
