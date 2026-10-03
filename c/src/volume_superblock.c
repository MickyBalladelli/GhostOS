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
static int checkpoint_records(const uint8_t *block, uint64_t expected_blocks, uint64_t generation, uint64_t next_checkpoint,
    uint32_t count, ghostos_volume_checkpoint *checkpoints, size_t checkpoints_capacity, size_t *written);
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
    if (checkpoint_records(block, expected_blocks, stored_generation, next_checkpoint, checkpoint_count, 0, 0, 0)) return 1;
    *format_version = version;
    *sequence = stored_sequence;
    *generation = stored_generation;
    *root = stored_root;
    return 0;
}
int ghostos_volume_superblock_set_limits(uint8_t *block, size_t capacity,
    uint64_t expected_blocks, uint64_t max_bytes, uint64_t max_files, uint64_t max_blocks) {
    uint16_t version;
    uint64_t sequence, generation;
    uint32_t root;
    if (capacity < GHOSTOS_VOLUME_SUPERBLOCK) return 2;
    if (ghostos_volume_superblock_decode(block, capacity, expected_blocks,
        &version, &sequence, &generation, &root) ||
        (max_blocks != UINT64_MAX && max_blocks > expected_blocks)) return 1;
    store_le64(block + 464, max_bytes);
    store_le64(block + 472, max_files);
    store_le64(block + 480, max_blocks);
    store_le64(block + 4088, checksum(block, 4088));
    return 0;
}
static int checkpoint_records(const uint8_t *block, uint64_t expected_blocks, uint64_t generation, uint64_t next_checkpoint,
    uint32_t count, ghostos_volume_checkpoint *checkpoints, size_t checkpoints_capacity, size_t *written) {
    uint32_t i, j;
    if (count > 16) return 1;
    for (i = 0; i < count; ++i) {
        size_t offset = 80u + (size_t)i * 24u;
        uint64_t id = load_le64(block + offset);
        uint64_t record_generation = load_le64(block + offset + 8);
        uint32_t root = load_le32(block + offset + 16);
        if (!id || id >= next_checkpoint || (root && (uint64_t)root > expected_blocks) || record_generation > generation) return 1;
        for (j = 0; j < i; ++j) if (load_le64(block + 80u + (size_t)j * 24u) == id) return 1;
        if (checkpoints && written && *written < checkpoints_capacity) {
            checkpoints[*written].id = id;
            checkpoints[*written].generation = record_generation;
            checkpoints[*written].root = root;
            *written += 1;
        }
    }
    return 0;
}
int ghostos_volume_superblock_set_checkpoints(uint8_t *block, size_t capacity, uint64_t expected_blocks,
    const ghostos_volume_checkpoint *checkpoints, size_t count) {
    uint16_t version = 0;
    uint64_t sequence = 0, generation = 0, next_checkpoint;
    uint32_t root = 0, i, j;
    if (capacity < GHOSTOS_VOLUME_SUPERBLOCK) return 2;
    if (ghostos_volume_superblock_decode(block, capacity, expected_blocks, &version, &sequence, &generation, &root)) return 1;
    if (count > 16) return 1;
    next_checkpoint = load_le64(block + 48);
    for (i = 0; i < count; ++i) {
        if (!checkpoints[i].id || checkpoints[i].id >= next_checkpoint || checkpoints[i].generation > generation) return 1;
        if (checkpoints[i].root && (uint64_t)checkpoints[i].root > expected_blocks) return 1;
        for (j = 0; j < i; ++j) if (checkpoints[j].id == checkpoints[i].id) return 1;
    }
    for (i = 0; i < 16u * 24u; ++i) block[80 + i] = 0;
    store_le32(block + 56, (uint32_t)count);
    for (i = 0; i < count; ++i) {
        size_t offset = 80u + (size_t)i * 24u;
        store_le64(block + offset, checkpoints[i].id);
        store_le64(block + offset + 8, checkpoints[i].generation);
        store_le32(block + offset + 16, checkpoints[i].root);
    }
    store_le64(block + 4088, checksum(block, 4088));
    return 0;
}
int ghostos_volume_superblock_checkpoints(const uint8_t *block, size_t capacity, uint64_t expected_blocks,
    ghostos_volume_checkpoint *checkpoints, size_t checkpoints_capacity, size_t *count) {
    uint16_t version = 0;
    uint64_t sequence = 0, generation = 0;
    uint32_t root = 0;
    if (ghostos_volume_superblock_decode(block, capacity, expected_blocks, &version, &sequence, &generation, &root)) return 1;
    *count = 0;
    return checkpoint_records(block, expected_blocks, generation, load_le64(block + 48), load_le32(block + 56), checkpoints, checkpoints_capacity, count);
}
