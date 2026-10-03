#ifndef GHOSTOS_VOLUME_PAYLOAD_H
#define GHOSTOS_VOLUME_PAYLOAD_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 corrupt. Block ids are 1-based and 0 ends the chain.
   Each block checksum covers that block's bytes. The record checksum is the
   running hash of every byte in order, and the summed lengths equal the size. */
#define GHOSTOS_VOLUME_PAYLOAD_BLOCKS 8u
typedef struct {
    uint32_t next;
    uint16_t length;
    uint64_t checksum;
    const uint8_t *bytes;
} ghostos_volume_payload;
int ghostos_volume_check_payload(const ghostos_volume_payload *blocks, size_t block_count, uint32_t first, uint64_t size,
    uint64_t expected_checksum, uint64_t object_id, uint64_t *owners, bool *owner_set);
#endif
