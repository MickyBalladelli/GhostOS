#ifndef GHOSTOS_VOLUME_MODE_H
#define GHOSTOS_VOLUME_MODE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 not found, 3 invalid version, 4 invalid path.
   The mode is masked to 07777 and stored on the selected version. A symlink
   path changes the link itself. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length, file_type;
    uint16_t mode;
    uint32_t version;
    bool occupied, deleted;
} ghostos_volume_mode_record;
int ghostos_volume_set_mode(ghostos_volume_mode_record *records, size_t count, const uint8_t *path, size_t path_length, uint16_t mode, uint64_t *generation);
#endif
