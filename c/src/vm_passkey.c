#include "ghostos/vm_passkey.h"
#include <string.h>

static bool character(const uint8_t *bytes, size_t length, size_t *width, uint32_t *value) {
    if (!length) return false;
    uint8_t first = bytes[0];
    if (first < 0x80) { *width = 1; *value = first; return true; }
    size_t count = first >= 0xc2 && first <= 0xdf ? 2 : first >= 0xe0 && first <= 0xef ? 3 : first >= 0xf0 && first <= 0xf4 ? 4 : 0;
    if (!count || length < count) return false;
    uint32_t code = first & (count == 2 ? 0x1f : count == 3 ? 0x0f : 7);
    for (size_t i = 1; i < count; ++i) {
        if ((bytes[i] & 0xc0) != 0x80) return false;
        code = (code << 6) | (bytes[i] & 0x3f);
    }
    if ((count == 3 && code < 0x800) || (count == 4 && code < 0x10000) ||
        (code >= 0xd800 && code <= 0xdfff) || code > 0x10ffff) return false;
    *width = count; *value = code; return true;
}
static bool utf8(const uint8_t *bytes, size_t length) {
    size_t index = 0;
    while (index < length) {
        size_t width; uint32_t value;
        if (!character(bytes + index, length - index, &width, &value)) return false;
        index += width;
    }
    return true;
}
static bool whitespace(uint32_t value) {
    return (value >= 9 && value <= 13) || value == 32 || value == 0x85 || value == 0xa0 ||
        value == 0x1680 || (value >= 0x2000 && value <= 0x200a) || value == 0x2028 ||
        value == 0x2029 || value == 0x202f || value == 0x205f || value == 0x3000;
}
/* Input must be valid UTF-8; every public entry point checks that invariant. */
static void trim(const uint8_t *input, size_t *start, size_t *length) {
    size_t index = 0, first = *length, last = 0;
    while (index < *length) {
        size_t width = 0; uint32_t value = 0;
        (void)character(input + *start + index, *length - index, &width, &value);
        if (!whitespace(value)) { if (first == *length) first = index; last = index + width; }
        index += width;
    }
    *start += first;
    *length = last > first ? last - first : 0;
}
static bool word(const uint8_t *input, size_t length, size_t *cursor, size_t *start, size_t *size) {
    size_t width = 0; uint32_t value = 0;
    while (*cursor < length) {
        (void)character(input + *cursor, length - *cursor, &width, &value);
        if (!whitespace(value)) break;
        *cursor += width;
    }
    if (*cursor == length) return false;
    *start = *cursor;
    while (*cursor < length) {
        (void)character(input + *cursor, length - *cursor, &width, &value);
        if (whitespace(value)) break;
        *cursor += width;
    }
    *size = *cursor - *start;
    return true;
}
static bool equal(const uint8_t *input, size_t length, const char *text) {
    return length == strlen(text) && (!length || !memcmp(input, text, length));
}

