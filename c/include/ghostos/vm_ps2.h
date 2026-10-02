#ifndef GHOSTOS_VM_PS2_H
#define GHOSTOS_VM_PS2_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_VM_PS2_OUTPUT_CAPACITY 64u

typedef struct ghostos_vm_ps2 ghostos_vm_ps2;
typedef void (*ghostos_vm_ps2_irq)(void *context, uint8_t vector);

ghostos_vm_ps2 *ghostos_vm_ps2_new(void);
void ghostos_vm_ps2_free(ghostos_vm_ps2 *ps2);
void ghostos_vm_ps2_reset(ghostos_vm_ps2 *ps2);
void ghostos_vm_ps2_set_keyboard_vector(ghostos_vm_ps2 *ps2, uint8_t vector);
void ghostos_vm_ps2_set_mouse_vector(ghostos_vm_ps2 *ps2, uint8_t vector);
bool ghostos_vm_ps2_input_pending(const ghostos_vm_ps2 *ps2);
void ghostos_vm_ps2_keyboard(ghostos_vm_ps2 *ps2, const uint8_t *bytes,
    size_t length, ghostos_vm_ps2_irq irq, void *context);
/* Retains excess input until guest reads make room. False means allocation
 * failed; no input was appended. Disabled keyboard input is ignored. */
bool ghostos_vm_ps2_keyboard_lossless(ghostos_vm_ps2 *ps2, const uint8_t *bytes,
    size_t length, ghostos_vm_ps2_irq irq, void *context);
void ghostos_vm_ps2_mouse_packet(ghostos_vm_ps2 *ps2, const uint8_t packet[3],
    ghostos_vm_ps2_irq irq, void *context);
void ghostos_vm_ps2_mouse_motion(ghostos_vm_ps2 *ps2, int16_t dx, int16_t dy,
    uint8_t buttons, ghostos_vm_ps2_irq irq, void *context);
/* Results: 0 success, 1 unsupported size, 2 invalid address. Callbacks run
 * synchronously and are never retained. They must not reenter the controller. */
uint8_t ghostos_vm_ps2_read(ghostos_vm_ps2 *ps2, uint16_t port, uint8_t size,
    uint64_t *value, ghostos_vm_ps2_irq irq, void *context);
uint8_t ghostos_vm_ps2_write(ghostos_vm_ps2 *ps2, uint16_t port, uint64_t value,
    uint8_t size, ghostos_vm_ps2_irq irq, void *context);

#endif
