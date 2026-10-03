#include "ghostos/volume_list.h"
#include <assert.h>
#include <string.h>
static void archive_lists_alias_before_the_latest_log(void) {
    const uint8_t data[] = {'/', 'd', 'a', 't', 'a'};
    const uint8_t archive[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e'};
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t alias[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'a', 'l', 'i', 'a', 's'};
    const uint8_t link[] = {'/', 'd', 'a', 't', 'a', '/', 'l', 'i', 'n', 'k'};
    const uint8_t bad[] = {'/', 'd', 'a', 't', 'a', ';', 'w', 'a', 't'};
    const uint8_t slash[] = {'/'};
    ghostos_volume_list_record records[6];
    ghostos_volume_list_entry entries[2];
    size_t written = 0, next = 0;
    bool has_next = false;
    memset(records, 0, sizeof records);
    records[0].name = data;
    records[0].name_length = sizeof data;
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
    records[2].size = 3;
    records[2].occupied = true;
    records[3].name = log;
    records[3].name_length = sizeof log;
    records[3].file_type = 1;
    records[3].mode = 0644;
    records[3].version = 2;
    records[3].size = 3;
    records[3].occupied = true;
    records[4].name = alias;
    records[4].name_length = sizeof alias;
    records[4].file_type = 1;
    records[4].version = 1;
    records[4].link_count = 2;
    records[4].size = 3;
    records[4].occupied = true;
    records[5].name = link;
    records[5].name_length = sizeof link;
    records[5].data = archive;
    records[5].file_type = 3;
    records[5].version = 1;
    records[5].size = sizeof archive;
    records[5].occupied = true;
    assert(ghostos_volume_list_directory(records, 6, bad, sizeof bad, entries, 2, &written) == 3);
    assert(ghostos_volume_list_directory(records, 6, log, sizeof log, entries, 2, &written) == 2);
    assert(ghostos_volume_list_directory(records, 6, link, 4, entries, 2, &written) == 1);
    assert(ghostos_volume_list_directory(records, 6, archive, sizeof archive, entries, 1, &written) == 5);
    assert(written == 1 && entries[0].name_length == 5 && !memcmp(entries[0].name, "alias", 5));
    assert(!ghostos_volume_list_directory_page(records, 6, archive, sizeof archive, 1, entries, 2, &written, &next, &has_next));
    assert(!has_next && written == 1 && entries[0].name_length == 3 && entries[0].version == 2 && entries[0].link_count == 1);
    assert(!memcmp(entries[0].name, "log", 3) && entries[0].mode == 0644);
    assert(!ghostos_volume_list_directory(records, 6, link, sizeof link, entries, 2, &written));
    assert(written == 2 && !memcmp(entries[0].name, "alias", 5) && entries[1].version == 2);
    assert(!ghostos_volume_list_directory(records, 6, slash, sizeof slash, entries, 2, &written));
    assert(written == 1 && entries[0].name_length == 4 && !memcmp(entries[0].name, "data", 4));
}
int main(void) {
    archive_lists_alias_before_the_latest_log();
    return 0;
}
