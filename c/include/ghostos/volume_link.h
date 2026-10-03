#ifndef GHOSTOS_VOLUME_LINK_H
#define GHOSTOS_VOLUME_LINK_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 2 not a directory, 3 invalid version,
   4 already exists, 5 capacity, 6 version overflow. A link copies the source
   object id. The reported count is the number of latest names that share it. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length, file_type;
    uint32_t version;
    uint64_t object_id;
    bool deleted;
} ghostos_volume_link_record;
int ghostos_volume_link(ghostos_volume_link_record *records, size_t count, const uint8_t *source, size_t source_length, const uint8_t *target, size_t target_length, uint64_t *generation);
int ghostos_volume_link_count(const ghostos_volume_link_record *records, size_t count, const uint8_t *path, size_t path_length, uint32_t *link_count);
#endif
