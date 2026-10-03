#include "ghostos/volume_superblock.h"
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
int ghostos_volume_superblock_encode(uint16_t format_version, uint64_t sequence, uint64_t generation, uint32_t root,
    uint64_t next_checkpoint, uint64_t next_object_id, uint64_t type_map_checksum, uint64_t max_blocks, uint8_t *block,
    size_t capacity) {
    size_t i;
    static const uint8_t magic[] = {'S', 'Y', 'N', 'F', 'S', 'V', 'O', 'L'};
    if (capacity < GHOSTOS_VOLUME_SUPERBLOCK) return 2;
    for (i = 0; i < GHOSTOS_VOLUME_SUPERBLOCK; ++i) block[i] = 0;
    for (i = 0; i < 8; ++i) block[i] = magic[i];
    store_le16(block + 8, format_version);
    store_le16(block + 10, 512);
    store_le32(block + 12, GHOSTOS_VOLUME_SUPERBLOCK);
    store_le64(block + 16, max_blocks);
    store_le64(block + 24, sequence);
    store_le64(block + 32, generation);
    store_le32(block + 40, root);
    store_le64(block + 48, next_checkpoint);
    store_le32(block + 56, 0);
    store_le64(block + 64, type_map_checksum);
    store_le64(block + 72, next_object_id);
    store_le64(block + 480, UINT64_MAX);
    store_le64(block + 4088, checksum(block, 4088));
    return 0;
}
int ghostos_volume_superblock_decode(const uint8_t *block, size_t capacity, uint64_t expected_blocks, uint16_t *format_version,
    uint64_t *sequence, uint64_t *generation, uint32_t *root) {
    uint16_t version;
    uint64_t stored_sequence, stored_generation, next_checkpoint, next_object, limit;
    uint32_t checkpoint_count, stored_root;
    size_t i;
    static const uint8_t magic[] = {'S', 'Y', 'N', 'F', 'S', 'V', 'O', 'L'};
    if (capacity < GHOSTOS_VOLUME_SUPERBLOCK) return 2;
    for (i = 0; i < 8; ++i) if (block[i] != magic[i]) return 1;
    version = load_le16(block + 8);
    if ((version != 3 && version != 4) || load_le16(block + 10) != 512 || load_le32(block + 12) != GHOSTOS_VOLUME_SUPERBLOCK ||
        load_le64(block + 16) != expected_blocks || load_le64(block + 4088) != checksum(block, 4088))
        return 1;
    stored_sequence = load_le64(block + 24);
    stored_generation = load_le64(block + 32);
    stored_root = load_le32(block + 40);
    next_checkpoint = load_le64(block + 48);
    checkpoint_count = load_le32(block + 56);
    next_object = load_le64(block + 72);
    limit = load_le64(block + 480);
    if (!stored_sequence || checkpoint_count > 16 || !next_checkpoint || !next_object || (limit != UINT64_MAX && limit > expected_blocks))
        return 1;
    if (stored_root && (uint64_t)stored_root > expected_blocks) return 1;
    *format_version = version;
    *sequence = stored_sequence;
    *generation = stored_generation;
    *root = stored_root;
    return 0;
}
