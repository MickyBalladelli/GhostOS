#include "ghostos/volume_directory.h"
#include <assert.h>
#include <string.h>
static void recursive_archive_creates_data(void) {
    const uint8_t path[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e'};
    const uint8_t slash[] = {'/'};
    const uint8_t dot[] = {'/', '.'};
    const uint8_t child[] = {'/', 'd', 'a', 't', 'a', '/', 'c', 'h', 'i', 'l', 'd'};
    ghostos_volume_directory_record records[3];
    uint64_t generation = 1;
    size_t created = 9;
    memset(records, 0, sizeof records);
    assert(ghostos_volume_create_directory(records, 3, slash, sizeof slash, true, &generation, &created) == 3);
    assert(ghostos_volume_create_directory(records, 3, dot, sizeof dot, true, &generation, &created) == 3);
    assert(!ghostos_volume_create_directory(records, 3, path, sizeof path, true, &generation, &created));
    assert(created == 2 && generation == 3);
    assert(records[0].name_length == 5 && records[0].file_type == 2);
    assert(records[1].name_length == sizeof path && records[1].file_type == 2);
    assert(ghostos_volume_create_directory(records, 3, path, sizeof path, true, &generation, &created) == 4);
    assert(!created && generation == 3);
    records[0].file_type = 1;
    assert(ghostos_volume_create_directory(records, 3, child, sizeof child, false, &generation, &created) == 2);
    memset(records, 0, sizeof records);
    generation = 1;
    assert(ghostos_volume_create_directory(records, 1, path, sizeof path, true, &generation, &created) == 5);
    assert(created == 1 && records[0].name_length == 5 && generation == 2);
}
int main(void) {
    recursive_archive_creates_data();
    return 0;
}
