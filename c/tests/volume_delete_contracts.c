#include "ghostos/volume_delete.h"
#include <assert.h>
#include <string.h>
static void delete_removes_one_log_version(void) {
    const uint8_t archive[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e'};
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t exact[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g', ';', '1'};
    const uint8_t bad[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g', ';', 'w', 'a', 't'};
    const uint8_t alias[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'a', 'l', 'i', 'a', 's'};
    const uint8_t slash[] = {'/'};
    ghostos_volume_delete_record records[4];
    uint64_t generation = 5;
    uint32_t links = 9;
    memset(records, 0, sizeof records);
    records[0].name = archive;
    records[0].name_length = sizeof archive;
    records[0].file_type = 2;
    records[0].version = 1;
    records[0].occupied = true;
    records[1].name = log;
    records[1].name_length = sizeof log;
    records[1].file_type = 1;
    records[1].version = 1;
    records[1].object_id = 7;
    records[1].size = 3;
    records[1].occupied = true;
    records[2].name = log;
    records[2].name_length = sizeof log;
    records[2].file_type = 1;
    records[2].version = 2;
    records[2].object_id = 7;
    records[2].size = 3;
    records[2].occupied = true;
    records[3].name = alias;
    records[3].name_length = sizeof alias;
    records[3].file_type = 3;
    records[3].version = 1;
    records[3].object_id = 7;
    records[3].size = 17;
    records[3].occupied = true;
    assert(ghostos_volume_delete(records, 4, slash, sizeof slash, &generation, &links) == 4);
    assert(ghostos_volume_delete(records, 4, bad, sizeof bad, &generation, &links) == 3);
    assert(ghostos_volume_delete(records, 4, archive, sizeof archive, &generation, &links) == 2);
    assert(generation == 5 && links == 9);
    assert(!ghostos_volume_delete(records, 4, alias, sizeof alias, &generation, &links));
    assert(records[3].deleted && records[3].size == 0 && records[3].checksum == 0xcbf29ce484222325ull);
    assert(links == 1 && generation == 6 && !records[2].deleted);
    assert(!ghostos_volume_delete(records, 4, log, sizeof log, &generation, &links));
    assert(records[2].deleted && !records[1].deleted && links == 1 && generation == 7);
    assert(!ghostos_volume_delete(records, 4, exact, sizeof exact, &generation, &links));
    assert(records[1].deleted && records[1].checksum == 0xcbf29ce484222325ull && !links && generation == 8);
    assert(ghostos_volume_delete(records, 4, log, sizeof log, &generation, &links) == 1);
}
int main(void) {
    delete_removes_one_log_version();
    return 0;
}
