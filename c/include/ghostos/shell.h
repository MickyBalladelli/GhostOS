#ifndef GHOSTOS_SHELL_H
#define GHOSTOS_SHELL_H

#include <stdbool.h>
#include <stdint.h>

typedef struct {
    uint8_t state;
    uint16_t parameter;
    uint16_t modifier;
    uint16_t third_parameter;
    uint8_t separators;
    uint8_t has_pending;
    uint8_t pending_byte;
    uint8_t has_resize;
    uint32_t resize_columns;
    uint32_t resize_rows;
    uint8_t utf8[4];
    uint32_t utf8_len;
    uint32_t utf8_expected;
} ghostos_vt_input_state;

typedef struct { uint32_t kind, character; } ghostos_vt_key;

enum {
    GHOSTOS_VT_KEY_NONE,
    GHOSTOS_VT_KEY_CHARACTER,
    GHOSTOS_VT_KEY_LEFT,
    GHOSTOS_VT_KEY_RIGHT,
    GHOSTOS_VT_KEY_UP,
    GHOSTOS_VT_KEY_DOWN,
    GHOSTOS_VT_KEY_HOME,
    GHOSTOS_VT_KEY_END,
    GHOSTOS_VT_KEY_SHIFT_LEFT,
    GHOSTOS_VT_KEY_SHIFT_RIGHT,
    GHOSTOS_VT_KEY_SHIFT_UP,
    GHOSTOS_VT_KEY_SHIFT_DOWN,
    GHOSTOS_VT_KEY_PAGE_UP,
    GHOSTOS_VT_KEY_PAGE_DOWN,
    GHOSTOS_VT_KEY_SHIFT_HOME,
    GHOSTOS_VT_KEY_SHIFT_END,
    GHOSTOS_VT_KEY_BACKSPACE,
    GHOSTOS_VT_KEY_DELETE,
    GHOSTOS_VT_KEY_TAB,
    GHOSTOS_VT_KEY_ENTER,
    GHOSTOS_VT_KEY_ESCAPE,
    GHOSTOS_VT_KEY_SAVE,
    GHOSTOS_VT_KEY_SAVE_EXIT,
    GHOSTOS_VT_KEY_DISCARD_EXIT,
    GHOSTOS_VT_KEY_CANCEL,
    GHOSTOS_VT_KEY_RESIZE
};

_Static_assert(sizeof(ghostos_vt_input_state) == 32, "VT input state layout");
_Static_assert(sizeof(ghostos_vt_key) == 8, "VT key layout");

void ghostos_vt_input_init(ghostos_vt_input_state *state);
ghostos_vt_key ghostos_vt_input_advance(ghostos_vt_input_state *state, uint8_t byte);
bool ghostos_vt_input_escape_pending(const ghostos_vt_input_state *state);
ghostos_vt_key ghostos_vt_input_flush_escape(ghostos_vt_input_state *state);
bool ghostos_vt_input_take_resize(ghostos_vt_input_state *state,
    uint32_t *columns, uint32_t *rows);
uint32_t ghostos_shell_expand_command(const uint8_t *line, uint32_t line_length,
    const uint8_t *command, uint32_t command_length, uint8_t *output,
    uint32_t output_capacity, uint32_t *output_length);

#endif
