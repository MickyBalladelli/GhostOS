#include "ghostos/volume_remove.h"
#include <assert.h>
#include <string.h>
static void occupied_archive_stays_until_the_log_is_gone(void) {
    const uint8_t data[] = {'/', 'd', 'a', 't', 'a'};
    const uint8_t archive[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e'};
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t nested[] = {'/', 'd', 'a', 't', 'a', '/', 'n', 'e', 's', 't', '/', 's', 'u', 'b', '/', 'f', 'i', 'l', 'e'};
    const uint8_t nest[] = {'/', 'd', 'a', 't', 'a', '/', 'n', 'e', 's', 't'};
    const uint8_t slash[] = {'/'};
    const uint8_t versioned[] = {'/', 'd', 'a', 't', 'a', ';', '1'};
    ghostos_volume_remove_record records[5];
    uint64_t generation = 3;
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
    records[3].name = nest;
    records[3].name_length = sizeof nest;
    records[3].file_type = 2;
    records[3].version = 1;
    records[3].occupied = true;
    records[4].name = nested;
    records[4].name_length = sizeof nested;
    records[4].file_type = 1;
    records[4].version = 1;
    records[4].occupied = true;
    assert(ghostos_volume_remove_directory(records, 5, slash, sizeof slash, &generation) == 4);
    assert(ghostos_volume_remove_directory(records, 5, versioned, sizeof versioned, &generation) == 4);
    assert(ghostos_volume_remove_directory(records, 5, log, sizeof log, &generation) == 2);
    assert(ghostos_volume_remove_directory(records, 5, archive, sizeof archive, &generation) == 3);
    assert(!records[1].deleted && generation == 3);
    assert(ghostos_volume_remove_directory(records, 5, data, sizeof data, &generation) == 3);
    records[2].deleted = true;
    assert(!ghostos_volume_remove_directory(records, 5, archive, sizeof archive, &generation));
    assert(records[1].deleted && records[1].size == 0 && records[1].checksum == 0xcbf29ce484222325ull);
    assert(generation == 4);
    assert(ghostos_volume_remove_directory(records, 5, archive, sizeof archive, &generation) == 1);
    assert(!ghostos_volume_remove_directory(records, 5, nest, sizeof nest, &generation));
    assert(records[3].deleted && generation == 5);
}
int main(void) {
    occupied_archive_stays_until_the_log_is_gone();
    return 0;
}
