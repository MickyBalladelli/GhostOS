#ifndef GHOSTOS_VOLUME_TYPE_MAP_H
#define GHOSTOS_VOLUME_TYPE_MAP_H
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 corrupt, 2 buffer too small.
   Kind: empty=0, leaf=1, branch=2, data=3. The map is 4096 bytes and begins
   with SYNFSMAP. The checksum covers the whole map. */
#define GHOSTOS_VOLUME_TYPE_MAP 4096u
int ghostos_volume_type_map_encode(const uint8_t *kinds, size_t block_count, uint8_t *map, size_t capacity, uint64_t *checksum);
int ghostos_volume_type_map_decode(const uint8_t *map, size_t capacity, size_t block_count, uint64_t expected, uint8_t *kinds, size_t kinds_capacity);
#endif
