#ifndef GHOSTOS_VOLUME_RETENTION_H
#define GHOSTOS_VOLUME_RETENTION_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid version. keep_latest of 0 is invalid.
   Purge tombstones the oldest live versions until only keep_latest remain,
   and stops after limit tombstones. Each tombstone advances the generation. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length;
    uint32_t version;
    bool deleted;
} ghostos_volume_version;
int ghostos_volume_purge(ghostos_volume_version *versions, size_t count, const uint8_t *name, size_t name_length,
    uint32_t keep_latest, size_t limit, uint64_t *generation, size_t *purged);
#endif
