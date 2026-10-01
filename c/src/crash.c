#include "ghostos/crash.h"

static atomic_bool crash_in_progress = ATOMIC_VAR_INIT(false);

static const uint8_t capsule_magic[8] = {'S', 'Y', 'N', 'C', 'R', 'S', 'H', '1'};

typedef struct {
    uint8_t *bytes;
    size_t capacity;
    size_t position;
} writer;

static bool write_bytes(writer *out, const uint8_t *bytes, size_t count)
{
    if (count > out->capacity - out->position) return false;
    for (size_t i = 0; i < count; ++i) out->bytes[out->position + i] = bytes[i];
    out->position += count;
    return true;
}

static bool write_u8(writer *out, uint8_t value)
{
    return write_bytes(out, &value, 1);
}

static bool write_u16(writer *out, uint16_t value)
{
    uint8_t bytes[2] = {(uint8_t)value, (uint8_t)(value >> 8)};
    return write_bytes(out, bytes, sizeof(bytes));
}

static bool write_u32(writer *out, uint32_t value)
{
    uint8_t bytes[4];
    for (size_t i = 0; i < sizeof(bytes); ++i) bytes[i] = (uint8_t)(value >> (i * 8));
    return write_bytes(out, bytes, sizeof(bytes));
}

static bool write_u64(writer *out, uint64_t value)
{
    uint8_t bytes[8];
    for (size_t i = 0; i < sizeof(bytes); ++i) bytes[i] = (uint8_t)(value >> (i * 8));
    return write_bytes(out, bytes, sizeof(bytes));
}

static uint64_t fnv1a(const char *text)
{
    uint64_t hash = UINT64_C(0xcbf29ce484222325);
    for (; *text; ++text) {
        hash ^= (uint8_t)*text;
        hash *= UINT64_C(0x100000001b3);
    }
    return hash;
}

void ghostos_crash_context_init(ghostos_crash_capability_context *context)
{
    if (!context) return;
    uint8_t *bytes = (uint8_t *)context;
    for (size_t i = 0; i < sizeof(*context); ++i) bytes[i] = 0;
}

uint64_t ghostos_crash_redact_u64(uint64_t value)
{
    uint64_t hash = UINT64_C(0xd6e8feb86659fd93);
    for (unsigned shift = 0; shift < 64; shift += 8) {
        hash ^= (value >> shift) & UINT64_C(0xff);
        hash *= UINT64_C(0x100000001b3);
    }
    return hash;
}

uint32_t ghostos_crash_redact_u32(uint32_t value)
{
    uint64_t hash = ghostos_crash_redact_u64(value);
    return (uint32_t)(hash ^ (hash >> 32));
}

bool ghostos_crash_capsule_encode(const ghostos_crash_capsule *capsule,
                                  uint8_t *destination, size_t capacity,
                                  size_t *length)
{
    if (!capsule || !destination || !length || capacity > GHOSTOS_CRASH_MAX_CAPSULE_BYTES)
        return false;
    writer out = {destination, capacity, 0};
    if (!write_bytes(&out, capsule_magic, sizeof(capsule_magic)) ||
        !write_u16(&out, GHOSTOS_CRASH_CAPSULE_VERSION) || !write_u16(&out, 0) ||
        !write_u16(&out, capsule->reason) || !write_u16(&out, 0) ||
        !write_u64(&out, capsule->status) ||
        !write_u64(&out, capsule->build.package) ||
        !write_u64(&out, capsule->build.version) ||
        !write_u64(&out, capsule->build.target) ||
        !write_u64(&out, capsule->registers.fault_address) ||
        !write_u64(&out, capsule->registers.instruction_pointer) ||
        !write_u64(&out, capsule->registers.stack_pointer) ||
        !write_u64(&out, capsule->registers.flags)) return false;
    for (size_t i = 0; i < 16; ++i)
        if (!write_u64(&out, capsule->registers.general[i])) return false;
    uint8_t record_count = capsule->capabilities.record_count;
    if (record_count > GHOSTOS_CRASH_MAX_CAPABILITY_RECORDS) return false;
    if (!write_u16(&out, capsule->capabilities.active) ||
        !write_u16(&out, capsule->capabilities.capacity) ||
        !write_u8(&out, record_count) || !write_u8(&out, 0)) return false;
    for (size_t i = 0; i < GHOSTOS_CRASH_MAX_CAPABILITY_RECORDS; ++i) {
        const ghostos_crash_capability_record *record = &capsule->capabilities.records[i];
        if (!write_u64(&out, record->redacted_handle) ||
            !write_u32(&out, record->redacted_owner) || !write_u16(&out, record->rights) ||
            !write_u8(&out, record->object_kind) || !write_u8(&out, 0)) return false;
    }
    if (!write_u64(&out, capsule->scheduler.clock) ||
        !write_u32(&out, capsule->scheduler.current_thread) ||
        !write_u8(&out, capsule->scheduler.current_cpu)) return false;
    for (size_t i = 0; i < 5; ++i)
        if (!write_u16(&out, capsule->scheduler.state_counts[i])) return false;
    if (!write_u64(&out, capsule->scheduler.current_instruction_pointer) ||
        !write_u64(&out, capsule->scheduler.current_stack_pointer) ||
        !write_u8(&out, capsule->audit_count > GHOSTOS_CRASH_MAX_AUDIT_IDS
                         ? GHOSTOS_CRASH_MAX_AUDIT_IDS : capsule->audit_count)) return false;
    const uint8_t padding[7] = {0};
    if (!write_bytes(&out, padding, sizeof(padding))) return false;
    for (size_t i = 0; i < GHOSTOS_CRASH_MAX_AUDIT_IDS; ++i)
        for (size_t word = 0; word < 2; ++word)
            if (!write_u64(&out, capsule->audit_ids[i][word])) return false;
    destination[10] = (uint8_t)out.position;
    destination[11] = (uint8_t)(out.position >> 8);
    *length = out.position;
    return true;
}

