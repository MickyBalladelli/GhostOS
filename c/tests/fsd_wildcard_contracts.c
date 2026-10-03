#include "ghostos/fsd_wildcard.h"
#include <assert.h>
#include <string.h>
static ghostos_fsd_record record(const uint8_t *name, uint8_t length, uint32_t version, uint32_t cursor) {
    ghostos_fsd_record item;
    item.name = name;
    item.name_length = length;
    item.version = version;
    item.cursor = cursor;
    item.deleted = false;
    return item;
}
static void exact_version_selects_one_live_record(void) {
    const uint8_t first[] = {'/', 'd', 'a', 't', 'a', '/', 'f', 'i', 'r', 's', 't'};
    const uint8_t second[] = {'/', 'd', 'a', 't', 'a', '/', 's', 'e', 'c', 'o', 'n', 'd'};
    const uint8_t source[] = {'/', 'd', 'a', 't', 'a', '/', 's', 'o', 'u', 'r', 'c', 'e'};
    const uint8_t selected[] = {'/', 'd', 'a', 't', 'a', '/', 's', 'o', 'u', 'r', 'c', 'e', '*', ';', '1'};
    const uint8_t bad[] = {'/', 'd', 'a', 't', 'a', '/', 's', 'o', 'u', 'r', 'c', 'e', '*', ';', 'w', 'a', 't'};
    const uint8_t star[] = {'/', 'd', 'a', 't', 'a', '/', '*'};
    ghostos_fsd_record records[4];
    size_t matches[4], count = 0, next = 0;
    bool has_next = true;
    records[0] = record(first, 11, 1, 0);
    records[1] = record(second, 12, 1, 1);
    records[2] = record(source, 12, 1, 2);
    records[3] = record(source, 12, 2, 3);
    assert(ghostos_fsd_expand_delete(1, star, sizeof star, records, 4, 0, matches, 4, &count, &next, &has_next) == 5);
    assert(ghostos_fsd_expand_delete(14, bad, sizeof bad, records, 4, 0, matches, 4, &count, &next, &has_next) == 1);
    assert(!ghostos_fsd_expand(selected, sizeof selected, records, 4, 0, matches, 4, &count, &next, &has_next));
    assert(count == 1 && !has_next && records[matches[0]].version == 1 && records[matches[0]].name_length == 12);
    records[2].deleted = true;
    assert(ghostos_fsd_expand(selected, sizeof selected, records, 4, 0, matches, 4, &count, &next, &has_next) == 4);
    assert(!ghostos_fsd_expand(star, sizeof star, records, 4, 0, matches, 1, &count, &next, &has_next));
    assert(count == 1 && has_next && next == 1 && !memcmp(records[matches[0]].name, first, 11));
    assert(!ghostos_fsd_expand(star, sizeof star, records, 4, 1, matches, 4, &count, &next, &has_next));
    assert(count == 2 && !has_next && !memcmp(records[matches[0]].name, second, 12));
    assert(records[matches[1]].version == 2);
}
int main(void) {
    exact_version_selects_one_live_record();
    return 0;
}
