#ifndef GHOSTOS_VM_MIGRATION_H
#define GHOSTOS_VM_MIGRATION_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_VM_MIGRATION_MAX_BYTES UINT64_C(8589934592)

typedef struct { const uint8_t *bytes; size_t length; } ghostos_vm_migration_part;
typedef struct { uint32_t min_version, max_version; uint64_t features; } ghostos_vm_migration_schema;
typedef struct {
    uint64_t issued_at;
    uint8_t checkpoint_id[32];
    const uint8_t *payload;
    size_t payload_length;
    uint8_t tag[32];
} ghostos_vm_migration_frame;
enum ghostos_vm_migration_result {
    GHOSTOS_VM_MIGRATION_OK,
    GHOSTOS_VM_MIGRATION_LENGTH,
    GHOSTOS_VM_MIGRATION_TOO_LARGE,
    GHOSTOS_VM_MIGRATION_AUTH,
    GHOSTOS_VM_MIGRATION_IDENTITY,
    GHOSTOS_VM_MIGRATION_SNAPSHOT,
    GHOSTOS_VM_MIGRATION_SCHEMA
};
typedef struct {
    void (*authenticate)(void *, const ghostos_vm_migration_part *, size_t, uint8_t[32]);
    void (*digest)(void *, const uint8_t *, size_t, uint8_t[32]);
    /* Return OK, SNAPSHOT, or SCHEMA. Store host snapshot errors in context. */
    uint32_t (*snapshot)(void *, const uint8_t *, size_t);
    void *context;
} ghostos_vm_migration_io;

/* Views borrow input. Decode performs no allocations and never exposes payload
 * bytes in diagnostics. On TOO_LARGE, declared_length holds the wire length. */
uint32_t ghostos_vm_migration_decode(const uint8_t *bytes, size_t length,
    ghostos_vm_migration_frame *frame, uint64_t *declared_length);
void ghostos_vm_migration_tag(const ghostos_vm_migration_frame *frame,
    ghostos_vm_migration_schema schema, const uint8_t key_id[16],
    const uint8_t sender_nonce[32], const uint8_t receiver_nonce[32],
    uint64_t declared_length, const ghostos_vm_migration_io *io, uint8_t tag[32]);
uint32_t ghostos_vm_migration_validate(const ghostos_vm_migration_frame *frame,
    ghostos_vm_migration_schema schema, const uint8_t key_id[16],
    const uint8_t sender_nonce[32], const uint8_t receiver_nonce[32],
    const ghostos_vm_migration_io *io);

#endif
