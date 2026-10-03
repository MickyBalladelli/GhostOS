#ifndef GHOSTOS_VOLUME_LINK_LIST_H
#define GHOSTOS_VOLUME_LINK_LIST_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 3 invalid version, 4 invalid path,
   5 buffer too small. Each entry is a latest name sharing the selected
   object. The entry version is zero because that name is current. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length;
    uint32_t version;
    uint64_t object_id;
    bool occupied, deleted;
} ghostos_volume_link_list_record;
typedef struct {
    uint8_t name[192];
    uint8_t name_length;
    uint32_t version;
} ghostos_volume_link_list_entry;
int ghostos_volume_list_links(const ghostos_volume_link_list_record *records, size_t count, const uint8_t *path, size_t path_length, ghostos_volume_link_list_entry *entries, size_t capacity, size_t *written);
#endif
