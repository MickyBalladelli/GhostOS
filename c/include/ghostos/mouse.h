#ifndef GHOSTOS_MOUSE_H
#define GHOSTOS_MOUSE_H

#include <stdint.h>

typedef struct {
    uint8_t buttons;
    int16_t delta_x;
    int16_t delta_y;
    uint32_t sequence;
} ghostos_mouse_state;

void ghostos_mouse_ingest(uint8_t byte);
ghostos_mouse_state ghostos_mouse_state_read(void);

#endif
