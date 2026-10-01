#include "ghostos/usb_keyboard.h"

static bool was_pressed(const ghostos_usb_keyboard_state *state, uint8_t usage) {
    for (size_t index = 0; index < 6; ++index) {
        if (state->previous[index] == usage) return true;
    }
    return false;
}

static bool enqueue(ghostos_usb_keyboard_state *state, uint8_t byte) {
    if (state->pending_count == sizeof(state->pending)) return false;
    size_t index = (state->pending_start + state->pending_count) % sizeof(state->pending);
    state->pending[index] = byte;
    ++state->pending_count;
    return true;
}

static bool enqueue_sequence(ghostos_usb_keyboard_state *state, const uint8_t *bytes,
    size_t length) {
    for (size_t index = 0; index < length; ++index) {
        if (!enqueue(state, bytes[index])) return false;
    }
    return true;
}

static bool navigation_sequence(uint8_t usage, bool shifted, const uint8_t **bytes,
    size_t *length) {
    static const uint8_t home[] = "\x1b[H";
    static const uint8_t del[] = "\x1b[3~";
    static const uint8_t end[] = "\x1b[F";
    static const uint8_t right[] = "\x1b[C";
    static const uint8_t right_shift[] = "\x1b[1;2C";
    static const uint8_t left[] = "\x1b[D";
    static const uint8_t left_shift[] = "\x1b[1;2D";
    static const uint8_t down[] = "\x1b[B";
    static const uint8_t down_shift[] = "\x1b[1;2B";
    static const uint8_t up[] = "\x1b[A";
    static const uint8_t up_shift[] = "\x1b[1;2A";
    switch (usage) {
        case 0x4a: *bytes = home; *length = sizeof(home) - 1; return true;
        case 0x4c: *bytes = del; *length = sizeof(del) - 1; return true;
        case 0x4d: *bytes = end; *length = sizeof(end) - 1; return true;
        case 0x4f: *bytes = shifted ? right_shift : right; *length = shifted ? sizeof(right_shift) - 1 : sizeof(right) - 1; return true;
        case 0x50: *bytes = shifted ? left_shift : left; *length = shifted ? sizeof(left_shift) - 1 : sizeof(left) - 1; return true;
        case 0x51: *bytes = shifted ? down_shift : down; *length = shifted ? sizeof(down_shift) - 1 : sizeof(down) - 1; return true;
        case 0x52: *bytes = shifted ? up_shift : up; *length = shifted ? sizeof(up_shift) - 1 : sizeof(up) - 1; return true;
        default: return false;
    }
}

static bool hid_usage(uint8_t usage, uint8_t modifiers, bool caps_lock, uint8_t *byte) {
    bool shifted = (modifiers & 0x22) != 0;
    bool controlled = (modifiers & 0x11) != 0;
    if (usage >= 0x04 && usage <= 0x1d) {
        uint8_t letter = (uint8_t)('a' + usage - 0x04);
        *byte = controlled ? (uint8_t)(letter & 0x1f) :
            (shifted ^ caps_lock ? (uint8_t)(letter - 'a' + 'A') : letter);
        return true;
    }
    if (usage >= 0x1e && usage <= 0x27) {
        static const uint8_t plain[] = "1234567890";
        static const uint8_t upper[] = "!@#$%^&*()";
        *byte = shifted ? upper[usage - 0x1e] : plain[usage - 0x1e];
        return true;
    }
    switch (usage) {
        case 0x28: *byte = '\r'; return true;
        case 0x29: *byte = 3; return true;
        case 0x2a: *byte = 8; return true;
        case 0x2b: *byte = '\t'; return true;
        case 0x2c: *byte = ' '; return true;
        case 0x2d: *byte = shifted ? '_' : '-'; return true;
        case 0x2e: *byte = shifted ? '+' : '='; return true;
        case 0x2f: *byte = shifted ? '{' : '['; return true;
        case 0x30: *byte = shifted ? '}' : ']'; return true;
        case 0x31: *byte = shifted ? '|' : '\\'; return true;
        case 0x33: *byte = shifted ? ':' : ';'; return true;
        case 0x34: *byte = shifted ? '"' : '\''; return true;
        case 0x35: *byte = shifted ? '~' : '`'; return true;
        case 0x36: *byte = shifted ? '<' : ','; return true;
        case 0x37: *byte = shifted ? '>' : '.'; return true;
        case 0x38: *byte = shifted ? '?' : '/'; return true;
        case 0x4c: *byte = 127; return true;
        default: return false;
    }
}

void ghostos_usb_keyboard_init(ghostos_usb_keyboard_state *state) {
    if (state) *state = (ghostos_usb_keyboard_state){0};
}

void ghostos_usb_keyboard_process_report(ghostos_usb_keyboard_state *state,
    const uint8_t report[8]) {
    if (!state || !report) return;
    uint8_t modifiers = report[0];
    bool shifted = (modifiers & 0x22) != 0;
    for (size_t index = 2; index < 8; ++index) {
        uint8_t usage = report[index];
        if (usage == 0 || was_pressed(state, usage)) continue;
        if (usage == 0x39) {
            state->caps_lock ^= 1;
            continue;
        }
        const uint8_t *sequence;
        size_t sequence_length;
        if (navigation_sequence(usage, shifted, &sequence, &sequence_length)) {
            (void)enqueue_sequence(state, sequence, sequence_length);
            continue;
        }
        uint8_t byte;
        if (hid_usage(usage, modifiers, state->caps_lock != 0, &byte))
            (void)enqueue(state, byte);
    }
    for (size_t index = 0; index < 6; ++index) state->previous[index] = report[index + 2];
}

bool ghostos_usb_keyboard_take_byte(ghostos_usb_keyboard_state *state, uint8_t *byte) {
    if (!state || !byte || state->pending_count == 0) return false;
    *byte = state->pending[state->pending_start];
    state->pending_start = (state->pending_start + 1) % sizeof(state->pending);
    --state->pending_count;
    return true;
}

bool ghostos_usb_keyboard_find_descriptor(const uint8_t *bytes, size_t length,
    uint8_t *interface_number, uint8_t *endpoint, uint16_t *packet_size, uint8_t *interval) {
    if ((!bytes && length) || !interface_number || !endpoint || !packet_size || !interval)
        return false;
    size_t offset = 0;
    bool has_keyboard_interface = false;
    uint8_t keyboard_interface = 0;
    while (offset + 2 <= length) {
        size_t descriptor_length = bytes[offset];
        uint8_t descriptor_type = bytes[offset + 1];
        if (descriptor_length < 2 || offset + descriptor_length > length) break;
        if (descriptor_type == 4 && descriptor_length >= 9) {
            uint8_t class = bytes[offset + 5];
            uint8_t subclass = bytes[offset + 6];
            uint8_t protocol = bytes[offset + 7];
            has_keyboard_interface = class == 3 && subclass == 1 && protocol == 1;
            if (has_keyboard_interface) keyboard_interface = bytes[offset + 2];
        } else if (descriptor_type == 5 && descriptor_length >= 7 && has_keyboard_interface) {
            uint8_t candidate = bytes[offset + 2];
            uint8_t attributes = bytes[offset + 3];
            if ((candidate & 0x80) && (attributes & 3) == 3) {
                *interface_number = keyboard_interface;
                *endpoint = candidate;
                *packet_size = ((uint16_t)bytes[offset + 4] |
                    (uint16_t)bytes[offset + 5] << 8) & 0x7ff;
                *interval = bytes[offset + 6];
                return true;
            }
        }
        offset += descriptor_length;
    }
    return false;
}