bool ghostos_crash_capture_and_persist(
    atomic_bool *in_progress, ghostos_crash_capsule *capsule,
    uint64_t fault_address, const uint64_t audit_ids[][2],
    size_t audit_count, ghostos_crash_persist_fn persist, void *context)
{
    if (!in_progress || !capsule || !persist) return false;
    bool expected = false;
    if (!atomic_compare_exchange_strong_explicit(in_progress, &expected, true,
                                                  memory_order_acq_rel,
                                                  memory_order_relaxed)) return false;
    capsule->registers.fault_address = fault_address;
    if (audit_count > GHOSTOS_CRASH_MAX_AUDIT_IDS) audit_count = GHOSTOS_CRASH_MAX_AUDIT_IDS;
    capsule->audit_count = (uint8_t)audit_count;
    for (size_t i = 0; i < GHOSTOS_CRASH_MAX_AUDIT_IDS; ++i) {
        capsule->audit_ids[i][0] = 0;
        capsule->audit_ids[i][1] = 0;
    }
    if (audit_ids)
        for (size_t i = 0; i < audit_count; ++i) {
            capsule->audit_ids[i][0] = audit_ids[i][0];
            capsule->audit_ids[i][1] = audit_ids[i][1];
        }
    uint8_t bytes[GHOSTOS_CRASH_MAX_CAPSULE_BYTES];
    size_t length = 0;
    return ghostos_crash_capsule_encode(capsule, bytes, sizeof(bytes), &length) &&
           persist(context, bytes, length);
}

bool ghostos_crash_capture_and_persist_once(
    ghostos_crash_capsule *capsule, ghostos_crash_persist_fn persist, void *context)
{
    if (!capsule || !persist) return false;
    bool expected = false;
    if (!atomic_compare_exchange_strong_explicit(&crash_in_progress, &expected, true,
                                                  memory_order_acq_rel,
                                                  memory_order_relaxed)) return false;
    uint8_t bytes[GHOSTOS_CRASH_MAX_CAPSULE_BYTES];
    size_t length = 0;
    return ghostos_crash_capsule_encode(capsule, bytes, sizeof(bytes), &length) &&
           persist(context, bytes, length);
}

bool ghostos_crash_claim(void)
{
    bool expected = false;
    return atomic_compare_exchange_strong_explicit(&crash_in_progress, &expected, true,
                                                   memory_order_acq_rel,
                                                   memory_order_relaxed);
}

ghostos_crash_build_identity ghostos_crash_build_identity_for(const char *package,
                                                               const char *version,
                                                               const char *target)
{
    return (ghostos_crash_build_identity){fnv1a(package), fnv1a(version), fnv1a(target)};
}
