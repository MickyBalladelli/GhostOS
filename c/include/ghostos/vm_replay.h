#ifndef GHOSTOS_VM_REPLAY_H
#define GHOSTOS_VM_REPLAY_H
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#define GHOSTOS_VM_REPLAY_FILE_HEADER 20
#define GHOSTOS_VM_REPLAY_EVENT_HEADER 56
#define GHOSTOS_VM_REPLAY_MAX_FILE UINT64_C(134217728)
#define GHOSTOS_VM_REPLAY_MAX_PAYLOAD UINT64_C(4194304)
#define GHOSTOS_VM_REPLAY_MAX_EVENTS UINT64_C(1000000)

enum ghostos_vm_replay_result {
    GHOSTOS_VM_REPLAY_OK, GHOSTOS_VM_REPLAY_CORRUPT,
    GHOSTOS_VM_REPLAY_CAPACITY, GHOSTOS_VM_REPLAY_PAYLOAD_TOO_LARGE,
    GHOSTOS_VM_REPLAY_WRONG_MODE, GHOSTOS_VM_REPLAY_END,
    GHOSTOS_VM_REPLAY_UNEXPECTED_KIND, GHOSTOS_VM_REPLAY_INPUT_MISMATCH
};
enum ghostos_vm_replay_mode {
    GHOSTOS_VM_REPLAY_DISABLED, GHOSTOS_VM_REPLAY_RECORDING,
    GHOSTOS_VM_REPLAY_REPLAYING
};
typedef struct {
    uint64_t sequence, a, b, c, d, payload_length;
    uint8_t kind;
} ghostos_vm_replay_event;
typedef struct {
    ghostos_vm_replay_event event;
    const uint8_t *payload;
} ghostos_vm_replay_view;
typedef struct {
    uint64_t sequence;
    uint32_t code, mode;
    uint8_t expected, actual, kind;
} ghostos_vm_replay_error;
typedef struct {
    uint64_t address;
    const uint8_t *bytes;
    size_t length;
} ghostos_vm_replay_dma_write;
typedef struct ghostos_vm_replay_session ghostos_vm_replay_session;
typedef void (*ghostos_vm_replay_dma_sink)(void *, uint64_t, const uint8_t *, size_t);
/* Reserved bytes are ignored on decode, matching SYNVMRP1. */
uint32_t ghostos_vm_replay_file_decode(const uint8_t *, size_t, uint64_t *);
void ghostos_vm_replay_file_encode(uint8_t[GHOSTOS_VM_REPLAY_FILE_HEADER], uint64_t);
uint32_t ghostos_vm_replay_event_decode(const uint8_t *, size_t, uint64_t,
    ghostos_vm_replay_event *);
uint32_t ghostos_vm_replay_event_encode(uint8_t[GHOSTOS_VM_REPLAY_EVENT_HEADER],
    const ghostos_vm_replay_event *);
uint32_t ghostos_vm_replay_validate(const ghostos_vm_replay_view *, size_t);
uint32_t ghostos_vm_replay_file_size(const ghostos_vm_replay_view *, size_t, uint64_t *);
uint32_t ghostos_vm_replay_dma_encode(const ghostos_vm_replay_dma_write *, size_t,
    uint8_t **, size_t *, uint64_t *);
uint32_t ghostos_vm_replay_dma_decode(const uint8_t *, size_t,
    ghostos_vm_replay_dma_sink, void *);
void ghostos_vm_replay_bytes_free(uint8_t *);

ghostos_vm_replay_session *ghostos_vm_replay_session_new(size_t);
void ghostos_vm_replay_session_free(ghostos_vm_replay_session *);
uint32_t ghostos_vm_replay_mode_get(const ghostos_vm_replay_session *);
size_t ghostos_vm_replay_position(const ghostos_vm_replay_session *);
size_t ghostos_vm_replay_pending(const ghostos_vm_replay_session *);
size_t ghostos_vm_replay_length(const ghostos_vm_replay_session *);
void ghostos_vm_replay_begin_recording(ghostos_vm_replay_session *);
uint32_t ghostos_vm_replay_begin(ghostos_vm_replay_session *,
    const ghostos_vm_replay_view *, size_t);
void ghostos_vm_replay_stop(ghostos_vm_replay_session *);
bool ghostos_vm_replay_take_error(ghostos_vm_replay_session *, ghostos_vm_replay_error *);
void ghostos_vm_replay_error_get(const ghostos_vm_replay_session *, ghostos_vm_replay_error *);
uint8_t ghostos_vm_replay_peek_kind(const ghostos_vm_replay_session *);
/* Borrowed views survive next/stop, but not begin/begin_recording/free. */
bool ghostos_vm_replay_event_get(const ghostos_vm_replay_session *, size_t,
    ghostos_vm_replay_view *);
uint32_t ghostos_vm_replay_next(ghostos_vm_replay_session *, uint8_t,
    ghostos_vm_replay_view *);
uint32_t ghostos_vm_replay_instruction_input(ghostos_vm_replay_session *,
    uint64_t, uint64_t, uint8_t, uint64_t, uint64_t *);
uint32_t ghostos_vm_replay_clock(ghostos_vm_replay_session *, uint64_t, uint64_t *);
uint32_t ghostos_vm_replay_timer(ghostos_vm_replay_session *, uint64_t, uint64_t);
uint32_t ghostos_vm_replay_interrupt(ghostos_vm_replay_session *, uint8_t);
uint32_t ghostos_vm_replay_host_input(ghostos_vm_replay_session *, uint64_t,
    bool, uint16_t, bool, uint16_t, const uint8_t *, size_t);
uint32_t ghostos_vm_replay_next_host_input(ghostos_vm_replay_session *,
    bool *, ghostos_vm_replay_view *, bool *, uint16_t *, uint16_t *);
uint32_t ghostos_vm_replay_device_completion(ghostos_vm_replay_session *,
    const ghostos_vm_replay_dma_write *, size_t, ghostos_vm_replay_dma_sink, void *);
#endif
