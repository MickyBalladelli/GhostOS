#ifndef GHOSTOS_VOLUME_CURRENT_LINK_H
#define GHOSTOS_VOLUME_CURRENT_LINK_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 3 invalid version, 4 invalid path.
   A name counts when its live record is also the newest record. A newer
   tombstone drops that name even if an older version is still live. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length;
    uint32_t version;
    uint64_t object_id;
    bool occupied, deleted;
} ghostos_volume_current_record;
int ghostos_volume_current_link_count(const ghostos_volume_current_record *records, size_t count, const uint8_t *path, size_t path_length, uint32_t *link_count);
#endif
