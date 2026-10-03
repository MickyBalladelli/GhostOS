#ifndef GHOSTOS_CONFIG_SIGNATURE_H
#define GHOSTOS_CONFIG_SIGNATURE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Trust: 0 stored, 1 duplicate key, 2 capacity.
 * Verify: 0 accepted, 1 target mismatch, 2 unknown key, 3 invalid signature. */
int ghostos_config_key_id(const uint8_t key[32], uint8_t id[16]);
int ghostos_config_sign(const uint8_t key[32], const uint8_t digest[32], uint64_t revision,
    uint32_t target, bool has_target, uint8_t signature[32]);
int ghostos_config_trust(uint8_t keys[][32], bool *occupied, size_t capacity, const uint8_t key[32], uint8_t id[16]);
int ghostos_config_verify(const uint8_t keys[][32], const bool *occupied, size_t count, const uint8_t signer[16],
    const uint8_t signature[32], const uint8_t digest[32], uint64_t revision, uint32_t target, bool has_target, uint32_t node);
int ghostos_config_verify_cluster(const uint8_t keys[][32], const bool *occupied, size_t count, const uint8_t signer[16],
    const uint8_t signature[32], const uint8_t digest[32], uint64_t revision, uint32_t target, bool has_target,
    const uint32_t *nodes, size_t node_count);
#endif
