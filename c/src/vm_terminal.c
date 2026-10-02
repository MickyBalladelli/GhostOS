#include "ghostos/vm_terminal.h"
#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(ghostos_vm_terminal_counters) == 48, "terminal counters ABI");
_Static_assert(offsetof(ghostos_vm_terminal_input, rows) == sizeof(void *) + sizeof(size_t),
    "terminal input ABI");
_Static_assert(offsetof(ghostos_vm_terminal_event, rows) == 2 * (sizeof(void *) + sizeof(size_t)),
    "terminal event ABI");

struct ghostos_vm_terminal {
    ghostos_vm_terminal_event *events;
    size_t event_count, event_capacity;
    uint8_t *input;
    size_t input_length, input_capacity;
    ghostos_vm_terminal_counters counters;
    uint64_t last_size_check;
    uint16_t rows, columns;
    bool previous_cr, has_size, has_size_check, resize;
};

static bool reserve(void **memory, size_t *capacity, size_t required, size_t unit) {
    if (required <= *capacity) return true;
    if (required > SIZE_MAX / unit) return false;
    size_t next = *capacity ? *capacity : 16;
    while (next < required) {
        if (next > SIZE_MAX / 2) { next = required; break; }
        next *= 2;
    }
    if (next > SIZE_MAX / unit) next = required;
    void *replacement = realloc(*memory, next * unit);
    if (!replacement) return false;
    *memory = replacement;
    *capacity = next;
    return true;
}

ghostos_vm_terminal *ghostos_vm_terminal_new(void) {
    return calloc(1, sizeof(ghostos_vm_terminal));
}
void ghostos_vm_terminal_free(ghostos_vm_terminal *terminal) {
    if (!terminal) return;
    for (size_t i = 0; i < terminal->event_count; ++i) {
        free((void *)terminal->events[i].raw);
        free((void *)terminal->events[i].bytes);
    }
    free(terminal->events);
    free(terminal->input);
    free(terminal);
}

bool ghostos_vm_terminal_poll_begin(ghostos_vm_terminal *terminal, uint64_t now, bool has_terminal) {
    if (!terminal) return false;
    ++terminal->counters.polls;
    terminal->input_length = 0;
    terminal->resize = false;
    uint64_t elapsed = now >= terminal->last_size_check ? now - terminal->last_size_check : 0;
    bool due = !terminal->has_size || !terminal->has_size_check || elapsed >= GHOSTOS_VM_TERMINAL_SIZE_POLL_NS;
    if (has_terminal && due) {
        terminal->last_size_check = now;
        terminal->has_size_check = true;
        return true;
    }
    return false;
}

static bool record(ghostos_vm_terminal *terminal, uint8_t kind,
    const uint8_t *raw, size_t raw_length, const uint8_t *bytes, size_t length,
    uint16_t rows, uint16_t columns) {
    if (terminal->event_count == SIZE_MAX) return false;
    /* Use a local void pointer to avoid aliasing typed pointer storage. */
    void *entries = terminal->events;
    if (!reserve(&entries, &terminal->event_capacity, terminal->event_count + 1, sizeof(*terminal->events))) return false;
    terminal->events = entries;
    uint8_t *raw_copy = raw_length ? malloc(raw_length) : NULL;
    uint8_t *copy = length ? malloc(length) : NULL;
    if ((raw_length && !raw_copy) || (length && !copy)) { free(raw_copy); free(copy); return false; }
    if (raw_length) memcpy(raw_copy, raw, raw_length);
    if (length) memcpy(copy, bytes, length);
    terminal->events[terminal->event_count++] = (ghostos_vm_terminal_event){
        raw_copy, raw_length, copy, length, rows, columns, kind
    };
    return true;
}

static bool append_input(ghostos_vm_terminal *terminal, const uint8_t *bytes, size_t length) {
    if (length > SIZE_MAX - terminal->input_length) return false;
    void *input = terminal->input;
    if (!reserve(&input, &terminal->input_capacity, terminal->input_length + length, 1)) return false;
    terminal->input = input;
    if (length) memcpy(terminal->input + terminal->input_length, bytes, length);
    terminal->input_length += length;
    return true;
}

