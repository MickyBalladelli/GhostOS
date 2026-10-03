#ifndef GHOSTOS_VOLUME_RENAME_H
#define GHOSTOS_VOLUME_RENAME_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 2 not a directory, 3 already exists,
   4 invalid path, 5 capacity. Rename moves the latest record. A directory
   also moves its latest descendants. Older versions stay. */
enum { GHOSTOS_VOLUME_RENAME_PATH = 192 };
typedef struct {
    uint8_t name[GHOSTOS_VOLUME_RENAME_PATH];
    const uint8_t *data;
    uint8_t name_length, file_type;
    uint32_t version;
    uint64_t object_id, size, checksum;
    bool occupied, deleted;
} ghostos_volume_rename_record;
int ghostos_volume_rename(ghostos_volume_rename_record *records, size_t count, const uint8_t *old_path, size_t old_length, const uint8_t *new_path, size_t new_length, uint64_t *generation);
#endif
