#include "ghostos/keyboard_stub.h"

void ghostos_keyboard_stub_init(ghostos_keyboard_stub *keyboard) {
    if (keyboard) keyboard->unused = 0;
}

bool ghostos_keyboard_stub_read_byte(ghostos_keyboard_stub *keyboard, uint8_t *byte) {
    (void)keyboard;
    (void)byte;
    return false;
}
