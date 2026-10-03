#include "ghostos/volume_type_map.h"
#include <assert.h>
#include <string.h>
static void type_map_keeps_four_kinds_and_rejects_a_trailing_byte(void) {
    const uint8_t kinds[] = {0, 1, 2, 3};
    uint8_t map[GHOSTOS_VOLUME_TYPE_MAP];
    uint8_t decoded[4];
    uint64_t sum = 0;
    assert(!ghostos_volume_type_map_encode(kinds, 4, map, sizeof map, &sum));
    assert(!memcmp(map, "SYNFSMAP", 8));
    assert(map[8] == (uint8_t)((1u << 2) | (2u << 4) | (3u << 6)));
    assert(!ghostos_volume_type_map_decode(map, sizeof map, 4, sum, decoded, 4));
    assert(!memcmp(decoded, kinds, 4));
    map[GHOSTOS_VOLUME_TYPE_MAP - 1] ^= 1;
    assert(ghostos_volume_type_map_decode(map, sizeof map, 4, sum, decoded, 4) == 1);
}
int main(void) {
    type_map_keeps_four_kinds_and_rejects_a_trailing_byte();
    return 0;
}
