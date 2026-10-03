#include "ghostos/volume_symlink.h"
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
static void alias_follows_the_archive_log(void) {
    const uint8_t archive[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e'};
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t alias[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'a', 'l', 'i', 'a', 's'};
    const uint8_t relative[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'r', 'e', 'l'};
    const uint8_t loop[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'o', 'p'};
    const uint8_t slash[] = {'/'};
    const uint8_t log_name[] = {'l', 'o', 'g'};
    const uint8_t loop_name[] = {'l', 'o', 'o', 'p'};
    const uint8_t empty[] = {0};
    ghostos_volume_symlink_record records[6];
    uint8_t output[32], resolved[32];
    uint64_t generation = 1, next_object = 1;
    size_t read = 0, resolved_length = 0;
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
    records[2].name = log;
    records[2].name_length = sizeof log;
    records[2].file_type = 1;
    records[2].version = 1;
    records[2].occupied = true;
    assert(ghostos_volume_symlink(records, 6, empty, 0, alias, sizeof alias, &generation, &next_object) == 4);
    assert(ghostos_volume_symlink(records, 6, log, sizeof log, slash, sizeof slash, &generation, &next_object) == 4);
    assert(ghostos_volume_symlink(records, 3, log, sizeof log, alias, sizeof alias, &generation, &next_object) == 6);
    assert(generation == 1 && next_object == 1);
    assert(!ghostos_volume_symlink(records, 6, log, sizeof log, alias, sizeof alias, &generation, &next_object));
    assert(records[3].file_type == 3 && records[3].version == 1 && records[3].object_id == 1);
    assert(records[3].size == sizeof log && records[3].checksum == fnv(log, sizeof log));
    assert(generation == 2 && next_object == 2);
    assert(!ghostos_volume_read_link(records, 6, alias, sizeof alias, output, sizeof output, &read));
    assert(read == sizeof log && !memcmp(output, log, sizeof log));
    assert(ghostos_volume_read_link(records, 6, alias, sizeof alias, output, 4, &read) == 8);
    assert(ghostos_volume_read_link(records, 6, log, sizeof log, output, sizeof output, &read) == 3);
    assert(!ghostos_volume_follow(records, 6, alias, sizeof alias, resolved, sizeof resolved, &resolved_length));
    assert(resolved_length == sizeof log && !memcmp(resolved, log, sizeof log));
    assert(ghostos_volume_symlink(records, 6, log, sizeof log, alias, sizeof alias, &generation, &next_object) == 5);
    assert(!ghostos_volume_symlink(records, 6, log_name, sizeof log_name, relative, sizeof relative, &generation, &next_object));
    assert(!ghostos_volume_follow(records, 6, relative, sizeof relative, resolved, sizeof resolved, &resolved_length));
    assert(resolved_length == sizeof log && !memcmp(resolved, log, sizeof log));
    assert(!ghostos_volume_symlink(records, 6, loop_name, sizeof loop_name, loop, sizeof loop, &generation, &next_object));
    assert(ghostos_volume_follow(records, 6, loop, sizeof loop, resolved, sizeof resolved, &resolved_length) == 9);
}
int main(void) {
    alias_follows_the_archive_log();
    return 0;
}
