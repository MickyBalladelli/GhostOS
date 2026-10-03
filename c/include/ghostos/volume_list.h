#ifndef GHOSTOS_VOLUME_LIST_H
#define GHOSTOS_VOLUME_LIST_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 2 not a directory, 3 invalid version,
   4 invalid path, 5 buffer too small, 9 symlink loop. Listing returns the
   latest direct child, using only the child name, in name order. A symlink
   path lists the directory it follows. */
typedef struct {
    const uint8_t *name, *data;
    uint8_t name_length, file_type;
    uint16_t mode;
    uint32_t version, link_count;
    uint64_t size;
    bool occupied, deleted;
} ghostos_volume_list_record;
typedef struct {
    uint8_t name[192];
    uint8_t name_length, file_type;
    uint16_t mode;
    uint32_t version, link_count;
    uint64_t size;
} ghostos_volume_list_entry;
int ghostos_volume_list_directory_page(const ghostos_volume_list_record *records, size_t count, const uint8_t *path, size_t path_length, size_t skip, ghostos_volume_list_entry *entries, size_t capacity, size_t *written, size_t *next, bool *has_next);
int ghostos_volume_list_directory(const ghostos_volume_list_record *records, size_t count, const uint8_t *path, size_t path_length, ghostos_volume_list_entry *entries, size_t capacity, size_t *written);
#endif
