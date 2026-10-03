#ifndef GHOSTOS_VOLUME_WILDCARD_H
#define GHOSTOS_VOLUME_WILDCARD_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 3 invalid version, 4 invalid path, 5 buffer too small,
   6 invalid pattern. A page returns the next live-record cursor when another
   match does not fit. `;0` keeps the latest version. `;N` keeps that version. */
typedef struct {
    const uint8_t *name;
    uint8_t name_length;
    uint32_t version;
    bool occupied, deleted;
} ghostos_volume_wildcard_record;
typedef struct {
    uint8_t name[192];
    uint8_t name_length;
    uint32_t version;
    size_t cursor;
} ghostos_volume_wildcard_entry;
int ghostos_volume_expand_page(const ghostos_volume_wildcard_record *records, size_t count, const uint8_t *pattern, size_t pattern_length, size_t continuation, ghostos_volume_wildcard_entry *entries, size_t capacity, size_t *written, size_t *next, bool *has_next);
int ghostos_volume_expand(const ghostos_volume_wildcard_record *records, size_t count, const uint8_t *pattern, size_t pattern_length, ghostos_volume_wildcard_entry *entries, size_t capacity, size_t *written);
#endif
