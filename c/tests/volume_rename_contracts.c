#include "ghostos/volume_rename.h"
#include <assert.h>
#include <string.h>
static void set_name(ghostos_volume_rename_record *record, const uint8_t *path, uint8_t length) {
    memcpy(record->name, path, length);
    record->name_length = length;
}
static void latest_log_moves_and_version_one_stays(void) {
    const uint8_t data[] = {'/', 'd', 'a', 't', 'a'};
    const uint8_t archive[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e'};
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t note[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'n', 'o', 't', 'e'};
    const uint8_t inner[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'i', 'n', 'n', 'e', 'r'};
    const uint8_t slash[] = {'/'};
    const uint8_t bytes[] = {'n', 'e', 'w'};
    ghostos_volume_rename_record records[5];
    uint64_t generation = 4;
    memset(records, 0, sizeof records);
    set_name(&records[0], data, sizeof data);
    records[0].file_type = 2;
    records[0].version = 1;
    records[0].occupied = true;
    set_name(&records[1], archive, sizeof archive);
    records[1].file_type = 2;
    records[1].version = 1;
    records[1].occupied = true;
    set_name(&records[2], log, sizeof log);
    records[2].file_type = 1;
    records[2].version = 1;
    records[2].object_id = 7;
    records[2].size = 3;
    records[2].occupied = true;
    set_name(&records[3], log, sizeof log);
    records[3].data = bytes;
    records[3].file_type = 1;
    records[3].version = 2;
    records[3].object_id = 7;
    records[3].size = 3;
    records[3].checksum = 99;
    records[3].occupied = true;
    assert(ghostos_volume_rename(records, 4, slash, sizeof slash, note, sizeof note, &generation) == 4);
    assert(ghostos_volume_rename(records, 4, log, sizeof log, log, sizeof log, &generation) == 3);
    assert(ghostos_volume_rename(records, 4, archive, sizeof archive, inner, sizeof inner, &generation) == 4);
    assert(generation == 4);
    assert(ghostos_volume_rename(records, 4, log, sizeof log, note, sizeof note, &generation) == 5);
    assert(!ghostos_volume_rename(records, 5, log, sizeof log, note, sizeof note, &generation));
    assert(records[3].deleted && records[3].size == 0 && records[3].checksum == 0xcbf29ce484222325ull);
    assert(!records[2].deleted && records[2].version == 1 && records[2].name_length == sizeof log);
    assert(records[4].version == 1 && records[4].object_id == 7 && records[4].size == 3 && records[4].checksum == 99);
    assert(records[4].name_length == sizeof note && !memcmp(records[4].name, note, sizeof note));
}
static void directory_rename_moves_the_child_until_a_target_exists(void) {
    const uint8_t data[] = {'/', 'd', 'a', 't', 'a'};
    const uint8_t nest[] = {'/', 'd', 'a', 't', 'a', '/', 'n', 'e', 's', 't'};
    const uint8_t file[] = {'/', 'd', 'a', 't', 'a', '/', 'n', 'e', 's', 't', '/', 'f', 'i', 'l', 'e'};
    const uint8_t kept[] = {'/', 'd', 'a', 't', 'a', '/', 'k', 'e', 'p', 't'};
    const uint8_t kept_file[] = {'/', 'd', 'a', 't', 'a', '/', 'k', 'e', 'p', 't', '/', 'f', 'i', 'l', 'e'};
    ghostos_volume_rename_record records[5];
    uint64_t generation = 2;
    memset(records, 0, sizeof records);
    set_name(&records[0], data, sizeof data);
    records[0].file_type = 2;
    records[0].version = 1;
    records[0].occupied = true;
    set_name(&records[1], nest, sizeof nest);
    records[1].file_type = 2;
    records[1].version = 1;
    records[1].occupied = true;
    set_name(&records[2], file, sizeof file);
    records[2].file_type = 1;
    records[2].version = 1;
    records[2].object_id = 8;
    records[2].size = 4;
    records[2].occupied = true;
    set_name(&records[3], kept_file, sizeof kept_file);
    records[3].file_type = 1;
    records[3].version = 1;
    records[3].occupied = true;
    assert(ghostos_volume_rename(records, 5, nest, sizeof nest, kept, sizeof kept, &generation) == 3);
    assert(records[1].deleted && generation == 3);
    assert(!records[2].deleted && records[2].name_length == sizeof file);
    assert(records[4].occupied && records[4].file_type == 2 && records[4].name_length == sizeof kept);
    assert(!memcmp(records[4].name, kept, sizeof kept));
}
int main(void) {
    latest_log_moves_and_version_one_stays();
    directory_rename_moves_the_child_until_a_target_exists();
    return 0;
}
