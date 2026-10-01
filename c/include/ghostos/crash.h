#ifndef GHOSTOS_CRASH_H
#define GHOSTOS_CRASH_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdatomic.h>

#define GHOSTOS_CRASH_CAPSULE_VERSION 1u
#define GHOSTOS_CRASH_MAX_CAPABILITY_RECORDS 8u
#define GHOSTOS_CRASH_MAX_AUDIT_IDS 8u
#define GHOSTOS_CRASH_MAX_CAPSULE_BYTES 1024u

typedef struct {
    uint64_t general[16];
    uint64_t instruction_pointer;
    uint64_t stack_pointer;
    uint64_t flags;
    uint64_t fault_address;
} ghostos_crash_register_state;

typedef struct {
    uint64_t redacted_handle;
    uint32_t redacted_owner;
    uint16_t rights;
    uint8_t object_kind;
} ghostos_crash_capability_record;

typedef struct {
    uint16_t active;
    uint16_t capacity;
    ghostos_crash_capability_record records[GHOSTOS_CRASH_MAX_CAPABILITY_RECORDS];
    uint8_t record_count;
} ghostos_crash_capability_context;

typedef struct {
    uint64_t clock;
    uint32_t current_thread;
    uint8_t current_cpu;
    uint16_t state_counts[5];
    uint64_t current_instruction_pointer;
    uint64_t current_stack_pointer;
} ghostos_crash_scheduler_state;

typedef struct {
    uint64_t package;
    uint64_t version;
    uint64_t target;
} ghostos_crash_build_identity;

typedef struct {
    uint16_t reason;
    uint64_t status;
    ghostos_crash_build_identity build;
    ghostos_crash_register_state registers;
    ghostos_crash_capability_context capabilities;
    ghostos_crash_scheduler_state scheduler;
    uint64_t audit_ids[GHOSTOS_CRASH_MAX_AUDIT_IDS][2];
    uint8_t audit_count;
} ghostos_crash_capsule;

typedef bool (*ghostos_crash_persist_fn)(void *context, const uint8_t *bytes,
                                         size_t length);

void ghostos_crash_context_init(ghostos_crash_capability_context *context);
ghostos_crash_build_identity ghostos_crash_build_identity_for(
    const char *package, const char *version, const char *target);
uint64_t ghostos_crash_redact_u64(uint64_t value);
uint32_t ghostos_crash_redact_u32(uint32_t value);
bool ghostos_crash_capsule_encode(const ghostos_crash_capsule *capsule,
                                  uint8_t *destination, size_t capacity,
                                  size_t *length);
bool ghostos_crash_capture_and_persist(
    atomic_bool *in_progress, ghostos_crash_capsule *capsule,
    uint64_t fault_address, const uint64_t audit_ids[][2],
    size_t audit_count, ghostos_crash_persist_fn persist, void *context);
bool ghostos_crash_capture_and_persist_once(
    ghostos_crash_capsule *capsule, ghostos_crash_persist_fn persist, void *context);
bool ghostos_crash_claim(void);

#endif
