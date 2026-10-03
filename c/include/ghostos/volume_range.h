#ifndef GHOSTOS_VOLUME_RANGE_H
#define GHOSTOS_VOLUME_RANGE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 2 not a directory, 3 invalid version,
   4 invalid path, 5 corrupt, 9 symlink loop. An offset past the file is an
   invalid version. A short buffer copies only what fits. A symlink is followed. */
typedef struct {
    const uint8_t *bytes;
    uint16_t length;
    uint32_t next;
    uint64_t checksum;
} ghostos_volume_range_block;
typedef struct {
    const uint8_t *name, *data;
    uint8_t name_length, file_type;
    uint32_t version, first_block;
    uint64_t size;
    bool occupied, deleted;
    uint64_t checksum;
} ghostos_volume_range_file;
int ghostos_volume_read_at(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, uint64_t offset, uint8_t *output, size_t output_capacity, size_t *read);
/* Whole-file read also checks the record checksum and the complete chain.
   Result 6 means buffer too small; required receives the full file size after
   lookup/type/size validation. read changes only on success. Corruption may
   leave copied bytes in output. Resolution currently follows absolute links
   at the complete path only; relative and intermediate links remain pending. */
int ghostos_volume_read(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, uint8_t *output, size_t output_capacity, size_t *read, size_t *required);
#endif