static size_t find(const uint8_t *bytes, size_t length, const uint8_t *needle, size_t size, bool last) {
    if (size > length) return SIZE_MAX;
    size_t found = SIZE_MAX;
    for (size_t i = 0; i <= length - size; ++i) {
        if (!memcmp(bytes + i, needle, size)) { found = i; if (!last) break; }
    }
    return found;
}
static size_t marker(const uint8_t *text, size_t length, const char *value) {
    return find(text, length, (const uint8_t *)value, strlen(value), true);
}
static bool contains(const uint8_t *text, size_t length, const char *value) { return marker(text, length, value) != SIZE_MAX; }
static bool ascii_equal(const uint8_t *bytes, size_t length, const char *value) {
    if (length != strlen(value)) return false;
    for (size_t i = 0; i < length; ++i) {
        uint8_t byte = bytes[i];
        if (byte >= 'A' && byte <= 'Z') byte = (uint8_t)(byte + 32);
        if (byte != (uint8_t)value[i]) return false;
    }
    return true;
}
static bool decimal(const uint8_t *bytes, size_t length, size_t *out) {
    size_t start = length && bytes[0] == '+' ? 1 : 0, value = 0;
    if (start == length) return false;
    for (size_t i = start; i < length; ++i) {
        if (bytes[i] < '0' || bytes[i] > '9') return false;
        unsigned digit = bytes[i] - '0';
        if (value > (SIZE_MAX - digit) / 10) return false;
        value = value * 10 + digit;
    }
    *out = value; return true;
}
static bool line(const uint8_t *bytes, size_t length, size_t *cursor, size_t *start, size_t *size) {
    if (*cursor == length) return false;
    *start = *cursor;
    while (*cursor < length && bytes[*cursor] != '\n') ++*cursor;
    *size = *cursor - *start;
    if (*cursor < length) { ++*cursor; if (*size && bytes[*start + *size - 1] == '\r') --*size; }
    return true;
}
bool ghostos_passkey_request_fits(size_t current, size_t incoming) { return current <= 16384 && incoming <= 16384 - current; }
uint32_t ghostos_passkey_request_parse(const uint8_t *bytes, size_t length,
    bool checked, ghostos_passkey_request *out) {
    size_t header_end = find(bytes, length, (const uint8_t *)"\r\n\r\n", 4, false);
    if (header_end == SIZE_MAX) return 0;
    if (!utf8(bytes, header_end)) return 2;
    size_t cursor = 0, start, size, content_length = 0;
    while (line(bytes, header_end, &cursor, &start, &size)) {
        size_t colon = find(bytes + start, size, (const uint8_t *)":", 1, false);
        if (colon == SIZE_MAX || !ascii_equal(bytes + start, colon, "content-length")) continue;
        size_t value_start = start + colon + 1, value_size = size - colon - 1;
        trim(bytes, &value_start, &value_size);
        if (decimal(bytes + value_start, value_size, &content_length)) break;
    }
    size_t body_start = header_end + 4;
    if (content_length > SIZE_MAX - body_start && checked) return 3;
    size_t body_end = body_start + content_length;
    if (body_end > length) return 0;
    cursor = 0;
    if (!line(bytes, header_end, &cursor, &start, &size)) return 2;
    size_t first_cursor = 0, method_start, method_size, target_start, target_size;
    if (!word(bytes + start, size, &first_cursor, &method_start, &method_size) ||
        !word(bytes + start, size, &first_cursor, &target_start, &target_size)) return 2;
    *out = (ghostos_passkey_request){.method_start = start + method_start, .method_length = method_size,
        .target_start = start + target_start, .target_length = target_size, .body_start = body_start, .body_length = content_length};
    while (line(bytes, header_end, &cursor, &start, &size)) {
        size_t colon = find(bytes + start, size, (const uint8_t *)":", 1, false);
        if (colon == SIZE_MAX || !ascii_equal(bytes + start, colon, "x-ghostos-code")) continue;
        size_t token_start = start + colon + 1, token_size = size - colon - 1;
        trim(bytes, &token_start, &token_size);
        out->token_start = token_start; out->token_length = token_size; out->has_token = true; break;
    }
    return body_end < body_start ? 4 : 1;
}
bool ghostos_passkey_query(const uint8_t *target, size_t length,
    const uint8_t *wanted, size_t wanted_length, size_t *start, size_t *size) {
    size_t question = find(target, length, (const uint8_t *)"?", 1, false);
    if (question == SIZE_MAX) return false;
    size_t cursor = question + 1;
    while (cursor <= length) {
        size_t begin = cursor;
        while (cursor < length && target[cursor] != '&') ++cursor;
        size_t pair_length = cursor - begin;
        size_t equals = find(target + begin, pair_length, (const uint8_t *)"=", 1, false);
        if (equals != SIZE_MAX && equals == wanted_length && (!equals || !memcmp(target + begin, wanted, equals))) {
            *start = begin + equals + 1; *size = pair_length - equals - 1; return true;
        }
        if (cursor == length) break;
        ++cursor;
    }
    return false;
}
static uint8_t digit(uint8_t byte) {
    return byte >= '0' && byte <= '9' ? byte - '0' : byte >= 'a' && byte <= 'f' ? byte - 'a' + 10 :
        byte >= 'A' && byte <= 'F' ? byte - 'A' + 10 : UINT8_MAX;
}
bool ghostos_passkey_percent_decode(const uint8_t *input, size_t length, uint8_t *output, size_t *written) {
    size_t count = 0;
    for (size_t i = 0; i < length;) {
        uint8_t byte = input[i];
        if (byte == '%' && length - i >= 3) {
            uint8_t high = digit(input[i + 1]), low = digit(input[i + 2]);
            if (high == UINT8_MAX || low == UINT8_MAX) return false;
            output[count++] = (uint8_t)((high << 4) | low); i += 3;
        } else if (byte == '+') { output[count++] = ' '; ++i; }
        else if (byte < 128) { output[count++] = byte; ++i; }
        else return false;
    }
    if (!utf8(output, count)) return false;
    *written = count; return true;
}
bool ghostos_passkey_valid_username(const uint8_t *input, size_t length) {
    if (!length || length > 32) return false;
    for (size_t i = 0; i < length; ++i) {
        uint8_t byte = input[i];
        if (!((byte >= '0' && byte <= '9') || (byte >= 'a' && byte <= 'z') ||
            (byte >= 'A' && byte <= 'Z') || byte == '.' || byte == '_' || byte == '-' || byte == '$')) return false;
    }
    return true;
}
bool ghostos_passkey_valid_hex(const uint8_t *input, size_t length, size_t maximum) {
    if (!length || length > maximum || length % 2) return false;
    for (size_t i = 0; i < length; ++i) if (digit(input[i]) == UINT8_MAX) return false;
    return true;
}
bool ghostos_passkey_decode_hex(const uint8_t *input, size_t length, uint8_t *output) {
    if (length % 2) return false;
    for (size_t i = 0; i < length / 2; ++i) {
        uint8_t high = digit(input[i * 2]), low = digit(input[i * 2 + 1]);
        if (high == UINT8_MAX || low == UINT8_MAX) return false;
        output[i] = (uint8_t)((high << 4) | low);
    }
    return true;
}
size_t ghostos_passkey_frame(const uint8_t *input, size_t length, uint32_t mode,
    size_t maximum, uint8_t *output, size_t capacity) {
    if (mode == 0) {
        if (length == SIZE_MAX || capacity <= length) return 0;
        if (length) memcpy(output, input, length);
        output[length] = '\r'; return length + 1;
    }
    size_t limit = mode == 1 ? 32 : maximum;
    if (!length || length > UINT16_MAX || length > limit || capacity < length + 3) return 0;
    output[0] = 0; output[1] = (uint8_t)length; output[2] = (uint8_t)(length >> 8);
    memcpy(output + 3, input, length); return length + 3;
}
bool ghostos_passkey_enrollment_pending(const uint8_t *text, size_t length) {
    if (contains(text, length, "Administrator account committed.")) return false;
    size_t enroll = marker(text, length, "\x1b]GhostOSEnroll\x07"), login = marker(text, length, "\x1b]GhostOSLogin\x07");
    return enroll != SIZE_MAX && (login == SIZE_MAX || enroll > login);
}
bool ghostos_passkey_succeeded(const uint8_t *text, size_t length) {
    size_t enroll = marker(text, length, "\x1b]GhostOSEnroll\x07"), login = marker(text, length, "\x1b]GhostOSLogin\x07");
    size_t latest = enroll == SIZE_MAX ? login : login == SIZE_MAX ? enroll : enroll > login ? enroll : login;
    size_t accepted = marker(text, length, "Login accepted.");
    if (accepted != SIZE_MAX && (latest == SIZE_MAX || accepted > latest)) return true;
    size_t shell = marker(text, length, "GhostOS user shell");
    return shell != SIZE_MAX && (latest == SIZE_MAX || shell > latest) && contains(text + shell, length - shell, "$ ");
}
bool ghostos_passkey_login_pending(const uint8_t *text, size_t length) {
    size_t login = marker(text, length, "\x1b]GhostOSLogin\x07");
    if (login == SIZE_MAX) return false;
    size_t enroll = marker(text, length, "\x1b]GhostOSEnroll\x07"), accepted = marker(text, length, "Login accepted."), shell = marker(text, length, "GhostOS user shell");
    return (enroll == SIZE_MAX || login > enroll) && (accepted == SIZE_MAX || login > accepted) && (shell == SIZE_MAX || login > shell);
}
bool ghostos_passkey_challenge(const uint8_t *text, size_t length, size_t *start) {
    static const char name[] = "\x1b]GhostOSChallenge:";
    size_t at = marker(text, length, name);
    if (at == SIZE_MAX) return false;
    at += sizeof(name) - 1;
    if (length - at < 64 || !ghostos_passkey_valid_hex(text + at, 64, 64)) return false;
    *start = at; return true;
}
void ghostos_passkey_observe(ghostos_passkey_state *state,
    const uint8_t *text, size_t length, const uint8_t *new_text, size_t new_length,
    ghostos_passkey_observation *out) {
    *out = (ghostos_passkey_observation){0};
    if (contains(text, length, "Administrator account committed.")) state->committed = true;
    if (state->committed && state->mode <= 1) {
        state->mode = 2; out->error_action = 1; out->challenge_action = 1;
        out->clear_flow = true; state->login_in_progress = false;
    }
    bool pending = !state->committed && ghostos_passkey_enrollment_pending(text, length);
    if (pending) { state->mode = 1; out->challenge_action = 1; }
    if (contains(new_text, new_length, "Administrator account committed.")) {
        state->mode = 2; out->error_action = 1; out->clear_flow = true;
    }
    if (state->login_in_progress && !pending && ghostos_passkey_challenge(text, length, &out->challenge_start)) {
        state->mode = 3; out->challenge_action = 2;
    }
    if (ghostos_passkey_succeeded(text, length)) {
        state->mode = 4; out->error_action = 1; state->login_in_progress = false; out->clear_flow = true;
    } else if (contains(new_text, new_length, "Login failed:")) {
        state->mode = 2; out->challenge_action = 1; out->error_action = 2; state->login_in_progress = false;
    } else if (contains(new_text, new_length, "Username must be 1-32 valid characters.") ||
        contains(new_text, new_length, "Unknown credential type.") || contains(new_text, new_length, "Credential material is invalid.") ||
        contains(new_text, new_length, "Credential rejected.") || contains(new_text, new_length, "Confirmation rejected.")) {
        state->mode = 1; out->clear_flow = true; out->error_action = 3; state->login_in_progress = false;
    } else if (!state->login_in_progress && ghostos_passkey_login_pending(text, length)) {
        state->mode = 2; out->challenge_action = 1;
    }
}
#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(ghostos_passkey_request) == 72, "passkey request ABI");
_Static_assert(sizeof(ghostos_passkey_state) == 8, "passkey state ABI");
_Static_assert(sizeof(ghostos_passkey_observation) == 24, "passkey observation ABI");
#endif
