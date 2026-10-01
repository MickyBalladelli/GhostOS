#include "ghostos/usb_keyboard.h"

#include <assert.h>

int main(void) {
    ghostos_usb_keyboard_state state;
    uint8_t byte = 0;
    const uint8_t a_report[8] = {0, 0, 0x04, 0, 0, 0, 0, 0};
    const uint8_t release_report[8] = {0};
    const uint8_t shift_a_report[8] = {0x02, 0, 0x04, 0, 0, 0, 0, 0};
    const uint8_t left_report[8] = {0, 0, 0x50, 0, 0, 0, 0, 0};
    const uint8_t descriptors[] = {
        9, 4, 2, 0, 1, 3, 1, 1, 0,
        7, 5, 0x81, 3, 8, 0, 10,
    };
    uint8_t interface_number = 0, endpoint = 0, interval = 0;
    uint16_t packet_size = 0;

    ghostos_usb_keyboard_init(&state);
    ghostos_usb_keyboard_process_report(&state, a_report);
    assert(ghostos_usb_keyboard_take_byte(&state, &byte) && byte == 'a');
    ghostos_usb_keyboard_process_report(&state, a_report);
    assert(!ghostos_usb_keyboard_take_byte(&state, &byte));
    ghostos_usb_keyboard_process_report(&state, release_report);
    ghostos_usb_keyboard_process_report(&state, shift_a_report);
    assert(ghostos_usb_keyboard_take_byte(&state, &byte) && byte == 'A');
    ghostos_usb_keyboard_process_report(&state, release_report);
    ghostos_usb_keyboard_process_report(&state, left_report);
    assert(ghostos_usb_keyboard_take_byte(&state, &byte) && byte == 0x1b);
    assert(ghostos_usb_keyboard_take_byte(&state, &byte) && byte == '[');
    assert(ghostos_usb_keyboard_take_byte(&state, &byte) && byte == 'D');
    assert(!ghostos_usb_keyboard_take_byte(&state, &byte));

    assert(ghostos_usb_keyboard_find_descriptor(descriptors, sizeof(descriptors),
        &interface_number, &endpoint, &packet_size, &interval));
    assert(interface_number == 2 && endpoint == 0x81 && packet_size == 8 && interval == 10);
    return 0;
}
