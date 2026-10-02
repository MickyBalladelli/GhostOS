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

#define GHOSTOS_VM_SERIAL_FIFO_SIZE 16u

typedef struct ghostos_vm_serial ghostos_vm_serial;
typedef void (*ghostos_vm_serial_console_write)(void *context,
    const uint8_t *bytes, size_t length);
typedef void (*ghostos_vm_serial_console_flush)(void *context);
typedef uint64_t (*ghostos_vm_serial_console_time)(void *context);

ghostos_vm_serial *ghostos_vm_serial_new(uint16_t base, uint64_t now_ns);
void ghostos_vm_serial_free(ghostos_vm_serial *serial);
void ghostos_vm_serial_reset(ghostos_vm_serial *serial, uint64_t now_ns);
uint16_t ghostos_vm_serial_base(const ghostos_vm_serial *serial);
/* Returns whether an edge-triggered receive interrupt should be delivered. */
bool ghostos_vm_serial_push_input(ghostos_vm_serial *serial,
    const uint8_t *bytes, size_t length);
bool ghostos_vm_serial_push_input_lossless(ghostos_vm_serial *serial,
    const uint8_t *bytes, size_t length, bool *interrupt);
bool ghostos_vm_serial_input_pending(const ghostos_vm_serial *serial);
bool ghostos_vm_serial_set_banner(ghostos_vm_serial *serial,
    const uint8_t *bytes, size_t length);
bool ghostos_vm_serial_guest_panicked(const ghostos_vm_serial *serial);
/* Views remain valid until the next mutation of this controller. Empty views
 * may be NULL. Clear-output releases capture storage, leaving host output. */
const uint8_t *ghostos_vm_serial_output(const ghostos_vm_serial *serial, size_t *length);
void ghostos_vm_serial_clear_output(ghostos_vm_serial *serial);
const uint8_t *ghostos_vm_serial_host_output(const ghostos_vm_serial *serial, size_t *length);
bool ghostos_vm_serial_append_host_output(ghostos_vm_serial *serial,
    const uint8_t *bytes, size_t length);
void ghostos_vm_serial_consume_host_output(ghostos_vm_serial *serial, size_t length);
bool ghostos_vm_serial_prepare_host_output(ghostos_vm_serial *serial, size_t *writable);
bool ghostos_vm_serial_auth_waiting(const ghostos_vm_serial *serial);
size_t ghostos_vm_serial_tx_count(const ghostos_vm_serial *serial);
size_t ghostos_vm_serial_rx_count(const ghostos_vm_serial *serial);
/* 0 success, 1 unsupported size, 2 allocation failure. Register aliases
 * retain the low-three-bit port-offset behavior of the existing VM. */
uint8_t ghostos_vm_serial_read(ghostos_vm_serial *serial, uint16_t port,
    uint8_t size, uint64_t *value);
uint8_t ghostos_vm_serial_write(ghostos_vm_serial *serial, uint16_t port,
    uint64_t value, uint8_t size, bool *interrupt);
/* All callbacks run synchronously and are not retained. They must not reenter
 * the controller. Console failures are ignored by the host callback, matching
 * the existing VM. False means host-buffer allocation failed. */
bool ghostos_vm_serial_flush(ghostos_vm_serial *serial,
    ghostos_vm_serial_console_write write_console,
    ghostos_vm_serial_console_flush flush_console,
    ghostos_vm_serial_console_time now_ns, void *context);

#endif
