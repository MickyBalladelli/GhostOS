#ifndef GHOSTOS_VM_TERMINAL_H
#define GHOSTOS_VM_TERMINAL_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_VM_TERMINAL_SIZE_POLL_NS UINT64_C(250000000)
typedef struct ghostos_vm_terminal ghostos_vm_terminal;
typedef struct {
    uint64_t polls, input_bytes, output_bytes, output_flushes, eof_events, resize_events;
} ghostos_vm_terminal_counters;
typedef struct {
    const uint8_t *bytes;
    size_t length;
    uint16_t rows, columns;
    bool resize;
} ghostos_vm_terminal_input;
typedef struct {
    const uint8_t *raw;
    size_t raw_length;
    const uint8_t *bytes;
    size_t length;
    uint16_t rows, columns;
    uint8_t kind; /* 1 = input, 2 = EOF, 3 = resize */
} ghostos_vm_terminal_event;

ghostos_vm_terminal *ghostos_vm_terminal_new(void);
void ghostos_vm_terminal_free(ghostos_vm_terminal *);
/* Begin clears the prior poll result, counts this poll, and returns whether
 * the host should query its terminal size. Query failure keeps size unknown. */
bool ghostos_vm_terminal_poll_begin(ghostos_vm_terminal *, uint64_t, bool);
bool ghostos_vm_terminal_resize(ghostos_vm_terminal *, uint16_t, uint16_t);
bool ghostos_vm_terminal_accept_input(ghostos_vm_terminal *, const uint8_t *, size_t);
bool ghostos_vm_terminal_accept_eof(ghostos_vm_terminal *);
/* Views borrow session storage; copy before the next mutating operation. */
void ghostos_vm_terminal_poll_input(const ghostos_vm_terminal *, ghostos_vm_terminal_input *);
void ghostos_vm_terminal_counters_get(const ghostos_vm_terminal *, ghostos_vm_terminal_counters *);
void ghostos_vm_terminal_output_written(ghostos_vm_terminal *, size_t);
void ghostos_vm_terminal_output_flushed(ghostos_vm_terminal *);
size_t ghostos_vm_terminal_event_count(const ghostos_vm_terminal *);
bool ghostos_vm_terminal_event_get(const ghostos_vm_terminal *, size_t, ghostos_vm_terminal_event *);
bool ghostos_vm_terminal_replay_event(const ghostos_vm_terminal_event *, ghostos_vm_terminal_input *);
bool ghostos_vm_terminal_translate(const uint8_t *, size_t, bool *, uint8_t *, size_t, size_t *);
#endif
