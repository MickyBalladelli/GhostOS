#include "ghostos/volume_current_link.h"
#include <assert.h>
#include <string.h>
static void tombstone_hides_the_older_log_name(void) {
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t exact[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g', ';', '2'};
    const uint8_t bad[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g', ';', 'w', 'a', 't'};
    const uint8_t alias[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'a', 'l', 'i', 'a', 's'};
    const uint8_t slash[] = {'/'};
    ghostos_volume_current_record records[3];
    uint32_t links = 9;
    memset(records, 0, sizeof records);
    records[0].name = log;
    records[0].name_length = sizeof log;
    records[0].version = 1;
    records[0].object_id = 7;
    records[0].occupied = true;
    records[1].name = log;
    records[1].name_length = sizeof log;
    records[1].version = 2;
    records[1].object_id = 7;
    records[1].occupied = true;
    records[1].deleted = true;
    records[2].name = alias;
    records[2].name_length = sizeof alias;
    records[2].version = 1;
    records[2].object_id = 7;
    records[2].occupied = true;
    assert(ghostos_volume_current_link_count(records, 3, slash, sizeof slash, &links) == 4);
    assert(ghostos_volume_current_link_count(records, 3, bad, sizeof bad, &links) == 3);
    assert(ghostos_volume_current_link_count(records, 3, exact, sizeof exact, &links) == 1);
    assert(links == 9);
    assert(!ghostos_volume_current_link_count(records, 3, alias, sizeof alias, &links));
    assert(links == 1);
    assert(!ghostos_volume_current_link_count(records, 3, log, sizeof log, &links));
    assert(links == 1);
    records[1].deleted = false;
    assert(!ghostos_volume_current_link_count(records, 3, log, sizeof log, &links));
    assert(links == 2);
}
int main(void) {
    tombstone_hides_the_older_log_name();
    return 0;
}
