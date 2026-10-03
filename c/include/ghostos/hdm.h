#ifndef GHOSTOS_HDM_H
#define GHOSTOS_HDM_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 alignment, 2 unsupported, 3 invalid device, 4 busy,
   5 decoder commit failed. Registers are little-endian. */
#define GHOSTOS_HDM_GRANULARITY (256u * 1024u * 1024u)
int ghostos_hdm_decoder_count(const uint8_t *registers, size_t bytes, size_t hdm_offset, uint8_t *count);
int ghostos_hdm_program(uint8_t *registers, size_t bytes, size_t hdm_offset, uint8_t decoder, uint64_t host_start,
    uint64_t host_length, uint64_t device_offset, uint8_t interleave_granularity, uint8_t interleave_ways,
    bool host_only_coherent, size_t spin_limit);
#endif
