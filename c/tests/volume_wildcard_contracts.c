#include "ghostos/volume_wildcard.h"
#include <assert.h>
#include <string.h>
static void archive_star_pages_alias_then_the_latest_log(void) {
    const uint8_t alias[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'a', 'l', 'i', 'a', 's'};
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t other[] = {'/', 'd', 'a', 't', 'a', '/', 'o', 't', 'h', 'e', 'r'};
    const uint8_t pattern[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', '*'};
    const uint8_t exact[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', '*', ';', '1'};
    const uint8_t bad[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', '*', ';', 'w', 'a', 't'};
    const uint8_t broken[] = {'/', 'd', 'a', 't', 'a', '/', '['};
    ghostos_volume_wildcard_record records[4];
    ghostos_volume_wildcard_entry entries[2];
    size_t written = 0, next = 0;
    bool has_next = false;
    memset(records, 0, sizeof records);
    records[0].name = log;
    records[0].name_length = sizeof log;
    records[0].version = 1;
    records[0].occupied = true;
    records[1].name = log;
    records[1].name_length = sizeof log;
    records[1].version = 2;
    records[1].occupied = true;
    records[2].name = alias;
    records[2].name_length = sizeof alias;
    records[2].version = 1;
    records[2].occupied = true;
    records[3].name = other;
    records[3].name_length = sizeof other;
    records[3].version = 1;
    records[3].occupied = true;
    assert(ghostos_volume_expand(records, 4, bad, sizeof bad, entries, 2, &written) == 3);
    assert(ghostos_volume_expand(records, 4, broken, sizeof broken, entries, 2, &written) == 6);
    assert(ghostos_volume_expand(records, 4, pattern, sizeof pattern, entries, 0, &written) == 5);
    assert(ghostos_volume_expand(records, 4, pattern, sizeof pattern, entries, 1, &written) == 5);
    assert(written == 1 && entries[0].version == 1 && entries[0].cursor == 0);
    assert(entries[0].name_length == sizeof alias && !memcmp(entries[0].name, alias, sizeof alias));
    assert(!ghostos_volume_expand_page(records, 4, pattern, sizeof pattern, 0, entries, 1, &written, &next, &has_next));
    assert(has_next && next == 2 && entries[0].cursor == 0);
    assert(!ghostos_volume_expand_page(records, 4, pattern, sizeof pattern, next, entries, 1, &written, &next, &has_next));
    assert(!has_next && written == 1 && entries[0].version == 2 && entries[0].cursor == 2);
    assert(!memcmp(entries[0].name, log, sizeof log));
    assert(!ghostos_volume_expand_page(records, 4, exact, sizeof exact, 0, entries, 1, &written, &next, &has_next));
    assert(has_next && next == 1 && written == 1 && entries[0].version == 1 && !memcmp(entries[0].name, alias, sizeof alias));
}
int main(void) {
    archive_star_pages_alias_then_the_latest_log();
    return 0;
}
