#ifndef GHOSTOS_VM_SERIAL_H
#define GHOSTOS_VM_SERIAL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

bool ghostos_vm_serial_translate_newlines(const uint8_t *input, size_t input_length,
    bool previous_was_cr, uint8_t *output, size_t output_capacity,
    size_t *output_length, bool *output_previous_was_cr);

void ghostos_vm_serial_observe_panic_marker(uint8_t byte, size_t *progress,
    bool *detected);

#endif
