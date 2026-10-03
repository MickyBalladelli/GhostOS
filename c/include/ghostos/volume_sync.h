#ifndef GHOSTOS_VOLUME_SYNC_H
#define GHOSTOS_VOLUME_SYNC_H
#include <stdint.h>
/* Result: 0 success, 1 invalid version, 2 version overflow.
   A peer counter of zero is invalid. The maximum counter overflows.
   A smaller counter leaves the local generation in place. */
int ghostos_volume_sync_generation(uint64_t *generation, uint64_t cluster_counter);
#endif
