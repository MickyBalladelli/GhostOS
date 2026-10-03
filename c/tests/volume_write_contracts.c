#include "ghostos/volume_write.h"
#include <assert.h>
#include <string.h>
static uint64_t fnv(const uint8_t *bytes, size_t length) {
    uint64_t hash = 0xcbf29ce484222325ull;
    size_t i;
    for (i = 0; i < length; ++i) {
        hash ^= bytes[i];
        hash *= 0x100000001b3ull;
    }
    return hash;
}
static void second_log_write_keeps_the_object(void) {
    const uint8_t archive[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e'};
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t versioned[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g', ';', '1'};
    const uint8_t missing[] = {'/', 'd', 'a', 't', 'a', '/', 'm', 'i', 's', 's', 'i', 'n', 'g', '/', 'l', 'o', 'g'};
    const uint8_t dot[] = {'/', '.'};
    const uint8_t old_bytes[] = {'o', 'l', 'd'};
    const uint8_t new_bytes[] = {'n', 'e', 'w'};
    ghostos_volume_write_record records[4];
    uint64_t generation = 1, next_object = 1;
    memset(records, 0, sizeof records);
    records[0].name = archive;
    records[0].name_length = 5;
    records[0].file_type = 2;
    records[0].version = 1;
    records[0].occupied = true;
    records[1].name = archive;
    records[1].name_length = sizeof archive;
    records[1].file_type = 2;
    records[1].version = 1;
    records[1].occupied = true;
    assert(ghostos_volume_write(records, 4, versioned, sizeof versioned, old_bytes, 3, &generation, &next_object) == 3);
    assert(ghostos_volume_write(records, 4, dot, sizeof dot, old_bytes, 3, &generation, &next_object) == 4);
    assert(ghostos_volume_write(records, 4, missing, sizeof missing, old_bytes, 3, &generation, &next_object) == 1);
    assert(ghostos_volume_write(records, 4, archive, sizeof archive, old_bytes, 3, &generation, &next_object) == 2);
    assert(!ghostos_volume_write(records, 4, log, sizeof log, old_bytes, 3, &generation, &next_object));
    assert(records[2].version == 1 && records[2].object_id == 1 && records[2].size == 3);
    assert(records[2].checksum == fnv(old_bytes, 3) && next_object == 2 && generation == 2);
    assert(!ghostos_volume_write(records, 4, log, sizeof log, new_bytes, 3, &generation, &next_object));
    assert(records[3].version == 2 && records[3].object_id == 1 && records[3].data == new_bytes);
    assert(records[3].checksum == fnv(new_bytes, 3) && next_object == 2 && generation == 3);
    records[3].version = UINT32_MAX;
    assert(ghostos_volume_write(records, 4, log, sizeof log, old_bytes, 3, &generation, &next_object) == 6);
    assert(generation == 3);
}
static void empty_version_one_is_replaced(void) {
    const uint8_t name[] = {'/', 'n', 'o', 't', 'e'};
    const uint8_t bytes[] = {'o', 'l', 'd'};
    ghostos_volume_write_record records[1];
    uint64_t generation = 4, next_object = 8;
    memset(records, 0, sizeof records);
    records[0].name = name;
    records[0].name_length = sizeof name;
    records[0].file_type = 1;
    records[0].version = 1;
    records[0].object_id = 9;
    records[0].occupied = true;
    assert(!ghostos_volume_write(records, 1, name, sizeof name, bytes, 3, &generation, &next_object));
    assert(records[0].version == 1 && records[0].object_id == 9 && records[0].size == 3);
    assert(next_object == 8 && generation == 5);
    assert(ghostos_volume_write(records, 1, name, sizeof name, bytes, 3, &generation, &next_object) == 5);
    assert(generation == 5 && next_object == 8);
}
int main(void) {
    second_log_write_keeps_the_object();
    empty_version_one_is_replaced();
    return 0;
}
