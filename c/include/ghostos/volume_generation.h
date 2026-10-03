#ifndef GHOSTOS_VOLUME_GENERATION_H
#define GHOSTOS_VOLUME_GENERATION_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 corrupt, 2 version overflow.
   Two banks are compared by sequence. Equal highest sequences are corrupt.
   A bank that fails to load is skipped in favor of an older valid bank. */
typedef struct {
    bool valid;
    uint64_t sequence, generation;
    uint32_t version;
} ghostos_volume_bank;
typedef struct {
    uint8_t active;
    uint64_t sequence;
    ghostos_volume_bank banks[2];
} ghostos_volume;
void ghostos_volume_format(ghostos_volume *volume);
int ghostos_volume_flush(ghostos_volume *volume, uint64_t generation, uint32_t version, uint64_t *sequence);
int ghostos_volume_select(const ghostos_volume *volume, const bool readable[2], uint64_t *sequence, uint64_t *generation, uint32_t *version);
#endif
