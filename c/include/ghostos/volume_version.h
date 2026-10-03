#ifndef GHOSTOS_VOLUME_VERSION_H
#define GHOSTOS_VOLUME_VERSION_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid version, 2 not found, 3 buffer too small,
   4 not a directory. Version 0 is invalid. A deleted record is not found.
   The selected version is copied only when the buffer holds the whole file. */
typedef struct {
    const uint8_t *name, *data;
    uint8_t name_length, data_length, file_type;
    uint32_t version;
    bool deleted;
} ghostos_volume_exact;
int ghostos_volume_read_version(const ghostos_volume_exact *files, size_t count, const uint8_t *name, size_t name_length, uint32_t version, uint8_t *output, size_t output_capacity, size_t *read);
#endif
