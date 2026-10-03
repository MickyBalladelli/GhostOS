#ifndef GHOSTOS_VOLUME_BLOCK_H
#define GHOSTOS_VOLUME_BLOCK_H
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 corrupt, 2 buffer too small.
   A block is 4096 bytes. Data payload starts at byte 16. Tree magic is SYNT. */
#define GHOSTOS_VOLUME_BLOCK 4096u
#define GHOSTOS_VOLUME_DATA (GHOSTOS_VOLUME_BLOCK - 16u)
int ghostos_volume_encode_data(uint32_t next, const uint8_t *payload, size_t length, uint8_t *block, size_t capacity);
int ghostos_volume_decode_data(const uint8_t *block, size_t capacity, uint32_t *next, uint8_t *payload, size_t payload_capacity, size_t *length);
int ghostos_volume_encode_tree(uint8_t kind, const uint8_t *payload, size_t length, uint8_t *block, size_t capacity);
int ghostos_volume_decode_tree(const uint8_t *block, size_t capacity, uint8_t *kind, uint8_t *payload, size_t payload_capacity, size_t *length);
int ghostos_volume_decode_empty(const uint8_t *block, size_t capacity);
#endif
