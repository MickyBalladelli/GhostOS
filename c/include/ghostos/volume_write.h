#ifndef GHOSTOS_VOLUME_WRITE_H
#define GHOSTOS_VOLUME_WRITE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 2 not a directory, 3 invalid version,
   4 invalid path, 5 capacity, 6 version overflow. A live file keeps its object
   id and stores the next version. An empty version 1 with no data is replaced. */
typedef struct {
    const uint8_t *name, *data;
    uint8_t name_length, file_type;
    uint32_t version;
    uint64_t object_id, size, checksum;
    bool occupied, deleted;
} ghostos_volume_write_record;
int ghostos_volume_write(ghostos_volume_write_record *records, size_t capacity, const uint8_t *path, size_t path_length, const uint8_t *contents, size_t contents_length, uint64_t *generation, uint64_t *next_object);
#endif
