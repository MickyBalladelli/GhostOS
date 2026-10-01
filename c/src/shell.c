#include "ghostos/shell.h"

#define VT_GROUND 0u
#define VT_ESCAPE 1u
#define VT_CSI 2u
#define VT_SS3 3u

static ghostos_vt_key key(uint32_t kind) { return (ghostos_vt_key){kind, 0}; }
static ghostos_vt_key character(uint32_t value) { return (ghostos_vt_key){GHOSTOS_VT_KEY_CHARACTER, value}; }

void ghostos_vt_input_init(ghostos_vt_input_state *state) {
    if (state) *state = (ghostos_vt_input_state){0};
}

static void reset_parameters(ghostos_vt_input_state *state) {
    state->parameter = 0;
    state->modifier = 0;
    state->third_parameter = 0;
    state->separators = 0;
}

static uint16_t saturating_digit(uint16_t value, uint8_t digit) {
    uint32_t next = (uint32_t)value * 10u + digit;
    return next > UINT16_MAX ? UINT16_MAX : (uint16_t)next;
}

static ghostos_vt_key navigation_key(ghostos_vt_input_state *state, uint8_t byte) {
    bool shifted = state->modifier == 2;
    switch (byte) {
        case 'A': return key(shifted ? GHOSTOS_VT_KEY_SHIFT_UP : GHOSTOS_VT_KEY_UP);
        case 'B': return key(shifted ? GHOSTOS_VT_KEY_SHIFT_DOWN : GHOSTOS_VT_KEY_DOWN);
        case 'C': return key(shifted ? GHOSTOS_VT_KEY_SHIFT_RIGHT : GHOSTOS_VT_KEY_RIGHT);
        case 'D': return key(shifted ? GHOSTOS_VT_KEY_SHIFT_LEFT : GHOSTOS_VT_KEY_LEFT);
        case 'H': return key(shifted ? GHOSTOS_VT_KEY_SHIFT_HOME : GHOSTOS_VT_KEY_HOME);
        case 'F': return key(shifted ? GHOSTOS_VT_KEY_SHIFT_END : GHOSTOS_VT_KEY_END);
        case 't':
            if (state->parameter == 8 && state->separators >= 2 && state->modifier && state->third_parameter) {
                state->resize_columns = state->third_parameter;
                state->resize_rows = state->modifier;
                state->has_resize = 1;
                return key(GHOSTOS_VT_KEY_RESIZE);
            }
            return key(GHOSTOS_VT_KEY_NONE);
        case '~':
            switch (state->parameter) {
                case 1: case 7: return key(shifted ? GHOSTOS_VT_KEY_SHIFT_HOME : GHOSTOS_VT_KEY_HOME);
                case 3: return key(GHOSTOS_VT_KEY_DELETE);
                case 4: case 8: return key(shifted ? GHOSTOS_VT_KEY_SHIFT_END : GHOSTOS_VT_KEY_END);
                case 5: return key(GHOSTOS_VT_KEY_PAGE_UP);
                case 6: return key(GHOSTOS_VT_KEY_PAGE_DOWN);
                default: return key(GHOSTOS_VT_KEY_NONE);
            }
        default: return key(GHOSTOS_VT_KEY_NONE);
    }
}

