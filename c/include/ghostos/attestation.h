#ifndef GHOSTOS_ATTESTATION_H
#define GHOSTOS_ATTESTATION_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid, 2 duplicate, 3 capacity, 4 not found,
 * 5 unauthorized, 6 expired, 7 signature mismatch.
 * Hardware root: TPM=1, TrustZone=2, SEV-SNP=3, TDX=4, NVIDIA=5. */
typedef struct {
    uint32_t node;
    uint64_t expires_at_us;
    uint8_t root, measurement[32], key[32], nonce[32];
    bool has_challenge, admitted, occupied;
} ghostos_attestation_node;
int ghostos_attestation_register(ghostos_attestation_node *nodes, size_t count, uint32_t node,
    uint8_t root, const uint8_t measurement[32], const uint8_t key[32], size_t *index);
int ghostos_attestation_rotate_key(ghostos_attestation_node *nodes, size_t count, uint32_t node, const uint8_t key[32]);
int ghostos_attestation_revoke(ghostos_attestation_node *nodes, size_t count, uint32_t node);
int ghostos_attestation_challenge(ghostos_attestation_node *nodes, size_t count, uint32_t node,
    const uint8_t nonce[32], uint64_t expires_at_us);
int ghostos_attestation_quote(uint32_t node, uint8_t root, uint64_t issued_at_us, const uint8_t nonce[32],
    const uint8_t measurement[32], const uint8_t key[32], uint8_t signature[32]);
int ghostos_attestation_admit(ghostos_attestation_node *nodes, size_t count, uint32_t node, uint8_t root,
    uint64_t issued_at_us, const uint8_t nonce[32], const uint8_t measurement[32], const uint8_t signature[32], uint64_t now_us);
bool ghostos_attestation_is_admitted(const ghostos_attestation_node *nodes, size_t count, uint32_t node);
#endif
