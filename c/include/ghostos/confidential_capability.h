#ifndef GHOSTOS_CONFIDENTIAL_CAPABILITY_H
#define GHOSTOS_CONFIDENTIAL_CAPABILITY_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid, 2 capacity, 3 unauthorized, 4 expired,
 * 5 authentication failed, 6 not found.
 * Resource kind: shared DSM=1, IPC channel=2, shared region=3.
 * Rights: read=1, write=2, send=4, receive=8. */
typedef struct {
    uint64_t issuer, epoch, next_id;
} ghostos_confidential_authority;
typedef struct {
    uint64_t issuer, epoch, id, subject, expires_at_us, resource_a, resource_b;
    uint32_t node;
    uint8_t kind, rights;
    uint8_t attestation[32], token[32];
    bool occupied;
} ghostos_confidential_capability;
int ghostos_confidential_authority_init(ghostos_confidential_authority *authority, uint64_t issuer);
int ghostos_confidential_issue(ghostos_confidential_authority *authority,
    ghostos_confidential_capability *records, size_t count, uint32_t node, uint64_t subject,
    uint8_t kind, uint64_t resource_a, uint64_t resource_b, uint8_t rights, uint64_t expires_at_us,
    uint64_t now_us, const uint8_t attestation[32], size_t *index);
int ghostos_confidential_validate(const ghostos_confidential_authority *authority,
    const ghostos_confidential_capability *records, size_t count,
    const ghostos_confidential_capability *capability, uint64_t subject, uint8_t required, uint64_t now_us);
int ghostos_confidential_revoke(ghostos_confidential_capability *records, size_t count,
    const ghostos_confidential_capability *capability);
uint64_t ghostos_confidential_revoke_all(ghostos_confidential_authority *authority,
    ghostos_confidential_capability *records, size_t count);
#endif
