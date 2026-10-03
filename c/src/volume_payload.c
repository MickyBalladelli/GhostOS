#include "ghostos/volume_payload.h"
enum { GHOSTOS_VOLUME_PAYLOAD_LIMIT = 4080 };
static uint64_t mix(uint64_t hash, const uint8_t *bytes, size_t length) {
    size_t i;
    for (i = 0; i < length; ++i) {
        hash ^= bytes[i];
        hash *= 0x100000001b3ull;
    }
    return hash;
}
int ghostos_volume_check_payload(const ghostos_volume_payload *blocks, size_t block_count, uint32_t first, uint64_t size,
    uint64_t expected_checksum, uint64_t object_id, uint64_t *owners, bool *owner_set) {
    bool seen[GHOSTOS_VOLUME_PAYLOAD_BLOCKS];
    uint64_t hash = 0xcbf29ce484222325ull, total = 0;
    uint32_t id = first;
    size_t i;
    if (block_count > GHOSTOS_VOLUME_PAYLOAD_BLOCKS) return 1;
    for (i = 0; i < block_count; ++i) seen[i] = false;
    while (id) {
        size_t index;
        uint64_t block_hash;
        if (id > block_count) return 1;
        index = (size_t)id - 1;
        if (seen[index]) return 1;
        seen[index] = true;
        if (owner_set[index] && owners[index] != object_id) return 1;
        if (!blocks[index].length || blocks[index].length > GHOSTOS_VOLUME_PAYLOAD_LIMIT || !blocks[index].bytes) return 1;
        block_hash = mix(0xcbf29ce484222325ull, blocks[index].bytes, blocks[index].length);
        if (block_hash != blocks[index].checksum) return 1;
        owner_set[index] = true;
        owners[index] = object_id;
        hash = mix(hash, blocks[index].bytes, blocks[index].length);
        if (total > UINT64_MAX - blocks[index].length) return 1;
        total += blocks[index].length;
        id = blocks[index].next;
    }
    if (total != size || hash != expected_checksum) return 1;
    return 0;
}
