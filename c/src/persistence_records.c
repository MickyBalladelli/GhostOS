#include "ghostos/persistence.h"
#include "ghostos/memory.h"
#include "ghostos/boot_diagnostics.h"

_Static_assert(GHOSTOS_PERSISTENCE_BOOT_BYTES == GHOSTOS_BOOT_DIAGNOSTIC_BYTES, "boot record bound");
_Static_assert(sizeof(ghostos_persistent_records) == 1096, "persistent record ABI");
_Static_assert(offsetof(ghostos_persistent_records, crash) == 64, "crash record ABI");
_Static_assert(offsetof(ghostos_persistent_records, crash_length) == 1088, "crash length ABI");

static const uint8_t container_magic[8] = {'S','Y','N','R','E','C','0','1'};
static const uint8_t crash_magic[8] = {'S','Y','N','C','R','S','H','1'};
static const uint8_t boot_magic[8] = {'S','Y','N','B','T','D','0','1'};

static uint16_t read16(const uint8_t *p) {
    return (uint16_t)p[0] | (uint16_t)p[1] << 8;
}
static uint32_t read32(const uint8_t *p) {
    return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}
static void write16(uint8_t *p, uint16_t value) {
    p[0] = (uint8_t)value; p[1] = (uint8_t)(value >> 8);
}
static void write32(uint8_t *p, uint32_t value) {
    for (size_t i = 0; i < 4; ++i) p[i] = (uint8_t)(value >> (i * 8));
}
static uint32_t checksum(const uint8_t *bytes, size_t length) {
    uint32_t value = UINT32_C(0x811c9dc5);
    for (size_t i = 0; i < length; ++i) value = (value ^ bytes[i]) * UINT32_C(16777619);
    return value;
}

void ghostos_persistence_records_decode(const uint8_t *bytes, size_t length,
    ghostos_persistent_records *records) {
    ghostos_memory_zero(records, sizeof(*records));
    if (length >= GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES &&
        ghostos_memory_equal(bytes, container_magic, 8) && read16(bytes + 8) == 1 &&
        read16(bytes + 10) == GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES) {
        size_t boot = read32(bytes + 12), crash = read32(bytes + 16);
        if (boot <= GHOSTOS_PERSISTENCE_BOOT_BYTES && crash <= GHOSTOS_PERSISTENCE_CRASH_BYTES &&
            GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES + boot + crash == length &&
            read32(bytes + 20) == checksum(bytes + GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES, boot + crash)) {
            records->boot_length = boot; records->crash_length = crash;
            ghostos_memory_copy(records->boot, bytes + GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES, boot);
            ghostos_memory_copy(records->crash, bytes + GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES + boot, crash);
            return;
        }
    }
    if (length < 8) return;
    if (ghostos_memory_equal(bytes, crash_magic, 8)) {
        records->crash_length = length < GHOSTOS_PERSISTENCE_CRASH_BYTES ? length : GHOSTOS_PERSISTENCE_CRASH_BYTES;
        ghostos_memory_copy(records->crash, bytes, records->crash_length);
    } else if (ghostos_memory_equal(bytes, boot_magic, 8)) {
        records->boot_length = length < GHOSTOS_PERSISTENCE_BOOT_BYTES ? length : GHOSTOS_PERSISTENCE_BOOT_BYTES;
        ghostos_memory_copy(records->boot, bytes, records->boot_length);
    }
}

bool ghostos_persistence_records_encode(const ghostos_persistent_records *records,
    uint8_t *bytes, size_t capacity, size_t *length) {
    if (records->boot_length > GHOSTOS_PERSISTENCE_BOOT_BYTES ||
        records->crash_length > GHOSTOS_PERSISTENCE_CRASH_BYTES) return false;
    size_t total = GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES + records->boot_length + records->crash_length;
    if (total > capacity) return false;
    ghostos_memory_zero(bytes, total);
    ghostos_memory_copy(bytes, container_magic, 8);
    write16(bytes + 8, 1); write16(bytes + 10, GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES);
    write32(bytes + 12, (uint32_t)records->boot_length);
    write32(bytes + 16, (uint32_t)records->crash_length);
    ghostos_memory_copy(bytes + GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES, records->boot, records->boot_length);
    ghostos_memory_copy(bytes + GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES + records->boot_length,
        records->crash, records->crash_length);
    write32(bytes + 20, checksum(bytes + GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES,
        records->boot_length + records->crash_length));
    *length = total;
    return true;
}

bool ghostos_persistence_records_update(ghostos_persistent_records *records,
    bool crash, const uint8_t *bytes, size_t length) {
    size_t capacity = crash ? GHOSTOS_PERSISTENCE_CRASH_BYTES : GHOSTOS_PERSISTENCE_BOOT_BYTES;
    if (length > capacity) return false;
    uint8_t *target = crash ? records->crash : records->boot;
    ghostos_memory_copy(target, bytes, length);
    ghostos_memory_zero(target + length, capacity - length);
    if (crash) records->crash_length = length;
    else records->boot_length = length;
    return true;
}

bool ghostos_persistence_records_boot(const ghostos_persistent_records *records,
    uint8_t *bytes, size_t capacity, size_t *length) {
    if (!records->boot_length || records->boot_length > capacity) return false;
    ghostos_memory_copy(bytes, records->boot, records->boot_length);
    *length = records->boot_length;
    return true;
}
