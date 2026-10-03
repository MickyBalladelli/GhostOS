#include "ghostos/fsd_links.h"
#include <assert.h>
#include <string.h>
static ghostos_fsd_record named(const uint8_t *name, uint8_t length, uint32_t version, uint32_t cursor, uint64_t object) {
    ghostos_fsd_record item;
    item.name = name;
    item.name_length = length;
    item.version = version;
    item.cursor = cursor;
    item.object_id = object;
    item.deleted = false;
    return item;
}
static void selected_version_lists_alias_before_source(void) {
    const uint8_t alias[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'l', 'i', 'a', 's'};
    const uint8_t source[] = {'/', 'd', 'a', 't', 'a', '/', 's', 'o', 'u', 'r', 'c', 'e'};
    const uint8_t pattern[] = {'/', 'd', 'a', 't', 'a', '/', 's', 'o', 'u', 'r', 'c', 'e', '*', ';', '1'};
    const uint8_t bad[] = {'/', 'd', 'a', 't', 'a', '/', 's', 'o', 'u', 'r', 'c', 'e', '*', ';', 'w', 'a', 't'};
    ghostos_fsd_record records[3];
    size_t matches[4], written = 0;
    uint8_t output[32];
    records[0] = named(alias, sizeof alias, 1, 0, 7);
    records[1] = named(source, sizeof source, 1, 1, 7);
    records[2] = named(source, sizeof source, 2, 2, 7);
    assert(ghostos_fsd_list_links(2, pattern, sizeof pattern, records, 3, matches, 4, output, sizeof output, &written) == 1);
    assert(ghostos_fsd_list_links(1, bad, sizeof bad, records, 3, matches, 4, output, sizeof output, &written) == 2);
    assert(ghostos_fsd_list_links(1, pattern, sizeof pattern, records, 3, matches, 4, output, 1, &written) == 5);
    assert(!ghostos_fsd_list_links(1, pattern, sizeof pattern, records, 3, matches, 4, output, sizeof output, &written));
    assert(written == sizeof alias + sizeof source + 2);
    assert(!memcmp(output, "/data/alias\n/data/source\n", written));
    records[1].deleted = true;
    assert(ghostos_fsd_list_links(1, pattern, sizeof pattern, records, 3, matches, 4, output, sizeof output, &written) == 4);
}
int main(void) {
    selected_version_lists_alias_before_source();
    return 0;
}
