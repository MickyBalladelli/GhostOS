#ifndef GHOSTOS_USB_KEYBOARD_H
#define GHOSTOS_USB_KEYBOARD_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct {
    uint8_t previous[6];
    uint8_t caps_lock;
    uint8_t pending[6];
    size_t pending_start;
    size_t pending_count;
} ghostos_usb_keyboard_state;

void ghostos_usb_keyboard_init(ghostos_usb_keyboard_state *state);
void ghostos_usb_keyboard_process_report(ghostos_usb_keyboard_state *state,
    const uint8_t report[8]);
bool ghostos_usb_keyboard_take_byte(ghostos_usb_keyboard_state *state, uint8_t *byte);
bool ghostos_usb_keyboard_find_descriptor(const uint8_t *bytes, size_t length,
    uint8_t *interface_number, uint8_t *endpoint, uint16_t *packet_size, uint8_t *interval);

#endif
