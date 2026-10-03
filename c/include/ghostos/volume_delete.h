#ifndef GHOSTOS_VOLUME_DELETE_H
#define GHOSTOS_VOLUME_DELETE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 2 not a directory, 3 invalid version,
   4 invalid path. Delete tombstones the selected version, clears its size,
   and reports how many latest names still share its object. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length, file_type;
    uint32_t version;
    uint64_t object_id, size, checksum;
    bool occupied, deleted;
} ghostos_volume_delete_record;
int ghostos_volume_delete(ghostos_volume_delete_record *records, size_t count, const uint8_t *path, size_t path_length, uint64_t *generation, uint32_t *link_count);
#endif
