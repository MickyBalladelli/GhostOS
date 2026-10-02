#include "ghostos/vm_input.h"

#include <stdbool.h>

static size_t decimal(uint16_t value, uint8_t *output) {
    uint8_t reversed[5];
    size_t length = 0;
    do {
        reversed[length++] = (uint8_t)('0' + value % 10);
        value /= 10;
    } while (value != 0);
    for (size_t i = 0; i < length; ++i) output[i] = reversed[length - i - 1];
    return length;
}

size_t ghostos_vm_input_resize(uint16_t rows, uint16_t columns, uint8_t output[16]) {
    output[0] = 0x1b;
    output[1] = '[';
    output[2] = '8';
    output[3] = ';';
    size_t length = 4 + decimal(rows, output + 4);
    output[length++] = ';';
    length += decimal(columns, output + length);
    output[length++] = 't';
    return length;
}

static size_t skip_digits(const uint8_t *input, size_t length, size_t index) {
    while (index < length && input[index] >= '0' && input[index] <= '9') ++index;
    return index;
}

static size_t resize_end(const uint8_t *input, size_t length, size_t start) {
    if (length - start < 4 || input[start] != 0x1b || input[start + 1] != '[' ||
        input[start + 2] != '8' || input[start + 3] != ';') return start;
    size_t index = start + 4;
    size_t end = skip_digits(input, length, index);
    if (end == index || end == length || input[end] != ';') return start;
    index = end + 1;
    end = skip_digits(input, length, index);
    if (end == index || end == length || input[end] != 't') return start;
    return end + 1;
}

size_t ghostos_vm_input_strip_resize(const uint8_t *input, size_t length, uint8_t *output) {
    size_t written = 0;
    for (size_t index = 0; index < length;) {
        size_t end = resize_end(input, length, index);
        if (end != index) index = end;
        else output[written++] = input[index++];
    }
    return written;
}

size_t ghostos_vm_input_scancodes(uint8_t byte, uint8_t output[4]) {
    static const uint8_t letters[26] = {
        0x1e, 0x30, 0x2e, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24,
        0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1f, 0x14,
        0x16, 0x2f, 0x11, 0x2d, 0x15, 0x2c
    };
    bool control = byte >= 1 && byte <= 26 && byte != '\t' &&
        byte != '\n' && byte != '\r' && byte != 0x08;
    if (control) byte = (uint8_t)('a' + byte - 1);
    uint8_t code;
    bool shift = false;
    if (byte >= 'a' && byte <= 'z') code = letters[byte - 'a'];
    else if (byte >= 'A' && byte <= 'Z') {
        code = letters[byte - 'A'];
        shift = true;
    } else if (byte >= '1' && byte <= '9') code = (uint8_t)(2 + byte - '1');
    else {
        switch (byte) {
            case '0': code = 0x0b; break;
            case '!': code = 0x02; shift = true; break;
            case '@': code = 0x03; shift = true; break;
            case '#': code = 0x04; shift = true; break;
            case '$': code = 0x05; shift = true; break;
            case '%': code = 0x06; shift = true; break;
            case '^': code = 0x07; shift = true; break;
            case '&': code = 0x08; shift = true; break;
            case '*': code = 0x09; shift = true; break;
            case '(': code = 0x0a; shift = true; break;
            case ')': code = 0x0b; shift = true; break;
            case ' ': code = 0x39; break;
            case '\n': case '\r': code = 0x1c; break;
            case '\t': code = 0x0f; break;
            case 0x08: case 0x7f: code = 0x0e; break;
            case '-': code = 0x0c; break;
            case '_': code = 0x0c; shift = true; break;
            case '=': code = 0x0d; break;
            case '+': code = 0x0d; shift = true; break;
            case '[': code = 0x1a; break;
            case '{': code = 0x1a; shift = true; break;
            case ']': code = 0x1b; break;
            case '}': code = 0x1b; shift = true; break;
            case '\\': code = 0x2b; break;
            case '|': code = 0x2b; shift = true; break;
            case ';': code = 0x27; break;
            case ':': code = 0x27; shift = true; break;
            case '\'': code = 0x28; break;
            case '"': code = 0x28; shift = true; break;
            case ',': code = 0x33; break;
            case '<': code = 0x33; shift = true; break;
            case '.': code = 0x34; break;
            case '>': code = 0x34; shift = true; break;
            case '/': code = 0x35; break;
            case '?': code = 0x35; shift = true; break;
            case '`': code = 0x29; break;
            case '~': code = 0x29; shift = true; break;
            default: return 0;
        }
    }
    size_t length = 0;
    if (shift) output[length++] = 0x2a;
    if (control) output[length++] = 0x1d;
    output[length++] = code;
    output[length++] = (uint8_t)(code | 0x80);
    if (control) output[length++] = 0x9d;
    if (shift) output[length++] = 0xaa;
    return length;
}
