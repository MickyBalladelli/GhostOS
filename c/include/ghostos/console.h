#ifndef GHOSTOS_CONSOLE_H
#define GHOSTOS_CONSOLE_H

#include "ghostos/boot_protocol.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct {
    void *context;
    void (*init)(void *context, ghostos_framebuffer_info framebuffer);
    void (*write_byte)(void *context, uint8_t byte);
    void (*clear)(void *context);
    void (*terminal_size)(void *context, size_t *columns, size_t *rows);
    bool (*read_byte)(void *context, uint8_t *byte);
} ghostos_console_backend;

void ghostos_console_init(ghostos_framebuffer_info framebuffer, ghostos_console_backend backend);
void ghostos_console_write_bytes(const uint8_t *bytes, size_t length);
void ghostos_console_write_byte(uint8_t byte);
bool ghostos_console_read_byte(uint8_t *byte);
void ghostos_console_clear(void);
void ghostos_console_terminal_size(size_t *columns, size_t *rows);
void ghostos_console_set_remote_terminal_size(size_t columns, size_t rows);

#endif
