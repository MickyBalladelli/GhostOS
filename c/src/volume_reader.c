#include "ghostos/volume_reader.h"
#include <limits.h>
static uint64_t load_le(const uint8_t *bytes, size_t count) {
    uint64_t value = 0;
    size_t i;
    for (i = 0; i < count; ++i) value |= (uint64_t)bytes[i] << (8 * i);
    return value;
}
int ghostos_volume_reader_init(ghostos_volume_reader *reader,
    const ghostos_volume_record *records, size_t record_count,
    const uint8_t *raw_blocks, const uint8_t *kinds, size_t block_count,
    ghostos_volume_range_file *files, size_t file_capacity,
    ghostos_volume_range_block *blocks, size_t block_capacity) {
    size_t i, byte;
    if (!reader || record_count > INT_MAX || block_count > UINT32_MAX ||
        block_count > SIZE_MAX / GHOSTOS_VOLUME_BLOCK) return 5;
    if (record_count > file_capacity || block_count > block_capacity) return 6;
    if ((record_count && (!records || !files)) ||
        (block_count && (!raw_blocks || !kinds || !blocks))) return 5;
    for (i = 0; i < record_count; ++i) {
        const ghostos_volume_record *record = &records[i];
        if (record->name_length > GHOSTOS_VOLUME_NAME || record->file_type > 3) return 5;
        for (byte = 0; byte < record->name_length; ++byte) if (!record->name[byte]) return 5;
    }
    for (i = 0; i < block_count; ++i) if (kinds[i] > 3) return 5;
    for (i = 0; i < record_count; ++i) {
        const ghostos_volume_record *record = &records[i];
        files[i].name = record->name;
        files[i].name_length = (uint8_t)record->name_length;
        files[i].data = 0;
        files[i].file_type = record->file_type ? record->file_type : 1;
        files[i].version = record->version;
        files[i].first_block = record->data;
        files[i].size = record->size;
        files[i].occupied = true;
        files[i].deleted = record->deleted;
        files[i].checksum = record->checksum;
    }
    for (i = 0; i < block_count; ++i) {
        const uint8_t *raw = raw_blocks + i * GHOSTOS_VOLUME_BLOCK;
        blocks[i].bytes = kinds[i] == 3 ? raw + 16 : 0;
        blocks[i].length = kinds[i] == 3 ? (uint16_t)load_le(raw + 4, 2) : 0;
        blocks[i].next = kinds[i] == 3 ? (uint32_t)load_le(raw, 4) : 0;
        blocks[i].checksum = kinds[i] == 3 ? load_le(raw + 8, 8) : 0;
    }
    reader->records = records;
    reader->files = files;
    reader->file_count = record_count;
    reader->blocks = blocks;
    reader->block_count = block_count;
    return 0;
}
int ghostos_volume_reader_lookup_following(const ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length, const ghostos_volume_record **record) {
    size_t index;
    int status = ghostos_volume_lookup_following(reader->files, reader->file_count,
        reader->blocks, reader->block_count, path, path_length, &index);
    if (status) return status;
    *record = &reader->records[index];
    return 0;
}
int ghostos_volume_reader_read(const ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length, uint8_t *output,
    size_t output_capacity, size_t *read, size_t *required) {
    return ghostos_volume_read_blocks(reader->files, reader->file_count,
        reader->blocks, reader->block_count, path, path_length, output,
        output_capacity, read, required);
}
int ghostos_volume_reader_read_at(const ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length, uint64_t offset,
    uint8_t *output, size_t output_capacity, size_t *read) {
    return ghostos_volume_read_at(reader->files, reader->file_count,
        reader->blocks, reader->block_count, path, path_length, offset,
        output, output_capacity, read);
}
int ghostos_volume_reader_read_version(const ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length, uint32_t version,
    uint8_t *output, size_t output_capacity, size_t *read, size_t *required) {
    return ghostos_volume_read_version_blocks(reader->files, reader->file_count,
        reader->blocks, reader->block_count, path, path_length, version,
        output, output_capacity, read, required);
}
