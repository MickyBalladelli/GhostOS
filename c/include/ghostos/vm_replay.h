#ifndef GHOSTOS_VM_REPLAY_H
#define GHOSTOS_VM_REPLAY_H
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_VM_REPLAY_FILE_HEADER 20
#define GHOSTOS_VM_REPLAY_EVENT_HEADER 56
#define GHOSTOS_VM_REPLAY_MAX_FILE UINT64_C(134217728)
#define GHOSTOS_VM_REPLAY_MAX_PAYLOAD UINT64_C(4194304)
#define GHOSTOS_VM_REPLAY_MAX_EVENTS UINT64_C(1000000)

enum ghostos_vm_replay_result {
    GHOSTOS_VM_REPLAY_OK, GHOSTOS_VM_REPLAY_CORRUPT,
    GHOSTOS_VM_REPLAY_CAPACITY, GHOSTOS_VM_REPLAY_PAYLOAD_TOO_LARGE
};
typedef struct {
    uint64_t sequence, a, b, c, d, payload_length;
    uint8_t kind;
} ghostos_vm_replay_event;
/* Reserved bytes are ignored on decode, matching SYNVMRP1. */
uint32_t ghostos_vm_replay_file_decode(const uint8_t *, size_t, uint64_t *);
void ghostos_vm_replay_file_encode(uint8_t[GHOSTOS_VM_REPLAY_FILE_HEADER], uint64_t);
uint32_t ghostos_vm_replay_event_decode(const uint8_t *, size_t, uint64_t,
    ghostos_vm_replay_event *);
uint32_t ghostos_vm_replay_event_encode(uint8_t[GHOSTOS_VM_REPLAY_EVENT_HEADER],
    const ghostos_vm_replay_event *);
#endif
