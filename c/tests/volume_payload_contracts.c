#include "ghostos/volume_payload.h"
#include <assert.h>
static uint64_t hash_bytes(const uint8_t *bytes, size_t length) {
    uint64_t hash = 0xcbf29ce484222325ull;
    size_t i;
    for (i = 0; i < length; ++i) {
        hash ^= bytes[i];
        hash *= 0x100000001b3ull;
    }
    return hash;
}
static void payload_checksum_covers_the_chain_in_order(void) {
    const uint8_t first[] = {'a', 'b'};
    const uint8_t second[] = {'c'};
    uint8_t combined[] = {'a', 'b', 'c'};
    ghostos_volume_payload blocks[2];
    uint64_t owners[2] = {0, 0};
    bool owner_set[2] = {false, false};
    blocks[0].next = 2;
    blocks[0].length = 2;
    blocks[0].bytes = first;
    blocks[0].checksum = hash_bytes(first, 2);
    blocks[1].next = 0;
    blocks[1].length = 1;
    blocks[1].bytes = second;
    blocks[1].checksum = hash_bytes(second, 1);
    assert(!ghostos_volume_check_payload(blocks, 2, 1, 3, hash_bytes(combined, 3), 7, owners, owner_set));
    assert(owners[0] == 7 && owners[1] == 7);
    assert(ghostos_volume_check_payload(blocks, 2, 1, 2, hash_bytes(combined, 3), 7, owners, owner_set) == 1);
    blocks[0].checksum ^= 1;
    owner_set[0] = false;
    owner_set[1] = false;
    assert(ghostos_volume_check_payload(blocks, 2, 1, 3, hash_bytes(combined, 3), 7, owners, owner_set) == 1);
    blocks[0].checksum ^= 1;
    blocks[1].next = 2;
    owner_set[0] = false;
    owner_set[1] = false;
    assert(ghostos_volume_check_payload(blocks, 2, 1, 3, hash_bytes(combined, 3), 7, owners, owner_set) == 1);
}
int main(void) {
    payload_checksum_covers_the_chain_in_order();
    return 0;
}
