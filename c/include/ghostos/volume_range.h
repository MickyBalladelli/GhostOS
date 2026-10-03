#ifndef GHOSTOS_VOLUME_RANGE_H
#define GHOSTOS_VOLUME_RANGE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 2 not a directory, 3 invalid version,
   4 invalid path, 5 corrupt, 9 symlink loop. An offset past the file is an
   invalid version. A short buffer copies only what fits. Relative and absolute
   symlinks in any component are followed, up to 40 resolution passes. Targets
   use first_block when nonzero; otherwise data supplies inline target bytes. */
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
/* Return the resolved record index without reading the final payload.
   Output changes only on success. Lookup follows the same symlink and version
   rules as whole-file and ranged reads. */
int ghostos_volume_lookup_following(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, size_t *index);
int ghostos_volume_read_at(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, uint64_t offset, uint8_t *output, size_t output_capacity, size_t *read);
/* Whole-file read also checks the record checksum and the complete chain.
   Result 6 means buffer too small; required receives the full file size after
   lookup/type/size validation. read changes only on success. Corruption may
   leave copied bytes in output. Invalid UTF-8 paths are invalid path; invalid
   UTF-8 link targets and oversized link targets are corrupt. */
int ghostos_volume_read_blocks(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, uint8_t *output, size_t output_capacity, size_t *read, size_t *required);
/* Exact-version reads use the same complete-chain and checksum validation.
   Version zero is invalid. The path must have no version suffix. Symlinks are
   read as stored file bytes rather than followed, matching read_version. */
int ghostos_volume_read_version_blocks(const ghostos_volume_range_file *files, size_t file_count, const ghostos_volume_range_block *blocks, size_t block_count, const uint8_t *path, size_t path_length, uint32_t version, uint8_t *output, size_t output_capacity, size_t *read, size_t *required);
#endif
