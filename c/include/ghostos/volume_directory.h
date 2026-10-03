#ifndef GHOSTOS_VOLUME_DIRECTORY_H
#define GHOSTOS_VOLUME_DIRECTORY_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 2 not a directory, 3 invalid path,
   4 already exists, 5 capacity, 6 version overflow. A recursive create fills
   missing parents. Each new directory advances the generation. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length, file_type;
    uint32_t version;
    bool occupied, deleted;
} ghostos_volume_directory_record;
int ghostos_volume_create_directory(ghostos_volume_directory_record *records, size_t capacity, const uint8_t *path, size_t path_length, bool recursive, uint64_t *generation, size_t *created);
#endif
