#ifndef GHOSTOS_VOLUME_REMOVE_H
#define GHOSTOS_VOLUME_REMOVE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 2 not a directory, 3 directory not empty,
   4 invalid path. Removal tombstones the latest directory record. A direct
   live child blocks removal. A nested path does not. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length, file_type;
    uint32_t version;
    uint64_t size, checksum;
    bool occupied, deleted;
} ghostos_volume_remove_record;
int ghostos_volume_remove_directory(ghostos_volume_remove_record *records, size_t count, const uint8_t *path, size_t path_length, uint64_t *generation);
#endif
