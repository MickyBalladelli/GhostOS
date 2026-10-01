#include "ghostos/mouse.h"

#include <stdatomic.h>

static atomic_uchar packet[3];
static atomic_uchar packet_index;
static atomic_uint sequence;

void ghostos_mouse_ingest(uint8_t byte) {
    uint8_t index = atomic_load_explicit(&packet_index, memory_order_relaxed);
    if (index == 0 && (byte & 0x08u) == 0) return;
    atomic_store_explicit(&packet[index], byte, memory_order_relaxed);
    if (index == 2) {
        atomic_store_explicit(&packet_index, 0, memory_order_release);
        atomic_fetch_add_explicit(&sequence, 1, memory_order_release);
    } else {
        atomic_store_explicit(&packet_index, (uint8_t)(index + 1), memory_order_release);
    }
}

ghostos_mouse_state ghostos_mouse_state_read(void) {
    for (;;) {
        uint32_t before = atomic_load_explicit(&sequence, memory_order_acquire);
        uint8_t flags = atomic_load_explicit(&packet[0], memory_order_relaxed);
        uint8_t x = atomic_load_explicit(&packet[1], memory_order_relaxed);
        uint8_t y = atomic_load_explicit(&packet[2], memory_order_relaxed);
        uint32_t after = atomic_load_explicit(&sequence, memory_order_acquire);
        if (before == after) {
            int16_t delta_x = (flags & 0x10u) ? (int16_t)((int16_t)x - 256) : (int16_t)x;
            int16_t raw_y = (flags & 0x20u) ? (int16_t)((int16_t)y - 256) : (int16_t)y;
            return (ghostos_mouse_state){(uint8_t)(flags & 0x07u), delta_x,
                (int16_t)-raw_y, after};
        }
    }
}
