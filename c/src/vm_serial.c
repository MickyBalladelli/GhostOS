#include "ghostos/vm_serial.h"

static const uint8_t panic_marker[] = "KERNEL PANIC";

void ghostos_vm_serial_observe_panic_marker(uint8_t byte, size_t *progress,
    bool *detected) {
    if (!progress || !detected || *detected) return;
    if (*progress >= sizeof(panic_marker) - 1) {
        *progress = 0;
        return;
    }
    if (byte == panic_marker[*progress]) {
        ++*progress;
        if (*progress == sizeof(panic_marker) - 1) *detected = true;
    } else {
        *progress = byte == panic_marker[0] ? 1 : 0;
    }
}

bool ghostos_vm_serial_translate_newlines(const uint8_t *input, size_t input_length,
    bool previous_was_cr, uint8_t *output, size_t output_capacity,
    size_t *output_length, bool *output_previous_was_cr) {
    if ((!input && input_length) || !output_length || !output_previous_was_cr)
        return false;
    size_t required = input_length;
    bool last_was_cr = previous_was_cr;
    for (size_t i = 0; i < input_length; ++i) {
        if (input[i] == '\n' && !last_was_cr) {
            if (required == SIZE_MAX) return false;
            ++required;
        }
        last_was_cr = input[i] == '\r';
    }
    *output_length = required;
    *output_previous_was_cr = last_was_cr;
    if (!output) return output_capacity == 0;
    if (output_capacity < required) return false;
    size_t position = 0;
    last_was_cr = previous_was_cr;
    for (size_t i = 0; i < input_length; ++i) {
        uint8_t byte = input[i];
        if (byte == '\n' && !last_was_cr) output[position++] = '\r';
        output[position++] = byte;
        last_was_cr = byte == '\r';
    }
    return position == required;
}
