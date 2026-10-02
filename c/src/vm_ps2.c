#include "ghostos/vm_ps2.h"

#include <stdlib.h>
#include <string.h>

typedef struct {
    uint8_t value;
    bool auxiliary;
} output_byte;

struct ghostos_vm_ps2 {
    output_byte output[GHOSTOS_VM_PS2_OUTPUT_CAPACITY];
    size_t output_head, output_length;
    uint8_t *pending_keyboard;
    size_t pending_head, pending_length, pending_capacity;
    uint8_t command_byte, keyboard_vector, mouse_vector;
    bool expecting_command_byte, expecting_mouse_command;
    bool keyboard_enabled, mouse_enabled, mouse_streaming;
};

static void initialize(ghostos_vm_ps2 *ps2) {
    *ps2 = (ghostos_vm_ps2){
        .command_byte = 0x44, .keyboard_vector = 0x21, .mouse_vector = 0x2c,
        .keyboard_enabled = true
    };
}

ghostos_vm_ps2 *ghostos_vm_ps2_new(void) {
    ghostos_vm_ps2 *ps2 = malloc(sizeof(*ps2));
    if (ps2) initialize(ps2);
    return ps2;
}

void ghostos_vm_ps2_free(ghostos_vm_ps2 *ps2) {
    if (!ps2) return;
    free(ps2->pending_keyboard);
    free(ps2);
}

void ghostos_vm_ps2_reset(ghostos_vm_ps2 *ps2) {
    free(ps2->pending_keyboard);
    initialize(ps2);
}

void ghostos_vm_ps2_set_keyboard_vector(ghostos_vm_ps2 *ps2, uint8_t vector) {
    ps2->keyboard_vector = vector;
}

void ghostos_vm_ps2_set_mouse_vector(ghostos_vm_ps2 *ps2, uint8_t vector) {
    ps2->mouse_vector = vector;
}

bool ghostos_vm_ps2_input_pending(const ghostos_vm_ps2 *ps2) {
    return ps2->output_length != 0 || ps2->pending_length != 0;
}

static void push_output(ghostos_vm_ps2 *ps2, uint8_t value, bool auxiliary,
    ghostos_vm_ps2_irq irq, void *context) {
    if (ps2->output_length == GHOSTOS_VM_PS2_OUTPUT_CAPACITY) {
        ps2->output_head = (ps2->output_head + 1) % GHOSTOS_VM_PS2_OUTPUT_CAPACITY;
        --ps2->output_length;
    }
    size_t slot = (ps2->output_head + ps2->output_length) % GHOSTOS_VM_PS2_OUTPUT_CAPACITY;
    ps2->output[slot] = (output_byte){value, auxiliary};
    ++ps2->output_length;
    bool enabled = auxiliary ? ps2->mouse_enabled && (ps2->command_byte & 2) :
        ps2->keyboard_enabled && (ps2->command_byte & 1);
    if (enabled && irq) irq(context, auxiliary ? ps2->mouse_vector : ps2->keyboard_vector);
}

static void refill_keyboard(ghostos_vm_ps2 *ps2, ghostos_vm_ps2_irq irq, void *context) {
    while (ps2->output_length < GHOSTOS_VM_PS2_OUTPUT_CAPACITY && ps2->pending_length) {
        uint8_t byte = ps2->pending_keyboard[ps2->pending_head++];
        --ps2->pending_length;
        push_output(ps2, byte, false, irq, context);
    }
    if (!ps2->pending_length) ps2->pending_head = 0;
}

void ghostos_vm_ps2_keyboard(ghostos_vm_ps2 *ps2, const uint8_t *bytes,
    size_t length, ghostos_vm_ps2_irq irq, void *context) {
    if (!ps2->keyboard_enabled) return;
    for (size_t i = 0; i < length; ++i) push_output(ps2, bytes[i], false, irq, context);
}

bool ghostos_vm_ps2_keyboard_lossless(ghostos_vm_ps2 *ps2, const uint8_t *bytes,
    size_t length, ghostos_vm_ps2_irq irq, void *context) {
    if (!ps2->keyboard_enabled || !length) return true;
    if (!bytes || length > SIZE_MAX - ps2->pending_length) return false;
    size_t needed = ps2->pending_length + length;
    if (needed > ps2->pending_capacity) {
        size_t capacity = ps2->pending_capacity;
        if (capacity < GHOSTOS_VM_PS2_OUTPUT_CAPACITY) capacity = GHOSTOS_VM_PS2_OUTPUT_CAPACITY;
        while (capacity < needed) {
            if (capacity > SIZE_MAX / 2) {
                capacity = needed;
                break;
            }
            capacity *= 2;
        }
        uint8_t *replacement = malloc(capacity);
        if (!replacement) return false;
        if (ps2->pending_length)
            memcpy(replacement, ps2->pending_keyboard + ps2->pending_head, ps2->pending_length);
        free(ps2->pending_keyboard);
        ps2->pending_keyboard = replacement;
        ps2->pending_capacity = capacity;
        ps2->pending_head = 0;
    } else if (length > ps2->pending_capacity - ps2->pending_head - ps2->pending_length) {
        memmove(ps2->pending_keyboard, ps2->pending_keyboard + ps2->pending_head,
            ps2->pending_length);
        ps2->pending_head = 0;
    }
    memcpy(ps2->pending_keyboard + ps2->pending_head + ps2->pending_length, bytes, length);
    ps2->pending_length = needed;
    refill_keyboard(ps2, irq, context);
    return true;
}

