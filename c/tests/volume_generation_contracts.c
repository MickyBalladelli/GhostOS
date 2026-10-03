#include "ghostos/volume_generation.h"
#include <assert.h>
static void newest_valid_generation_is_selected(void) {
    ghostos_volume volume;
    bool readable[2] = {true, true};
    uint64_t first = 0, second = 0, sequence = 0, generation = 0;
    uint32_t version = 0;
    ghostos_volume_format(&volume);
    assert(!ghostos_volume_flush(&volume, 1, 1, &first));
    assert(!ghostos_volume_flush(&volume, 2, 2, &second));
    assert(second > first);
    assert(!ghostos_volume_select(&volume, readable, &sequence, &generation, &version));
    assert(sequence == second && generation == 2 && version == 2);
    readable[volume.active] = false;
    assert(!ghostos_volume_select(&volume, readable, &sequence, &generation, &version));
    assert(sequence == first && version == 1);
    volume.banks[0].valid = false;
    volume.banks[1].valid = false;
    assert(ghostos_volume_select(&volume, readable, &sequence, &generation, &version) == 1);
    volume.banks[0].valid = true;
    volume.banks[1].valid = true;
    volume.banks[0].sequence = 5;
    volume.banks[1].sequence = 5;
    readable[0] = true;
    readable[1] = true;
    assert(ghostos_volume_select(&volume, readable, &sequence, &generation, &version) == 1);
}
int main(void) {
    newest_valid_generation_is_selected();
    return 0;
}
