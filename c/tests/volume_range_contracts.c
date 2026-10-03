#include "ghostos/volume_range.h"
#include <assert.h>
#include <string.h>
static uint64_t fnv(const uint8_t *bytes, size_t length) {
    uint64_t hash = 0xcbf29ce484222325ull;
    size_t i;
    for (i = 0; i < length; ++i) {
        hash ^= bytes[i];
        hash *= 0x100000001b3ull;
    }
    return hash;
}
static void offset_three_reads_defg(void) {
    const uint8_t log[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t link[] = {'/', 'd', 'a', 't', 'a', '/', 'l', 'i', 'n', 'k'};
    const uint8_t archive[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e'};
    const uint8_t bad[] = {'/', 'd', 'a', 't', 'a', ';', 'w', 'a', 't'};
    const uint8_t first[] = {'a', 'b', 'c', 'd'};
    const uint8_t second[] = {'e', 'f', 'g', 'h', 'i', 'j'};
    ghostos_volume_range_block blocks[2];
    ghostos_volume_range_file files[3];
    uint8_t output[10];
    size_t read = 9;
    memset(blocks, 0, sizeof blocks);
    memset(files, 0, sizeof files);
    blocks[0].bytes = first;
    blocks[0].length = 4;
    blocks[0].next = 2;
    blocks[0].checksum = fnv(first, 4);
    blocks[1].bytes = second;
    blocks[1].length = 6;
    blocks[1].checksum = fnv(second, 6);
    files[0].name = log;
    files[0].name_length = sizeof log;
    files[0].file_type = 1;
    files[0].version = 1;
    files[0].first_block = 1;
    files[0].size = 10;
    files[0].occupied = true;
    files[1].name = archive;
    files[1].name_length = sizeof archive;
    files[1].file_type = 2;
    files[1].version = 1;
    files[1].occupied = true;
    files[2].name = link;
    files[2].name_length = sizeof link;
    files[2].data = log;
    files[2].file_type = 3;
    files[2].version = 1;
    files[2].size = sizeof log;
    files[2].occupied = true;
    assert(ghostos_volume_read_at(files, 3, blocks, 2, bad, sizeof bad, 0, output, 4, &read) == 3);
    assert(read == 9);
    assert(ghostos_volume_read_at(files, 3, blocks, 2, archive, sizeof archive, 0, output, 4, &read) == 2);
    assert(ghostos_volume_read_at(files, 3, blocks, 2, log, sizeof log, 11, output, 4, &read) == 3);
    assert(!ghostos_volume_read_at(files, 3, blocks, 2, log, sizeof log, 10, output, 4, &read));
    assert(!read);
    assert(!ghostos_volume_read_at(files, 3, blocks, 2, log, sizeof log, 3, output, 4, &read));
    assert(read == 4 && !memcmp(output, "defg", 4));
    assert(!ghostos_volume_read_at(files, 3, blocks, 2, link, sizeof link, 0, output, 3, &read));
    assert(read == 3 && !memcmp(output, "abc", 3));
    blocks[0].checksum ^= 1;
    assert(ghostos_volume_read_at(files, 3, blocks, 2, log, sizeof log, 0, output, 4, &read) == 5);
    blocks[0].checksum ^= 1;
    blocks[0].next = 0;
    assert(ghostos_volume_read_at(files, 3, blocks, 2, log, sizeof log, 0, output, 10, &read) == 5);
}
int main(void) {
    offset_three_reads_defg();
    return 0;
}
