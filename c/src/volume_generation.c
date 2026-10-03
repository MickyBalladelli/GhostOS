#include "ghostos/volume_generation.h"
void ghostos_volume_format(ghostos_volume *volume) {
    volume->active = 0;
    volume->sequence = 1;
    volume->banks[0].valid = true;
    volume->banks[0].sequence = 1;
    volume->banks[0].generation = 0;
    volume->banks[0].version = 0;
    volume->banks[1].valid = false;
    volume->banks[1].sequence = 0;
    volume->banks[1].generation = 0;
    volume->banks[1].version = 0;
}
int ghostos_volume_flush(ghostos_volume *volume, uint64_t generation, uint32_t version, uint64_t *sequence) {
    uint8_t bank;
    if (volume->sequence == UINT64_MAX) return 2;
    bank = (uint8_t)(1u - volume->active);
    volume->banks[bank].valid = true;
    volume->banks[bank].sequence = volume->sequence + 1;
    volume->banks[bank].generation = generation;
    volume->banks[bank].version = version;
    volume->sequence += 1;
    volume->active = bank;
    *sequence = volume->sequence;
    return 0;
}
int ghostos_volume_select(const ghostos_volume *volume, const bool readable[2], uint64_t *sequence, uint64_t *generation, uint32_t *version) {
    int best = -1, other, attempt, i;
    for (i = 0; i < 2; ++i) {
        if (!volume->banks[i].valid || !volume->banks[i].sequence) continue;
        if (best < 0 || volume->banks[i].sequence > volume->banks[best].sequence) best = i;
        else if (volume->banks[i].sequence == volume->banks[best].sequence) return 1;
    }
    if (best < 0) return 1;
    other = 1 - best;
    for (i = 0; i < 2; ++i) {
        attempt = i == 0 ? best : other;
        if (!volume->banks[attempt].valid || !readable[attempt]) continue;
        *sequence = volume->banks[attempt].sequence;
        *generation = volume->banks[attempt].generation;
        *version = volume->banks[attempt].version;
        return 0;
    }
    return 1;
}
