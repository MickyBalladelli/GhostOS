#include "ghostos/config_signature.h"
#include "ghostos/vm_snapshot_auth.h"
static void hmac(const uint8_t key[32], const uint8_t *message, size_t length, uint8_t tag[32]) {
    ghostos_vm_snapshot_auth_part part = {message, length};
    ghostos_vm_snapshot_hmac_sha256(key, &part, 1, tag);
}
static bool same16(const uint8_t *left, const uint8_t *right) {
    size_t i;
    for (i = 0; i < 16; ++i) if (left[i] != right[i]) return false;
    return true;
}
static void material(const uint8_t digest[32], uint64_t revision, uint32_t target, bool has_target, uint8_t out[44]) {
    size_t i;
    for (i = 0; i < 32; ++i) out[i] = digest[i];
    for (i = 0; i < 8; ++i) out[32 + i] = (uint8_t)(revision >> (56 - 8 * i));
    out[40] = has_target ? (uint8_t)(target >> 24) : 0;
    out[41] = has_target ? (uint8_t)(target >> 16) : 0;
    out[42] = has_target ? (uint8_t)(target >> 8) : 0;
    out[43] = has_target ? (uint8_t)target : 0;
}
int ghostos_config_key_id(const uint8_t key[32], uint8_t id[16]) {
    uint8_t digest[32];
    size_t i;
    ghostos_vm_snapshot_sha256(key, 32, digest);
    for (i = 0; i < 16; ++i) id[i] = digest[i];
    return 0;
}
int ghostos_config_sign(const uint8_t key[32], const uint8_t digest[32], uint64_t revision,
    uint32_t target, bool has_target, uint8_t signature[32]) {
    uint8_t bytes[44];
    material(digest, revision, target, has_target, bytes);
    hmac(key, bytes, sizeof bytes, signature);
    return 0;
}
int ghostos_config_trust(uint8_t keys[][32], bool *occupied, size_t capacity, const uint8_t key[32], uint8_t id[16]) {
    uint8_t candidate[16];
    size_t free_slot = capacity;
    size_t i, j;
    ghostos_config_key_id(key, candidate);
    for (i = 0; i < capacity; ++i) {
        uint8_t existing[16];
        if (!occupied[i]) {
            if (free_slot == capacity) free_slot = i;
            continue;
        }
        ghostos_config_key_id(keys[i], existing);
        if (same16(existing, candidate)) return 1;
    }
    if (free_slot == capacity) return 2;
    for (j = 0; j < 32; ++j) keys[free_slot][j] = key[j];
    occupied[free_slot] = true;
    for (j = 0; j < 16; ++j) id[j] = candidate[j];
    return 0;
}
int ghostos_config_verify(const uint8_t keys[][32], const bool *occupied, size_t count, const uint8_t signer[16],
    const uint8_t signature[32], const uint8_t digest[32], uint64_t revision, uint32_t target, bool has_target, uint32_t node) {
    uint8_t expected[32];
    size_t i;
    if (has_target && target != node) return 1;
    for (i = 0; i < count; ++i) {
        uint8_t id[16];
        if (!occupied[i]) continue;
        ghostos_config_key_id(keys[i], id);
        if (!same16(id, signer)) continue;
        ghostos_config_sign(keys[i], digest, revision, target, has_target, expected);
        return ghostos_vm_snapshot_auth_equal(expected, signature, 32) ? 0 : 3;
    }
    return 2;
}
int ghostos_config_verify_cluster(const uint8_t keys[][32], const bool *occupied, size_t count, const uint8_t signer[16],
    const uint8_t signature[32], const uint8_t digest[32], uint64_t revision, uint32_t target, bool has_target,
    const uint32_t *nodes, size_t node_count) {
    size_t i;
    if (!node_count) return 1;
    for (i = 0; i < node_count; ++i) {
        int status = ghostos_config_verify(keys, occupied, count, signer, signature, digest, revision, target, has_target, nodes[i]);
        if (status) return status;
    }
    return 0;
}
