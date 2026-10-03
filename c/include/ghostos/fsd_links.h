#ifndef GHOSTOS_FSD_LINKS_H
#define GHOSTOS_FSD_LINKS_H
#include "ghostos/fsd_wildcard.h"
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 access denied, 2 invalid version, 3 invalid pattern,
   4 not found, 5 buffer too small, 6 partial match.
   Listing requires the read right, bit 0. */
int ghostos_fsd_list_links(uint16_t rights, const uint8_t *pattern, size_t pattern_length, const ghostos_fsd_record *records,
    size_t record_count, size_t *matches, size_t match_capacity, uint8_t *output, size_t output_capacity, size_t *written);
#endif
