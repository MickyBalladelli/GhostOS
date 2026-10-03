#ifndef GHOSTOS_FSD_WILDCARD_H
#define GHOSTOS_FSD_WILDCARD_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid version, 2 invalid pattern, 3 buffer too small,
   4 not found, 5 access denied. Delete requires delete|write|admin, bits 2|4|8.
   A version of 0, or no suffix, selects the newest live record for each name. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length;
    uint32_t version, cursor;
    bool deleted;
} ghostos_fsd_record;
int ghostos_fsd_expand(const uint8_t *pattern, size_t pattern_length, const ghostos_fsd_record *records,
    size_t record_count, size_t continuation, size_t *matches, size_t capacity, size_t *count, size_t *next, bool *has_next);
int ghostos_fsd_expand_delete(uint16_t rights, const uint8_t *pattern, size_t pattern_length, const ghostos_fsd_record *records,
    size_t record_count, size_t continuation, size_t *matches, size_t capacity, size_t *count, size_t *next, bool *has_next);
#endif
