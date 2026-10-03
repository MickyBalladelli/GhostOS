#include "ghostos/volume_sync.h"
int ghostos_volume_sync_generation(uint64_t *generation, uint64_t cluster_counter) {
    if (!cluster_counter) return 1;
    if (cluster_counter == UINT64_MAX) return 2;
    if (cluster_counter > *generation) *generation = cluster_counter;
    return 0;
}
