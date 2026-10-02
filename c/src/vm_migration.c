#include "ghostos/vm_migration.h"
#include <string.h>

static uint64_t read_le(const uint8_t *bytes, unsigned count) {
    uint64_t value = 0;
    for (unsigned i = 0; i < count; ++i) value |= (uint64_t)bytes[i] << (i * 8);
    return value;
}

static void write_le(uint8_t *bytes, uint64_t value, unsigned count) {
    for (unsigned i = 0; i < count; ++i) bytes[i] = (uint8_t)(value >> (i * 8));
}

uint32_t ghostos_vm_migration_decode(const uint8_t *bytes, size_t length,
    ghostos_vm_migration_frame *frame, uint64_t *declared_length) {
    *declared_length = 0;
    *frame = (ghostos_vm_migration_frame){0};
    if (length < 80) return GHOSTOS_VM_MIGRATION_LENGTH;
    uint64_t payload_length = read_le(bytes + 40, 8);
    *declared_length = payload_length;
    if (!payload_length || payload_length > GHOSTOS_VM_MIGRATION_MAX_BYTES)
        return GHOSTOS_VM_MIGRATION_TOO_LARGE;
    if (payload_length > SIZE_MAX - 80 || length != (size_t)payload_length + 80)
        return GHOSTOS_VM_MIGRATION_LENGTH;
    frame->issued_at = read_le(bytes, 8);
    memcpy(frame->checkpoint_id, bytes + 8, 32);
    frame->payload = bytes + 48;
    frame->payload_length = (size_t)payload_length;
    memcpy(frame->tag, bytes + 48 + frame->payload_length, 32);
    return GHOSTOS_VM_MIGRATION_OK;
}

void ghostos_vm_migration_tag(const ghostos_vm_migration_frame *frame,
    ghostos_vm_migration_schema schema, const uint8_t key_id[16],
    const uint8_t sender_nonce[32], const uint8_t receiver_nonce[32],
    uint64_t declared_length, const ghostos_vm_migration_io *io, uint8_t tag[32]) {
    static const uint8_t domain[] = "SYNOS-MIGRATION-HMAC-SHA256-V3";
    static const uint8_t name[] = "frame";
    uint8_t min_version[4], max_version[4], features[8], issued_at[8], length[8];
    write_le(min_version, schema.min_version, 4);
    write_le(max_version, schema.max_version, 4);
    write_le(features, schema.features, 8);
    write_le(issued_at, frame->issued_at, 8);
    write_le(length, declared_length, 8);
    const ghostos_vm_migration_part parts[] = {
        {domain, sizeof(domain) - 1}, {name, sizeof(name) - 1}, {key_id, 16},
        {sender_nonce, 32}, {receiver_nonce, 32}, {min_version, 4}, {max_version, 4},
        {features, 8}, {issued_at, 8}, {frame->checkpoint_id, 32}, {length, 8},
        {frame->payload, frame->payload_length}
    };
    io->authenticate(io->context, parts, sizeof(parts) / sizeof(parts[0]), tag);
}

uint32_t ghostos_vm_migration_validate(const ghostos_vm_migration_frame *frame,
    ghostos_vm_migration_schema schema, const uint8_t key_id[16],
    const uint8_t sender_nonce[32], const uint8_t receiver_nonce[32],
    const ghostos_vm_migration_io *io) {
    if (!frame->payload_length || frame->payload_length > GHOSTOS_VM_MIGRATION_MAX_BYTES)
        return GHOSTOS_VM_MIGRATION_TOO_LARGE;
    uint8_t expected[32];
    ghostos_vm_migration_tag(frame, schema, key_id, sender_nonce, receiver_nonce,
        frame->payload_length, io, expected);
    /* Volatile accumulation keeps every tag byte in the comparison. */
    volatile uint8_t difference = 0;
    for (unsigned i = 0; i < 32; ++i) difference |= expected[i] ^ frame->tag[i];
    if (difference) return GHOSTOS_VM_MIGRATION_AUTH;
    io->digest(io->context, frame->payload, frame->payload_length, expected);
    if (memcmp(expected, frame->checkpoint_id, 32)) return GHOSTOS_VM_MIGRATION_IDENTITY;
    return io->snapshot(io->context, frame->payload, frame->payload_length);
}
