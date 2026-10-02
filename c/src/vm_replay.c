#include "ghostos/vm_replay.h"
#include <string.h>

static uint64_t read64(const uint8_t *p) {
    uint64_t value = 0;
    for (size_t i = 0; i < 8; ++i) value |= (uint64_t)p[i] << (i * 8);
    return value;
}
static void write64(uint8_t *p, uint64_t value) {
    for (size_t i = 0; i < 8; ++i) p[i] = (uint8_t)(value >> (i * 8));
}
uint32_t ghostos_vm_replay_file_decode(const uint8_t *bytes, size_t length,
    uint64_t *count) {
    if (!bytes || !count || length < GHOSTOS_VM_REPLAY_FILE_HEADER ||
        length > GHOSTOS_VM_REPLAY_MAX_FILE || memcmp(bytes, "SYNVMRP1", 8) ||
        bytes[8] != 1 || bytes[9] != 0) return GHOSTOS_VM_REPLAY_CORRUPT;
    *count = read64(bytes + 12);
    return *count > GHOSTOS_VM_REPLAY_MAX_EVENTS ?
        GHOSTOS_VM_REPLAY_CAPACITY : GHOSTOS_VM_REPLAY_OK;
}
void ghostos_vm_replay_file_encode(uint8_t *bytes, uint64_t count) {
    if (!bytes) return;
    memset(bytes, 0, GHOSTOS_VM_REPLAY_FILE_HEADER);
    memcpy(bytes, "SYNVMRP1", 8);
    bytes[8] = 1;
    write64(bytes + 12, count);
}
uint32_t ghostos_vm_replay_event_decode(const uint8_t *bytes, size_t length,
    uint64_t sequence, ghostos_vm_replay_event *event) {
    if (!bytes || !event || length < GHOSTOS_VM_REPLAY_EVENT_HEADER ||
        bytes[0] < 1 || bytes[0] > 6 || read64(bytes + 8) != sequence)
        return GHOSTOS_VM_REPLAY_CORRUPT;
    uint64_t payload = read64(bytes + 48);
    if (payload > GHOSTOS_VM_REPLAY_MAX_PAYLOAD)
        return GHOSTOS_VM_REPLAY_PAYLOAD_TOO_LARGE;
    if (payload > length - GHOSTOS_VM_REPLAY_EVENT_HEADER)
        return GHOSTOS_VM_REPLAY_CORRUPT;
    event->kind = bytes[0];
    event->sequence = sequence;
    event->a = read64(bytes + 16);
    event->b = read64(bytes + 24);
    event->c = read64(bytes + 32);
    event->d = read64(bytes + 40);
    event->payload_length = payload;
    return GHOSTOS_VM_REPLAY_OK;
}
uint32_t ghostos_vm_replay_event_encode(uint8_t *bytes,
    const ghostos_vm_replay_event *event) {
    if (!bytes || !event || event->kind < 1 || event->kind > 6)
        return GHOSTOS_VM_REPLAY_CORRUPT;
    if (event->payload_length > GHOSTOS_VM_REPLAY_MAX_PAYLOAD)
        return GHOSTOS_VM_REPLAY_PAYLOAD_TOO_LARGE;
    memset(bytes, 0, GHOSTOS_VM_REPLAY_EVENT_HEADER);
    bytes[0] = event->kind;
    write64(bytes + 8, event->sequence);
    write64(bytes + 16, event->a);
    write64(bytes + 24, event->b);
    write64(bytes + 32, event->c);
    write64(bytes + 40, event->d);
    write64(bytes + 48, event->payload_length);
    return GHOSTOS_VM_REPLAY_OK;
}
