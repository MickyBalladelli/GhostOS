#include "ghostos/hdm.h"
enum {
    GHOSTOS_HDM_CAPABILITY = 0x00,
    GHOSTOS_HDM_STRIDE = 0x20,
    GHOSTOS_HDM_DECODER = 0x10,
    GHOSTOS_HDM_CONTROL = 0x10,
    GHOSTOS_HDM_DPA_HIGH = 0x18,
    GHOSTOS_HDM_COMMIT = 1u << 9,
    GHOSTOS_HDM_COMMITTED = 1u << 10,
    GHOSTOS_HDM_ERROR = 1u << 11,
    GHOSTOS_HDM_HOST_ONLY = 1u << 12
};
static uint32_t load32(const uint8_t *bytes) {
    return (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8) | ((uint32_t)bytes[2] << 16) | ((uint32_t)bytes[3] << 24);
}
static void store32(uint8_t *bytes, uint32_t value) {
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8);
    bytes[2] = (uint8_t)(value >> 16);
    bytes[3] = (uint8_t)(value >> 24);
}
static int registers_ok(size_t bytes, size_t hdm_offset) {
    if (hdm_offset > bytes || bytes - hdm_offset < 0x30) return 3;
    return 0;
}
int ghostos_hdm_decoder_count(const uint8_t *registers, size_t bytes, size_t hdm_offset, uint8_t *count) {
    uint32_t encoded;
    int status = registers_ok(bytes, hdm_offset);
    if (status) return status;
    encoded = load32(registers + hdm_offset + GHOSTOS_HDM_CAPABILITY) & 0x0fu;
    if (encoded > 5) return 2;
    *count = (uint8_t)(1u << encoded);
    return 0;
}
int ghostos_hdm_program(uint8_t *registers, size_t bytes, size_t hdm_offset, uint8_t decoder, uint64_t host_start,
    uint64_t host_length, uint64_t device_offset, uint8_t interleave_granularity, uint8_t interleave_ways,
    bool host_only_coherent, size_t spin_limit) {
    size_t offset;
    uint8_t count = 0;
    uint32_t control;
    int status;
    if (host_start % GHOSTOS_HDM_GRANULARITY || host_length % GHOSTOS_HDM_GRANULARITY || device_offset % GHOSTOS_HDM_GRANULARITY ||
        interleave_granularity > 6 || interleave_ways > 6)
        return 1;
    status = ghostos_hdm_decoder_count(registers, bytes, hdm_offset, &count);
    if (status) return status;
    if (decoder >= count) return 3;
    if (hdm_offset > SIZE_MAX - GHOSTOS_HDM_DECODER || decoder > (SIZE_MAX - hdm_offset - GHOSTOS_HDM_DECODER) / GHOSTOS_HDM_STRIDE)
        return 3;
    offset = hdm_offset + GHOSTOS_HDM_DECODER + (size_t)decoder * GHOSTOS_HDM_STRIDE;
    if (offset > bytes || bytes - offset < GHOSTOS_HDM_DPA_HIGH + 4) return 3;
    if (load32(registers + offset + GHOSTOS_HDM_CONTROL) & GHOSTOS_HDM_COMMITTED) return 4;
    store32(registers + offset, (uint32_t)host_start);
    store32(registers + offset + 4, (uint32_t)(host_start >> 32));
    store32(registers + offset + 8, (uint32_t)host_length);
    store32(registers + offset + 12, (uint32_t)(host_length >> 32));
    store32(registers + offset + 20, (uint32_t)device_offset);
    store32(registers + offset + 24, (uint32_t)(device_offset >> 32));
    control = interleave_granularity | ((uint32_t)interleave_ways << 4) | GHOSTOS_HDM_COMMIT;
    if (host_only_coherent) control |= GHOSTOS_HDM_HOST_ONLY;
    store32(registers + offset + GHOSTOS_HDM_CONTROL, control);
    for (;;) {
        uint32_t status_bits = load32(registers + offset + GHOSTOS_HDM_CONTROL);
        if (status_bits & GHOSTOS_HDM_ERROR) return 5;
        if (status_bits & GHOSTOS_HDM_COMMITTED) return 0;
        if (!spin_limit) return 5;
        spin_limit -= 1;
    }
}
