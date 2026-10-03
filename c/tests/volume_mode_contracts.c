#include "ghostos/volume_mode.h"
#include <assert.h>
#include <string.h>
static void mode_changes_the_selected_version(void) {
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t exact[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g', ';', '1'};
    const uint8_t bad[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g', ';', 'w', 'a', 't'};
    const uint8_t alias[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'a', 'l', 'i', 'a', 's'};
    const uint8_t slash[] = {'/'};
    ghostos_volume_mode_record records[3];
    uint64_t generation = 4;
    memset(records, 0, sizeof records);
    records[0].name = log;
    records[0].name_length = sizeof log;
    records[0].file_type = 1;
    records[0].mode = 0644;
    records[0].version = 1;
    records[0].occupied = true;
    records[1].name = log;
    records[1].name_length = sizeof log;
    records[1].file_type = 1;
    records[1].mode = 0644;
    records[1].version = 2;
    records[1].occupied = true;
    records[2].name = alias;
    records[2].name_length = sizeof alias;
    records[2].file_type = 3;
    records[2].mode = 0777;
    records[2].version = 1;
    records[2].occupied = true;
    assert(ghostos_volume_set_mode(records, 3, slash, sizeof slash, 0755, &generation) == 4);
    assert(ghostos_volume_set_mode(records, 3, bad, sizeof bad, 0755, &generation) == 3);
    assert(!ghostos_volume_set_mode(records, 3, log, sizeof log, 0755, &generation));
    assert(records[1].mode == 0755 && records[0].mode == 0644 && generation == 5);
    assert(!ghostos_volume_set_mode(records, 3, exact, sizeof exact, 0700, &generation));
    assert(records[0].mode == 0700 && records[1].mode == 0755 && generation == 6);
    assert(!ghostos_volume_set_mode(records, 3, alias, sizeof alias, 0xffff, &generation));
    assert(records[2].mode == 07777 && records[1].mode == 0755);
    records[1].deleted = true;
    assert(!ghostos_volume_set_mode(records, 3, log, sizeof log, 0600, &generation));
    assert(records[0].mode == 0600 && records[1].mode == 0755);
    records[0].deleted = true;
    assert(ghostos_volume_set_mode(records, 3, log, sizeof log, 0600, &generation) == 1);
}
int main(void) {
    mode_changes_the_selected_version();
    return 0;
}
