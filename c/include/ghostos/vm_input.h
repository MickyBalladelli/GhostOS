#ifndef GHOSTOS_VM_INPUT_H
#define GHOSTOS_VM_INPUT_H

#include <stddef.h>
#include <stdint.h>

/* Output buffers need 16, input length, and 4 bytes respectively.
 * Resize filtering supports in-place input/output. Lengths exclude NUL. */
size_t ghostos_vm_input_resize(uint16_t rows, uint16_t columns, uint8_t output[16]);
size_t ghostos_vm_input_strip_resize(const uint8_t *input, size_t length, uint8_t *output);
size_t ghostos_vm_input_scancodes(uint8_t byte, uint8_t output[4]);

#endif
