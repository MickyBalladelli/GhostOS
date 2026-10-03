#include "ghostos/volume_block.h"
static uint64_t checksum(const uint8_t *bytes, size_t length) {
    uint64_t hash = 0xcbf29ce484222325ull;
    size_t i;
    for (i = 0; i < length; ++i) {
        hash ^= bytes[i];
        hash *= 0x100000001b3ull;
    }
    return hash;
}
static void store_le16(uint8_t *bytes, uint16_t value) {
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8);
}
static void store_le32(uint8_t *bytes, uint32_t value) {
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8);
    bytes[2] = (uint8_t)(value >> 16);
    bytes[3] = (uint8_t)(value >> 24);
}
static void store_le64(uint8_t *bytes, uint64_t value) {
    size_t i;
    for (i = 0; i < 8; ++i) bytes[i] = (uint8_t)(value >> (8 * i));
}
static uint16_t load_le16(const uint8_t *bytes) { return (uint16_t)bytes[0] | ((uint16_t)bytes[1] << 8); }
static uint32_t load_le32(const uint8_t *bytes) {
    return (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8) | ((uint32_t)bytes[2] << 16) | ((uint32_t)bytes[3] << 24);
}
static uint64_t load_le64(const uint8_t *bytes) {
    uint64_t value = 0;
    size_t i;
    for (i = 0; i < 8; ++i) value |= (uint64_t)bytes[i] << (8 * i);
    return value;
}
static void zero(uint8_t *block) {
    size_t i;
    for (i = 0; i < GHOSTOS_VOLUME_BLOCK; ++i) block[i] = 0;
}
int ghostos_volume_encode_data(uint32_t next, const uint8_t *payload, size_t length, uint8_t *block, size_t capacity) {
    size_t i;
    if (capacity < GHOSTOS_VOLUME_BLOCK) return 2;
    if (length > GHOSTOS_VOLUME_DATA) return 1;
    zero(block);
    store_le32(block, next);
    store_le16(block + 4, (uint16_t)length);
    store_le64(block + 8, checksum(payload, length));
    for (i = 0; i < length; ++i) block[16 + i] = payload[i];
    return 0;
}
int ghostos_volume_decode_data(const uint8_t *block, size_t capacity, uint32_t *next, uint8_t *payload, size_t payload_capacity, size_t *length) {
    size_t stored, i;
    if (capacity < GHOSTOS_VOLUME_BLOCK) return 2;
    stored = load_le16(block + 4);
    if (stored > GHOSTOS_VOLUME_DATA || checksum(block + 16, stored) != load_le64(block + 8)) return 1;
    if (stored > payload_capacity) return 2;
    *next = load_le32(block);
    *length = stored;
    for (i = 0; i < stored; ++i) payload[i] = block[16 + i];
    return 0;
}
int ghostos_volume_encode_tree(uint8_t kind, const uint8_t *payload, size_t length, uint8_t *block, size_t capacity) {
    size_t i;
    if (capacity < GHOSTOS_VOLUME_BLOCK) return 2;
    if (!kind || kind > 2 || length > GHOSTOS_VOLUME_BLOCK - 16) return 1;
    zero(block);
    block[0] = 'S';
    block[1] = 'Y';
    block[2] = 'N';
    block[3] = 'T';
    block[4] = kind;
    block[5] = 1;
    store_le16(block + 6, (uint16_t)length);
    store_le64(block + 8, checksum(payload, length));
    for (i = 0; i < length; ++i) block[16 + i] = payload[i];
    return 0;
}
int ghostos_volume_decode_tree(const uint8_t *block, size_t capacity, uint8_t *kind, uint8_t *payload, size_t payload_capacity, size_t *length) {
    size_t stored, i;
    if (capacity < GHOSTOS_VOLUME_BLOCK) return 2;
    stored = load_le16(block + 6);
    if (block[0] != 'S' || block[1] != 'Y' || block[2] != 'N' || block[3] != 'T' || block[5] != 1) return 1;
    if (stored > GHOSTOS_VOLUME_BLOCK - 16 || checksum(block + 16, stored) != load_le64(block + 8)) return 1;
    if (block[4] != 1 && block[4] != 2) return 1;
    if (stored > payload_capacity) return 2;
    *kind = block[4];
    *length = stored;
    for (i = 0; i < stored; ++i) payload[i] = block[16 + i];
    return 0;
}
int ghostos_volume_decode_empty(const uint8_t *block, size_t capacity) {
    size_t i;
    if (capacity < GHOSTOS_VOLUME_BLOCK) return 2;
    for (i = 0; i < GHOSTOS_VOLUME_BLOCK; ++i) if (block[i]) return 1;
    return 0;
}
