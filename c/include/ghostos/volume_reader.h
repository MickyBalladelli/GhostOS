#ifndef GHOSTOS_VOLUME_READER_H
#define GHOSTOS_VOLUME_READER_H
#include "ghostos/volume_record.h"
#include "ghostos/volume_block.h"
#include "ghostos/volume_range.h"
/* Borrowed filesystem read view. Records, raw blocks, and view arrays must
   remain alive and unchanged until the reader is rebuilt. Raw blocks are a
   contiguous array of block_count 4096-byte blocks, indexed by block ID - 1.
   Kinds use the decoded type map: empty=0, leaf=1, branch=2, data=3.
   Init results: 0 success, 5 corrupt input, 6 insufficient view capacity.
   Init leaves reader unchanged on failure. View arrays may be overwritten.
   Data checksums and chain validity are checked lazily by read operations;
   unrelated corrupt data does not prevent opening the view. */
typedef struct {
    const ghostos_volume_record *records;
    const ghostos_volume_range_file *files;
    size_t file_count;
    const ghostos_volume_range_block *blocks;
    size_t block_count;
} ghostos_volume_reader;
int ghostos_volume_reader_init(ghostos_volume_reader *reader,
    const ghostos_volume_record *records, size_t record_count,
    const uint8_t *raw_blocks, const uint8_t *kinds, size_t block_count,
    ghostos_volume_range_file *files, size_t file_capacity,
    ghostos_volume_range_block *blocks, size_t block_capacity);
/* Read results and output semantics match volume_range.h. */
int ghostos_volume_reader_lookup_following(const ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length, const ghostos_volume_record **record);
int ghostos_volume_reader_lookup(const ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length, const ghostos_volume_record **record);
int ghostos_volume_reader_read(const ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length, uint8_t *output,
    size_t output_capacity, size_t *read, size_t *required);
int ghostos_volume_reader_read_at(const ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length, uint64_t offset,
    uint8_t *output, size_t output_capacity, size_t *read);
int ghostos_volume_reader_read_version(const ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length, uint32_t version,
    uint8_t *output, size_t output_capacity, size_t *read, size_t *required);
#endif
