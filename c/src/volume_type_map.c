#include "ghostos/volume_type_map.h"
static uint64_t checksum(const uint8_t *bytes, size_t length) {
    uint64_t hash = 0xcbf29ce484222325ull;
    size_t i;
    for (i = 0; i < length; ++i) {
        hash ^= bytes[i];
        hash *= 0x100000001b3ull;
    }
    return hash;
}
static int pack_limit(size_t block_count, size_t *used) {
    if (block_count > (GHOSTOS_VOLUME_TYPE_MAP - 8u) * 4u) return 1;
    *used = 8u + (block_count + 3u) / 4u;
    return 0;
}
int ghostos_volume_type_map_encode(const uint8_t *kinds, size_t block_count, uint8_t *map, size_t capacity, uint64_t *checksum_out) {
    size_t i, used = 0;
    static const uint8_t magic[] = {'S', 'Y', 'N', 'F', 'S', 'M', 'A', 'P'};
    if (capacity < GHOSTOS_VOLUME_TYPE_MAP) return 2;
    if (pack_limit(block_count, &used)) return 1;
    (void)used;
    for (i = 0; i < block_count; ++i) if (kinds[i] > 3) return 1;
    for (i = 0; i < GHOSTOS_VOLUME_TYPE_MAP; ++i) map[i] = 0;
    for (i = 0; i < 8; ++i) map[i] = magic[i];
    for (i = 0; i < block_count; ++i) map[8 + i / 4] |= (uint8_t)(kinds[i] << ((i % 4) * 2));
    *checksum_out = checksum(map, GHOSTOS_VOLUME_TYPE_MAP);
    return 0;
}
int ghostos_volume_type_map_decode(const uint8_t *map, size_t capacity, size_t block_count, uint64_t expected, uint8_t *kinds, size_t kinds_capacity) {
    size_t i, used = 0;
    static const uint8_t magic[] = {'S', 'Y', 'N', 'F', 'S', 'M', 'A', 'P'};
    if (capacity < GHOSTOS_VOLUME_TYPE_MAP) return 2;
    if (kinds_capacity < block_count) return 2;
    if (pack_limit(block_count, &used)) return 1;
    for (i = 0; i < 8; ++i) if (map[i] != magic[i]) return 1;
    if (checksum(map, GHOSTOS_VOLUME_TYPE_MAP) != expected) return 1;
    for (i = used; i < GHOSTOS_VOLUME_TYPE_MAP; ++i) if (map[i]) return 1;
    for (i = 0; i < block_count; ++i) kinds[i] = (uint8_t)((map[8 + i / 4] >> ((i % 4) * 2)) & 0x03);
    return 0;
}
