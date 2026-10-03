#include "ghostos/dsm.h"
static void store_be16(uint8_t *bytes, uint16_t value) {
    bytes[0] = (uint8_t)(value >> 8);
    bytes[1] = (uint8_t)value;
}
static void store_be32(uint8_t *bytes, uint32_t value) {
    bytes[0] = (uint8_t)(value >> 24);
    bytes[1] = (uint8_t)(value >> 16);
    bytes[2] = (uint8_t)(value >> 8);
    bytes[3] = (uint8_t)value;
}
static void store_be64(uint8_t *bytes, uint64_t value) {
    size_t i;
    for (i = 0; i < 8; ++i) bytes[i] = (uint8_t)(value >> (8 * (7 - i)));
}
static uint16_t load_be16(const uint8_t *bytes) { return (uint16_t)(((uint16_t)bytes[0] << 8) | bytes[1]); }
static uint32_t load_be32(const uint8_t *bytes) {
    return ((uint32_t)bytes[0] << 24) | ((uint32_t)bytes[1] << 16) | ((uint32_t)bytes[2] << 8) | bytes[3];
}
static uint64_t load_be64(const uint8_t *bytes) {
    uint64_t value = 0;
    size_t i;
    for (i = 0; i < 8; ++i) value = (value << 8) | bytes[i];
    return value;
}
static int header_ok(uint8_t kind, uint32_t source, uint32_t destination, uint64_t page_address, uint16_t fragment, uint16_t fragment_count, size_t payload_length) {
    if (!kind || kind > 6 || !source || !destination || !fragment_count || fragment >= fragment_count || page_address % GHOSTOS_DSM_PAGE != 0 || payload_length > GHOSTOS_DSM_FRAGMENT)
        return 1;
    return 0;
}
int ghostos_dsm_encode(uint8_t kind, uint32_t source, uint32_t destination, uint32_t sequence, uint64_t page_address,
    uint32_t lease_epoch, uint16_t fragment, uint16_t fragment_count, const uint8_t *payload, size_t payload_length,
    uint8_t *output, size_t capacity, size_t *written) {
    size_t length = 32 + payload_length, i;
    int status = header_ok(kind, source, destination, page_address, fragment, fragment_count, payload_length);
    if (status) return status;
    if (capacity < length) return 2;
    for (i = 0; i < length; ++i) output[i] = 0;
    output[0] = 0x88;
    output[1] = 0xb5;
    output[2] = 1;
    output[3] = kind;
    store_be32(output + 4, source);
    store_be32(output + 8, destination);
    store_be32(output + 12, sequence);
    store_be64(output + 16, page_address);
    store_be32(output + 24, lease_epoch);
    store_be16(output + 28, fragment);
    store_be16(output + 30, fragment_count);
    for (i = 0; i < payload_length; ++i) output[32 + i] = payload[i];
    *written = length;
    return 0;
}
int ghostos_dsm_decode(const uint8_t *input, size_t length, uint8_t *kind, uint32_t *source, uint32_t *destination,
    uint32_t *sequence, uint64_t *page_address, const uint8_t **payload, size_t *payload_length) {
    uint16_t fragment, fragment_count;
    int status;
    if (length < 32 || input[0] != 0x88 || input[1] != 0xb5 || input[2] != 1) return 1;
    fragment = load_be16(input + 28);
    fragment_count = load_be16(input + 30);
    status = header_ok(input[3], load_be32(input + 4), load_be32(input + 8), load_be64(input + 16), fragment, fragment_count, length - 32);
    if (status) return status;
    *kind = input[3];
    *source = load_be32(input + 4);
    *destination = load_be32(input + 8);
    *sequence = load_be32(input + 12);
    *page_address = load_be64(input + 16);
    *payload = input + 32;
    *payload_length = length - 32;
    return 0;
}
int ghostos_dsm_page_fragment(uint32_t source, uint32_t destination, uint32_t sequence, uint64_t page_address,
    uint32_t lease_epoch, size_t fragment, const uint8_t *page, size_t page_length, uint8_t *payload, size_t payload_capacity,
    size_t *payload_length) {
    size_t start, end, i;
    (void)source;
    (void)destination;
    (void)sequence;
    (void)lease_epoch;
    if (page_address % GHOSTOS_DSM_PAGE != 0) return 1;
    if (fragment >= GHOSTOS_DSM_FRAGMENTS) return 3;
    start = fragment * GHOSTOS_DSM_FRAGMENT;
    end = start + GHOSTOS_DSM_FRAGMENT;
    if (end > page_length) end = page_length;
    if (start > page_length || end < start || end - start > payload_capacity) return 1;
    for (i = start; i < end; ++i) payload[i - start] = page[i];
    *payload_length = end - start;
    return 0;
}
int ghostos_dsm_push(ghostos_dsm_assembler *assembler, uint8_t kind, uint64_t page_address, uint32_t sequence,
    uint16_t fragment, uint16_t fragment_count, const uint8_t *payload, size_t payload_length, bool *complete) {
    size_t start, end, i;
    if (kind != 2 || page_address != assembler->page_address || sequence != assembler->sequence || fragment_count != GHOSTOS_DSM_FRAGMENTS || fragment >= GHOSTOS_DSM_FRAGMENTS)
        return 1;
    start = (size_t)fragment * GHOSTOS_DSM_FRAGMENT;
    if (payload_length > SIZE_MAX - start) return 1;
    end = start + payload_length;
    if (end > GHOSTOS_DSM_PAGE || (fragment + 1 < GHOSTOS_DSM_FRAGMENTS && payload_length != GHOSTOS_DSM_FRAGMENT)) return 1;
    for (i = 0; i < payload_length; ++i) assembler->bytes[start + i] = payload[i];
    assembler->received = (uint8_t)(assembler->received | (uint8_t)(1u << fragment));
    *complete = assembler->received == (uint8_t)((1u << GHOSTOS_DSM_FRAGMENTS) - 1);
    return 0;
}