static ghostos_vt_key advance_now(ghostos_vt_input_state *state, uint8_t byte) {
    switch (state->state) {
        case VT_GROUND:
            switch (byte) {
                case 0x1b: state->state = VT_ESCAPE; return key(GHOSTOS_VT_KEY_NONE);
                case '\n': case '\r': return key(GHOSTOS_VT_KEY_ENTER);
                case '\t': return key(GHOSTOS_VT_KEY_TAB);
                case 8: case 127: return key(GHOSTOS_VT_KEY_BACKSPACE);
                case 19: return key(GHOSTOS_VT_KEY_SAVE);
                case 24: return key(GHOSTOS_VT_KEY_DISCARD_EXIT);
                case 26: return key(GHOSTOS_VT_KEY_SAVE_EXIT);
                case 3: return key(GHOSTOS_VT_KEY_CANCEL);
                default: break;
            }
            if (byte >= 0xc2 && byte <= 0xdf) state->utf8_expected = 2;
            else if (byte >= 0xe0 && byte <= 0xef) state->utf8_expected = 3;
            else if (byte >= 0xf0 && byte <= 0xf4) state->utf8_expected = 4;
            else if (byte >= 0x20 && byte <= 0x7e) return character(byte);
            else return key(GHOSTOS_VT_KEY_NONE);
            state->utf8[0] = byte;
            state->utf8_len = 1;
            return key(GHOSTOS_VT_KEY_NONE);
        case VT_ESCAPE:
            reset_parameters(state);
            if (byte == '[') state->state = VT_CSI;
            else if (byte == 'O') state->state = VT_SS3;
            else {
                state->state = VT_GROUND;
                state->pending_byte = byte;
                state->has_pending = 1;
                return key(GHOSTOS_VT_KEY_ESCAPE);
            }
            return key(GHOSTOS_VT_KEY_NONE);
        case VT_CSI:
            if (byte >= '0' && byte <= '9') {
                uint16_t *target = state->separators == 0 ? &state->parameter :
                    state->separators == 1 ? &state->modifier : &state->third_parameter;
                *target = saturating_digit(*target, (uint8_t)(byte - '0'));
                return key(GHOSTOS_VT_KEY_NONE);
            }
            if (byte == ';') {
                if (state->separators != UINT8_MAX) ++state->separators;
                return key(GHOSTOS_VT_KEY_NONE);
            }
            state->state = VT_GROUND;
            return navigation_key(state, byte);
        case VT_SS3:
            state->state = VT_GROUND;
            return navigation_key(state, byte);
        default:
            state->state = VT_GROUND;
            return key(GHOSTOS_VT_KEY_NONE);
    }
}

static uint32_t utf8_scalar(const uint8_t *bytes, uint32_t length) {
    if (length == 2) return ((uint32_t)(bytes[0] & 0x1f) << 6) | (bytes[1] & 0x3f);
    if (length == 3) return ((uint32_t)(bytes[0] & 0x0f) << 12) |
        ((uint32_t)(bytes[1] & 0x3f) << 6) | (bytes[2] & 0x3f);
    return ((uint32_t)(bytes[0] & 0x07) << 18) | ((uint32_t)(bytes[1] & 0x3f) << 12) |
        ((uint32_t)(bytes[2] & 0x3f) << 6) | (bytes[3] & 0x3f);
}

ghostos_vt_key ghostos_vt_input_advance(ghostos_vt_input_state *state, uint8_t byte) {
    if (!state) return key(GHOSTOS_VT_KEY_NONE);
    if (state->has_pending) {
        uint8_t pending = state->pending_byte;
        state->has_pending = 0;
        ghostos_vt_key output = advance_now(state, pending);
        if (output.kind != GHOSTOS_VT_KEY_NONE) {
            state->pending_byte = byte;
            state->has_pending = 1;
            return output;
        }
    }
    if (state->state == VT_GROUND && state->utf8_expected) {
        if ((byte & 0xc0) != 0x80 || state->utf8_len >= 4) {
            state->utf8_len = 0;
            state->utf8_expected = 0;
            return key(GHOSTOS_VT_KEY_NONE);
        }
        uint32_t position = state->utf8_len;
        state->utf8[position] = byte;
        ++state->utf8_len;
        if (state->utf8_len != state->utf8_expected) return key(GHOSTOS_VT_KEY_NONE);
        uint32_t scalar = utf8_scalar(state->utf8, state->utf8_len);
        bool valid = !(scalar < (state->utf8_len == 2 ? 0x80u : state->utf8_len == 3 ? 0x800u : 0x10000u) ||
            scalar > 0x10ffffu || (scalar >= 0xd800u && scalar <= 0xdfffu));
        state->utf8_len = 0;
        state->utf8_expected = 0;
        return valid ? character(scalar) : key(GHOSTOS_VT_KEY_NONE);
    }
    return advance_now(state, byte);
}

bool ghostos_vt_input_escape_pending(const ghostos_vt_input_state *state) {
    return state && state->state == VT_ESCAPE;
}

ghostos_vt_key ghostos_vt_input_flush_escape(ghostos_vt_input_state *state) {
    if (!ghostos_vt_input_escape_pending(state)) return key(GHOSTOS_VT_KEY_NONE);
    state->state = VT_GROUND;
    return key(GHOSTOS_VT_KEY_ESCAPE);
}

