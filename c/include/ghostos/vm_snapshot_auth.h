#ifndef GHOSTOS_VM_SNAPSHOT_AUTH_H
#define GHOSTOS_VM_SNAPSHOT_AUTH_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
typedef struct { const uint8_t *bytes; size_t length; } ghostos_vm_snapshot_auth_part;
void ghostos_vm_snapshot_sha256(const uint8_t *bytes, size_t length, uint8_t digest[32]);
void ghostos_vm_snapshot_hmac_sha256(const uint8_t key[32],
    const ghostos_vm_snapshot_auth_part *parts, size_t count, uint8_t tag[32]);
bool ghostos_vm_snapshot_auth_equal(const uint8_t *left, const uint8_t *right, size_t length);
#endif