bool ghostos_vm_terminal_resize(ghostos_vm_terminal *terminal, uint16_t rows, uint16_t columns) {
    if (!terminal) return false;
    if (terminal->has_size && terminal->rows == rows && terminal->columns == columns) return true;
    terminal->resize = terminal->has_size = true;
    terminal->rows = rows;
    terminal->columns = columns;
    if (!record(terminal, 3, NULL, 0, NULL, 0, rows, columns)) return false;
    ++terminal->counters.resize_events;
    return true;
}

bool ghostos_vm_terminal_translate(const uint8_t *bytes, size_t length, bool *previous_cr,
    uint8_t *output, size_t capacity, size_t *output_length) {
    if ((length && (!bytes || !output)) || !previous_cr || !output_length || capacity < length) return false;
    size_t used = 0;
    for (size_t i = 0; i < length; ++i) {
        uint8_t byte = bytes[i] == 0x7f ? 8 : bytes[i];
        if (byte == '\n' && *previous_cr) { *previous_cr = false; continue; }
        output[used++] = byte;
        *previous_cr = byte == '\r';
    }
    *output_length = used;
    return true;
}

bool ghostos_vm_terminal_accept_input(ghostos_vm_terminal *terminal, const uint8_t *raw, size_t length) {
    if (!terminal || (length && !raw)) return false;
    uint8_t *translated = length ? malloc(length) : NULL;
    if (length && !translated) return false;
    size_t used;
    bool result = ghostos_vm_terminal_translate(raw, length, &terminal->previous_cr, translated, length, &used);
    if (result) result = append_input(terminal, translated, used);
    if (result) {
        terminal->counters.input_bytes += length;
        result = record(terminal, 1, raw, length, translated, used, 0, 0);
    }
    free(translated);
    return result;
}
bool ghostos_vm_terminal_accept_eof(ghostos_vm_terminal *terminal) {
    if (!terminal) return false;
    ++terminal->counters.eof_events;
    uint8_t byte = 4;
    return append_input(terminal, &byte, 1) && record(terminal, 2, NULL, 0, &byte, 1, 0, 0);
}

void ghostos_vm_terminal_poll_input(const ghostos_vm_terminal *terminal, ghostos_vm_terminal_input *input) {
    if (terminal && input) *input = (ghostos_vm_terminal_input){
        terminal->input, terminal->input_length, terminal->rows, terminal->columns, terminal->resize
    };
}
void ghostos_vm_terminal_counters_get(const ghostos_vm_terminal *terminal, ghostos_vm_terminal_counters *counters) {
    if (terminal && counters) *counters = terminal->counters;
}
void ghostos_vm_terminal_output_written(ghostos_vm_terminal *terminal, size_t count) {
    if (terminal) terminal->counters.output_bytes += count;
}
void ghostos_vm_terminal_output_flushed(ghostos_vm_terminal *terminal) {
    if (terminal) ++terminal->counters.output_flushes;
}
size_t ghostos_vm_terminal_event_count(const ghostos_vm_terminal *terminal) {
    return terminal ? terminal->event_count : 0;
}
bool ghostos_vm_terminal_event_get(const ghostos_vm_terminal *terminal, size_t index,
    ghostos_vm_terminal_event *event) {
    if (!terminal || !event || index >= terminal->event_count) return false;
    *event = terminal->events[index];
    return true;
}
bool ghostos_vm_terminal_replay_event(const ghostos_vm_terminal_event *event, ghostos_vm_terminal_input *input) {
    if (!event || !input || event->kind < 1 || event->kind > 3) return false;
    *input = (ghostos_vm_terminal_input){
        event->kind == 3 ? NULL : event->bytes, event->kind == 3 ? 0 : event->length,
        event->rows, event->columns, event->kind == 3
    };
    return true;
}
