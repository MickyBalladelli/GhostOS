#ifndef GHOSTOS_VOLUME_SNAPSHOT_H
#define GHOSTOS_VOLUME_SNAPSHOT_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 too many checkpoints, 2 version overflow, 3 not found,
   4 buffer too small. A pin keeps the newest version whose generation is not
   newer than the checkpoint. Later writes stay on the live view. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length, data_length;
    uint32_t version;
    uint64_t generation;
    const uint8_t *data;
    bool deleted;
} ghostos_volume_file;
typedef struct {
    bool occupied;
    uint64_t id, generation;
} ghostos_volume_pin;
int ghostos_volume_pin(ghostos_volume_pin *pins, size_t capacity, uint64_t *next_id, uint64_t generation, uint64_t *id);
int ghostos_volume_unpin(ghostos_volume_pin *pins, size_t capacity, uint64_t id);
int ghostos_volume_read(const ghostos_volume_file *files, size_t count, const uint8_t *name, size_t name_length, uint8_t *output, size_t output_capacity, size_t *read);
int ghostos_volume_snapshot_read(const ghostos_volume_pin *pins, size_t pin_count, const ghostos_volume_file *files, size_t file_count, uint64_t id, const uint8_t *name, size_t name_length, uint8_t *output, size_t output_capacity, size_t *read);
#endif