void ghostos_vm_ps2_mouse_packet(ghostos_vm_ps2 *ps2, const uint8_t packet[3],
    ghostos_vm_ps2_irq irq, void *context) {
    if (!ps2->mouse_enabled || !ps2->mouse_streaming) return;
    for (unsigned i = 0; i < 3; ++i) push_output(ps2, packet[i], true, irq, context);
}

void ghostos_vm_ps2_mouse_motion(ghostos_vm_ps2 *ps2, int16_t dx, int16_t dy,
    uint8_t buttons, ghostos_vm_ps2_irq irq, void *context) {
    if (dx < -255) dx = -255;
    if (dx > 255) dx = 255;
    if (dy < -255) dy = -255;
    if (dy > 255) dy = 255;
    /* Preserve the Rust model's wrap to eight bits before sign selection. */
    uint8_t x = (uint8_t)dx, y = (uint8_t)dy;
    uint8_t packet[3] = {
        (uint8_t)(0x08 | (buttons & 7) | (x & 0x80 ? 0x10 : 0) | (y & 0x80 ? 0x20 : 0)), x, y
    };
    ghostos_vm_ps2_mouse_packet(ps2, packet, irq, context);
}

static void keyboard_command(ghostos_vm_ps2 *ps2, uint8_t value,
    ghostos_vm_ps2_irq irq, void *context) {
    push_output(ps2, 0xfa, false, irq, context);
    switch (value) {
        case 0xff: push_output(ps2, 0xaa, false, irq, context); break;
        case 0xf2:
            push_output(ps2, 0xab, false, irq, context);
            push_output(ps2, 0x83, false, irq, context);
            break;
        case 0xf4: ps2->keyboard_enabled = true; break;
        case 0xf5: ps2->keyboard_enabled = false; break;
        default: break;
    }
}

static void mouse_command(ghostos_vm_ps2 *ps2, uint8_t value,
    ghostos_vm_ps2_irq irq, void *context) {
    if (value == 0xf4) ps2->mouse_streaming = true;
    if (value == 0xf5) ps2->mouse_streaming = false;
    push_output(ps2, 0xfa, true, irq, context);
    if (value == 0xff) push_output(ps2, 0xaa, true, irq, context);
    if (value == 0xff || value == 0xf2) push_output(ps2, 0, true, irq, context);
}

uint8_t ghostos_vm_ps2_read(ghostos_vm_ps2 *ps2, uint16_t port, uint8_t size,
    uint64_t *value, ghostos_vm_ps2_irq irq, void *context) {
    if (size != 1) return 1;
    if (port == 0x60) {
        *value = 0;
        if (ps2->output_length) {
            *value = ps2->output[ps2->output_head].value;
            ps2->output_head = (ps2->output_head + 1) % GHOSTOS_VM_PS2_OUTPUT_CAPACITY;
            --ps2->output_length;
        }
        refill_keyboard(ps2, irq, context);
    } else if (port == 0x64) {
        *value = ps2->output_length ?
            1u | (ps2->output[ps2->output_head].auxiliary ? 0x20u : 0u) : 0;
    } else return 2;
    return 0;
}

uint8_t ghostos_vm_ps2_write(ghostos_vm_ps2 *ps2, uint16_t port, uint64_t raw,
    uint8_t size, ghostos_vm_ps2_irq irq, void *context) {
    if (size != 1) return 1;
    uint8_t value = (uint8_t)raw;
    if (port == 0x64) {
        switch (value) {
            case 0x20: push_output(ps2, ps2->command_byte, false, irq, context); break;
            case 0x60: ps2->expecting_command_byte = true; break;
            case 0xa7: ps2->mouse_enabled = false; ps2->command_byte |= 0x20; break;
            case 0xa8: ps2->mouse_enabled = true; ps2->command_byte &= (uint8_t)~0x20; break;
            case 0xad: ps2->keyboard_enabled = false; ps2->command_byte |= 0x10; break;
            case 0xae: ps2->keyboard_enabled = true; ps2->command_byte &= (uint8_t)~0x10; break;
            case 0xaa: push_output(ps2, 0x55, false, irq, context); break;
            case 0xab: push_output(ps2, 0, false, irq, context); break;
            case 0xd4: ps2->expecting_mouse_command = true; break;
            default: break;
        }
    } else if (port == 0x60) {
        if (ps2->expecting_command_byte) {
            ps2->command_byte = value;
            ps2->keyboard_enabled = !(value & 0x10);
            ps2->mouse_enabled = !(value & 0x20);
            ps2->expecting_command_byte = false;
        } else if (ps2->expecting_mouse_command) {
            ps2->expecting_mouse_command = false;
            mouse_command(ps2, value, irq, context);
        } else keyboard_command(ps2, value, irq, context);
    } else return 2;
    return 0;
}
