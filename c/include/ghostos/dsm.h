#ifndef GHOSTOS_DSM_H
#define GHOSTOS_DSM_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 corrupt packet, 2 capacity, 3 invalid range.
   Kind: page request=1, page data=2, invalidate=3, invalidate ack=4,
   lease request=5, lease grant=6. */
#define GHOSTOS_DSM_PAGE 4096u
#define GHOSTOS_DSM_FRAGMENT 1400u
#define GHOSTOS_DSM_FRAGMENTS 3u
typedef struct {
    uint64_t page_address;
    uint32_t sequence;
    uint8_t received;
    uint8_t bytes[GHOSTOS_DSM_PAGE];
} ghostos_dsm_assembler;
int ghostos_dsm_encode(uint8_t kind, uint32_t source, uint32_t destination, uint32_t sequence, uint64_t page_address,
    uint32_t lease_epoch, uint16_t fragment, uint16_t fragment_count, const uint8_t *payload, size_t payload_length,
    uint8_t *output, size_t capacity, size_t *written);
int ghostos_dsm_decode(const uint8_t *input, size_t length, uint8_t *kind, uint32_t *source, uint32_t *destination,
    uint32_t *sequence, uint64_t *page_address, const uint8_t **payload, size_t *payload_length);
int ghostos_dsm_page_fragment(uint32_t source, uint32_t destination, uint32_t sequence, uint64_t page_address,
    uint32_t lease_epoch, size_t fragment, const uint8_t *page, size_t page_length, uint8_t *payload, size_t payload_capacity,
    size_t *payload_length);
int ghostos_dsm_push(ghostos_dsm_assembler *assembler, uint8_t kind, uint64_t page_address, uint32_t sequence,
    uint16_t fragment, uint16_t fragment_count, const uint8_t *payload, size_t payload_length, bool *complete);
#endif
