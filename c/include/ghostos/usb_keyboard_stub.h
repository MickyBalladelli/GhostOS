#ifndef GHOSTOS_USB_KEYBOARD_STUB_H
#define GHOSTOS_USB_KEYBOARD_STUB_H

#include <stdbool.h>
#include <stdint.h>

typedef struct { uint8_t unused; } ghostos_usb_keyboard_stub;

void ghostos_usb_keyboard_stub_init(ghostos_usb_keyboard_stub *keyboard);
bool ghostos_usb_keyboard_stub_read_byte(ghostos_usb_keyboard_stub *keyboard, uint8_t *byte);

#endif
