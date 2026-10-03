#ifndef GHOSTOS_FSD_LIST_H
#define GHOSTOS_FSD_LIST_H
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 access denied, 2 buffer too small.
   Read right is bit 0. Each record is 22 bytes plus the name. */
int ghostos_fsd_list(uint16_t rights, const uint8_t *const *names, const uint8_t *name_lengths, size_t count, size_t skip,
    uint8_t *output, size_t output_capacity, size_t *written, size_t *next);
#endif
