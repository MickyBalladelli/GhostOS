#include "ghostos/volume_link_list.h"
#include <assert.h>
#include <string.h>
static void tombstoned_log_still_lists_its_live_version(void) {
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t exact[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g', ';', '2'};
    const uint8_t bad[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g', ';', 'w', 'a', 't'};
    const uint8_t alias[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'a', 'l', 'i', 'a', 's'};
    const uint8_t slash[] = {'/'};
    const uint8_t missing[] = {'/', 'd', 'a', 't', 'a', '/', 'm', 'i', 's', 's', 'i', 'n', 'g'};
    ghostos_volume_link_list_record records[3];
    ghostos_volume_link_list_entry entries[2];
    size_t written = 9;
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
    assert(ghostos_volume_list_links(records, 3, slash, sizeof slash, entries, 2, &written) == 4);
    assert(ghostos_volume_list_links(records, 3, bad, sizeof bad, entries, 2, &written) == 3);
    assert(ghostos_volume_list_links(records, 3, exact, sizeof exact, entries, 2, &written) == 1);
    assert(ghostos_volume_list_links(records, 3, missing, sizeof missing, entries, 2, &written) == 1);
    assert(written == 9);
    assert(ghostos_volume_list_links(records, 3, alias, sizeof alias, entries, 1, &written) == 5);
    assert(written == 1 && !entries[0].version && entries[0].name_length == sizeof alias);
    assert(!memcmp(entries[0].name, alias, sizeof alias));
    assert(!ghostos_volume_list_links(records, 3, log, sizeof log, entries, 2, &written));
    assert(written == 2 && !entries[0].version && !entries[1].version);
    assert(!memcmp(entries[0].name, alias, sizeof alias));
    assert(entries[1].name_length == sizeof log && !memcmp(entries[1].name, log, sizeof log));
}
int main(void) {
    tombstoned_log_still_lists_its_live_version();
    return 0;
}
