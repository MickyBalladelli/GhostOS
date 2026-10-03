#ifndef GHOSTOS_VOLUME_SYMLINK_H
#define GHOSTOS_VOLUME_SYMLINK_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 2 not a directory, 3 not a symlink,
   4 invalid path, 5 already exists, 6 capacity, 7 version overflow,
   8 buffer too small, 9 symlink loop. A symlink stores its target bytes.
   Follow joins a relative target under the link's parent. */
typedef struct {
    const uint8_t *name, *data;
    uint8_t name_length, file_type;
    uint32_t version;
    uint64_t object_id, size, checksum;
    bool occupied, deleted;
} ghostos_volume_symlink_record;
int ghostos_volume_symlink(ghostos_volume_symlink_record *records, size_t capacity, const uint8_t *target, size_t target_length, const uint8_t *link, size_t link_length, uint64_t *generation, uint64_t *next_object);
int ghostos_volume_read_link(const ghostos_volume_symlink_record *records, size_t capacity, const uint8_t *path, size_t path_length, uint8_t *output, size_t output_capacity, size_t *read);
int ghostos_volume_follow(const ghostos_volume_symlink_record *records, size_t capacity, const uint8_t *path, size_t path_length, uint8_t *resolved, size_t resolved_capacity, size_t *resolved_length);
#endif
