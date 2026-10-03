#include "ghostos/volume_link.h"
#include <assert.h>
#include <string.h>
static void alias_shares_the_log_object(void) {
    const uint8_t directory[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e'};
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t alias[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'a', 'l', 'i', 'a', 's'};
    const uint8_t versioned[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g', ';', '1'};
    ghostos_volume_link_record records[4];
    uint64_t generation = 2;
    uint32_t links = 0;
    memset(records, 0, sizeof records);
    records[0].name = directory;
    records[0].name_length = sizeof directory;
    records[0].file_type = 2;
    records[0].version = 1;
    records[1].name = log;
    records[1].name_length = sizeof log;
    records[1].file_type = 1;
    records[1].version = 1;
    records[1].object_id = 7;
    records[2].name = log;
    records[2].name_length = sizeof log;
    records[2].file_type = 1;
    records[2].version = 2;
    records[2].object_id = 7;
    assert(ghostos_volume_link(records, 4, log, sizeof log, versioned, sizeof versioned, &generation) == 3);
    assert(!ghostos_volume_link(records, 4, log, sizeof log, alias, sizeof alias, &generation));
    assert(generation == 3 && records[3].object_id == 7 && records[3].version == 1);
    assert(!ghostos_volume_link_count(records, 4, alias, sizeof alias, &links));
    assert(links == 2);
    assert(!ghostos_volume_link_count(records, 4, log, sizeof log, &links));
    assert(links == 2);
    assert(ghostos_volume_link(records, 4, log, sizeof log, alias, sizeof alias, &generation) == 4);
    assert(ghostos_volume_link(records, 4, directory, sizeof directory, alias, sizeof alias, &generation) == 2);
}
int main(void) {
    alias_shares_the_log_object();
    return 0;
}
