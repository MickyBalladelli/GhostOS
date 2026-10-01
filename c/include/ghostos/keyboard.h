#ifndef GHOSTOS_KEYBOARD_H
#define GHOSTOS_KEYBOARD_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_PS2_KEYBOARD_DATA_PORT UINT16_C(0x60)
#define GHOSTOS_PS2_KEYBOARD_STATUS_PORT UINT16_C(0x64)

typedef struct {
    void *context;
    uint8_t (*read_port)(void *context, uint16_t port);
    void (*write_port)(void *context, uint16_t port, uint8_t value);
    void (*spin)(void *context);
    void (*mouse_byte)(void *context, uint8_t byte);
} ghostos_keyboard_io;

typedef struct {
    bool left_shift, right_shift, control, caps_lock, extended;
    uint8_t pending[4];
    uint8_t pending_start, pending_count;
} ghostos_keyboard;

void ghostos_keyboard_init(ghostos_keyboard *keyboard);
void ghostos_keyboard_create(ghostos_keyboard *keyboard, const ghostos_keyboard_io *io);
void ghostos_keyboard_initialize_controller(const ghostos_keyboard_io *io);
bool ghostos_keyboard_read_byte(ghostos_keyboard *keyboard, const ghostos_keyboard_io *io, uint8_t *byte);
bool ghostos_keyboard_read_boot_byte(const ghostos_keyboard_io *io, uint8_t *byte);
bool ghostos_keyboard_x86_io(ghostos_keyboard_io *io, void *context,
    void (*mouse_byte)(void *context, uint8_t byte));

#endif