bool ghostos_vt_input_take_resize(ghostos_vt_input_state *state,
    uint32_t *columns, uint32_t *rows) {
    if (!state || !columns || !rows || !state->has_resize) return false;
    *columns = state->resize_columns;
    *rows = state->resize_rows;
    state->has_resize = 0;
    return true;
}

typedef struct { uint32_t start, end; bool present; } word_range;

static bool ascii_space(uint8_t value) {
    return value == ' ' || (value >= '\t' && value <= '\r');
}

static word_range find_word(const uint8_t *line, uint32_t length, uint32_t offset) {
    uint32_t start = offset < length ? offset : length;
    while (start < length && ascii_space(line[start])) ++start;
    if (start == length) return (word_range){0, 0, false};
    uint32_t end = start;
    while (end < length && !ascii_space(line[end])) ++end;
    return (word_range){start, end, true};
}

static bool ascii_equal(const uint8_t *left, uint32_t left_length,
    const uint8_t *right, uint32_t right_length) {
    if (left_length != right_length) return false;
    for (uint32_t index = 0; index < left_length; ++index) {
        uint8_t a = left[index], b = right[index];
        if (a >= 'a' && a <= 'z') a = (uint8_t)(a - ('a' - 'A'));
        if (b >= 'a' && b <= 'z') b = (uint8_t)(b - ('a' - 'A'));
        if (a != b) return false;
    }
    return true;
}

static bool word_is(const uint8_t *line, word_range range, const char *word, uint32_t length) {
    return range.present && ascii_equal(line + range.start, range.end - range.start,
        (const uint8_t *)word, length);
}

static bool help_target(const uint8_t *line, word_range range) {
    return word_is(line, range, "SHOW", 4) || word_is(line, range, "SHO", 3) ||
        word_is(line, range, "TOP", 3) || word_is(line, range, "SET", 3);
}

uint32_t ghostos_shell_expand_command(const uint8_t *line, uint32_t line_length,
    const uint8_t *command, uint32_t command_length, uint8_t *output,
    uint32_t output_capacity, uint32_t *output_length) {
    if (!line || !command || !output || !output_length) return 0;
    word_range first = find_word(line, line_length, 0);
    if (!first.present) return 0;
    bool is_help = word_is(line, first, "HELP", 4);
    word_range start_word = first, end_word = first;
    if (is_help) {
        word_range second = find_word(line, line_length, first.end);
        if (!second.present) return 0;
        start_word = first;
        end_word = second;
        if (help_target(line, second)) {
            word_range third = find_word(line, line_length, second.end);
            if (third.present) end_word = third;
        }
    } else if (word_is(line, first, "SHOW", 4) || word_is(line, first, "SHO", 3) ||
        word_is(line, first, "TOP", 3) || word_is(line, first, "SET", 3)) {
        word_range second = find_word(line, line_length, first.end);
        if (second.present) end_word = second;
    }
    uint32_t span_start = start_word.start, span_end = end_word.end;
    uint32_t replacement_length = is_help ? 5u : 0u;
    if (command_length > output_capacity || replacement_length > output_capacity - command_length)
        return 2;
    uint8_t replacement[512];
    if (replacement_length) {
        replacement[0] = 'H'; replacement[1] = 'E'; replacement[2] = 'L';
        replacement[3] = 'P'; replacement[4] = ' ';
    }
    for (uint32_t index = 0; index < command_length; ++index) {
        replacement[replacement_length + index] = command[index] == '-' ? ' ' : command[index];
    }
    replacement_length += command_length;
    if (ascii_equal(line + span_start, span_end - span_start, replacement, replacement_length)) return 0;
    uint32_t suffix_length = line_length - span_end;
    if (span_start > output_capacity || replacement_length > output_capacity - span_start ||
        suffix_length > output_capacity - span_start - replacement_length) return 2;
    for (uint32_t index = 0; index < span_start; ++index) output[index] = line[index];
    for (uint32_t index = 0; index < replacement_length; ++index)
        output[span_start + index] = replacement[index];
    for (uint32_t index = 0; index < suffix_length; ++index)
        output[span_start + replacement_length + index] = line[span_end + index];
    *output_length = span_start + replacement_length + suffix_length;
    return 1;
}
