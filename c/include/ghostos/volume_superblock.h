#ifndef GHOSTOS_VOLUME_SUPERBLOCK_H
#define GHOSTOS_VOLUME_SUPERBLOCK_H
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not a candidate, 2 buffer too small.
   The block is 4096 bytes and begins with SYNFSVOL. The checksum covers the
   first 4088 bytes. Format versions 3 and 4 are accepted. Sequence 0 is not. */
#define GHOSTOS_VOLUME_SUPERBLOCK 4096u
int ghostos_volume_superblock_encode(uint16_t format_version, uint64_t sequence, uint64_t generation, uint32_t root,
    uint64_t next_checkpoint, uint64_t next_object_id, uint64_t type_map_checksum, uint64_t max_blocks, uint8_t *block,
    size_t capacity);
int ghostos_volume_superblock_decode(const uint8_t *block, size_t capacity, uint64_t expected_blocks, uint16_t *format_version,
    uint64_t *sequence, uint64_t *generation, uint32_t *root);
#endif
