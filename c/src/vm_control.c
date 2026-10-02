#include "ghostos/vm_control.h"
#include <stdlib.h>
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
static bool ascii_whitespace(uint8_t value) { return value == 9 || value == 10 || value == 12 || value == 13 || value == 32; }
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
uint32_t ghostos_monitor_frame(const ghostos_monitor_buffer *state, bool eof, size_t *command_length) {
    size_t newline = 0;
    while (newline < state->length && state->bytes[newline] != '\n') ++newline;
    if (newline == state->length) return eof ? 3 : 0;
    for (size_t i = newline + 1; i < state->length; ++i) if (!ascii_whitespace(state->bytes[i])) return 4;
    size_t length = newline;
    if (length && state->bytes[length - 1] == '\r') --length;
    if (!utf8(state->bytes, length)) return 5;
    *command_length = length; return 1;
}
uint32_t ghostos_monitor_push(ghostos_monitor_buffer *state, const uint8_t *bytes,
    size_t length, size_t *command_length) {
    if (length > 4096 - state->length) return 2;
    if (length) memcpy(state->bytes + state->length, bytes, length);
    state->length += length;
    return ghostos_monitor_frame(state, false, command_length);
}
uint32_t ghostos_monitor_parse(const uint8_t *input, size_t length, ghostos_monitor_command *out) {
    if (!utf8(input, length)) return 4;
    size_t start = 0;
    trim(input, &start, &length);
    if (length > 2048) return 1;
    const uint8_t *command = input + start;
    *out = (ghostos_monitor_command){0};
    if (equal(command, length, "help") || equal(command, length, "?")) return 0;
    if (equal(command, length, "quit") || equal(command, length, "exit")) { out->kind = 3; return 0; }
    static const char *const names[] = {"status", "devices", "disks", "snapshots", "migration", "registers"};
    for (uint32_t topic = 0; topic < 6; ++topic) {
        size_t name_length = strlen(names[topic]);
        bool matched = equal(command, length, names[topic]) ||
            (length == name_length + 5 && !memcmp(command, "info ", 5) && !memcmp(command + 5, names[topic], name_length));
        bool sensitive = topic == 5;
        if ((topic == 0 || topic == 2) && length == name_length + 22 &&
            !memcmp(command, "info ", 5) && !memcmp(command + 5, names[topic], name_length) &&
            !memcmp(command + 5 + name_length, " --show-sensitive", 17)) { matched = true; sensitive = true; }
        if (matched) { out->kind = 1; out->topic = topic; out->sensitive = sensitive; return 0; }
    }
    if (length >= 5 && !memcmp(command, "save ", 5)) {
        size_t path_start = start + 5, path_length = length - 5;
        trim(input, &path_start, &path_length);
        if (!path_length) return 2;
        out->kind = 2; out->path_start = path_start; out->path_length = path_length; return 0;
    }
    return 3;
}
uint32_t ghostos_monitor_permissions(const uint8_t *input, size_t length,
    uint8_t *permissions, size_t *error_start, size_t *error_length) {
    if (!utf8(input, length)) return 3;
    uint8_t result = 0;
    size_t cursor = 0;
    while (cursor < length) {
        size_t start = cursor;
        while (cursor < length && input[cursor] != ',') ++cursor;
        size_t size = cursor - start;
        trim(input, &start, &size);
        if (cursor < length) ++cursor;
        if (!size) continue;
        const uint8_t *name = input + start;
        if (equal(name, size, "all")) { *permissions = UINT8_MAX; return 0; }
        uint8_t bit = equal(name, size, "status") ? 1 : equal(name, size, "device") || equal(name, size, "devices") ? 2 :
            equal(name, size, "disk") || equal(name, size, "disks") ? 4 : equal(name, size, "migration") ? 8 :
            equal(name, size, "save") || equal(name, size, "snapshot-save") ? 16 : equal(name, size, "quit") || equal(name, size, "stop") ? 32 :
            equal(name, size, "sensitive") || equal(name, size, "diagnostics") ? 64 : 0;
        if (!bit) { *error_start = start; *error_length = size; return 2; }
        result |= bit;
    }
    if (!result) return 1;
    *permissions = result; return 0;
}
size_t ghostos_monitor_json_string(const uint8_t *input, size_t length, uint8_t *output, size_t capacity) {
    size_t needed = 2;
    for (size_t i = 0; i < length; ++i) {
        uint8_t byte = input[i];
        size_t width = byte == '"' || byte == '\\' || byte == '\n' || byte == '\r' || byte == '\t' ? 2 : byte <= 31 ? 6 : 1;
        if (needed > SIZE_MAX - width) return SIZE_MAX;
        needed += width;
    }
    if (!output || capacity < needed) return needed;
    static const char hex[] = "0123456789abcdef";
    size_t index = 0; output[index++] = '"';
    for (size_t i = 0; i < length; ++i) {
        uint8_t byte = input[i];
        if (byte == '"' || byte == '\\' || byte == '\n' || byte == '\r' || byte == '\t') {
            output[index++] = '\\';
            output[index++] = byte == '\n' ? 'n' : byte == '\r' ? 'r' : byte == '\t' ? 't' : byte;
        } else if (byte <= 31) {
            memcpy(output + index, "\\u00", 4); index += 4;
            output[index++] = (uint8_t)hex[byte >> 4]; output[index++] = (uint8_t)hex[byte & 15];
        } else output[index++] = byte;
    }
    output[index++] = '"'; return index;
}
bool ghostos_monitor_response_fits(size_t length) { return length <= 65536; }
typedef struct { uint8_t *output; size_t capacity, length; bool overflow; } writer;
static void append(writer *state, const void *bytes, size_t length) {
    if (state->overflow) return;
    if (length > SIZE_MAX - state->length) { state->overflow = true; return; }
    if (state->output && state->length <= state->capacity && length <= state->capacity - state->length && length)
        memcpy(state->output + state->length, bytes, length);
    state->length += length;
}
static void text(writer *state, const char *value) { append(state, value, strlen(value)); }
static void json(writer *state, const uint8_t *input, size_t length) {
    if (state->overflow) return;
    size_t needed = ghostos_monitor_json_string(input, length, NULL, 0);
    if (needed == SIZE_MAX || needed > SIZE_MAX - state->length) { state->overflow = true; return; }
    if (state->output && state->length <= state->capacity && needed <= state->capacity - state->length)
        (void)ghostos_monitor_json_string(input, length, state->output + state->length, state->capacity - state->length);
    state->length += needed;
}
static size_t finish(const writer *state) { return state->overflow ? SIZE_MAX : state->length; }
static void envelope_start(writer *state, const uint8_t *command, size_t length) {
    text(state, "{\"ok\":true,\"command\":"); json(state, command, length); text(state, ",\"data\":");
}
size_t ghostos_monitor_envelope(const uint8_t *command, size_t command_length,
    const uint8_t *data, size_t data_length, uint8_t *output, size_t capacity) {
    writer state = {output, capacity, 0, false};
    envelope_start(&state, command, command_length); append(&state, data, data_length); text(&state, "}\n");
    return finish(&state);
}
size_t ghostos_monitor_failure(bool has_command, const uint8_t *command, size_t command_length,
    const uint8_t *code, size_t code_length, const uint8_t *message, size_t message_length,
    uint8_t *output, size_t capacity) {
    writer state = {output, capacity, 0, false};
    text(&state, "{\"ok\":false,\"command\":");
    if (has_command) json(&state, command, command_length); else text(&state, "null");
    text(&state, ",\"error\":{\"code\":"); json(&state, code, code_length);
    text(&state, ",\"message\":"); json(&state, message, message_length); text(&state, "}}\n");
    return finish(&state);
}
size_t ghostos_monitor_action(const uint8_t *command, size_t command_length,
    const uint8_t *action, size_t action_length, uint8_t *output, size_t capacity) {
    writer state = {output, capacity, 0, false};
    envelope_start(&state, command, command_length); text(&state, "{\"action\":");
    json(&state, action, action_length); text(&state, ",\"completed\":true}}\n");
    return finish(&state);
}
size_t ghostos_monitor_help(uint8_t *output, size_t capacity) {
    static const char commands[] = "{\"commands\":[\"help\",\"info status\",\"info status --show-sensitive\",\"info devices\",\"info disks\",\"info disks --show-sensitive\",\"info snapshots\",\"info migration\",\"info registers\",\"save PATH\",\"quit\"]}";
    return ghostos_monitor_envelope((const uint8_t *)"help", 4, (const uint8_t *)commands, sizeof(commands) - 1, output, capacity);
}

struct ghostos_monitor_nonces { uint8_t nonces[1024][32]; size_t head, length; };
ghostos_monitor_nonces *ghostos_monitor_nonces_new(void) { return calloc(1, sizeof(ghostos_monitor_nonces)); }
void ghostos_monitor_nonces_free(ghostos_monitor_nonces *state) { free(state); }
static bool number(const uint8_t *input, size_t length, uint64_t *value) {
    size_t i = 0;
    if (length && input[0] == '+') ++i;
    if (i == length) return false;
    uint64_t result = 0;
    for (; i < length; ++i) {
        if (input[i] < '0' || input[i] > '9') return false;
        unsigned digit = input[i] - '0';
        if (result > (UINT64_MAX - digit) / 10) return false;
        result = result * 10 + digit;
    }
    *value = result; return true;
}
static uint8_t hex_value(uint8_t byte) {
    return byte >= '0' && byte <= '9' ? byte - '0' : byte >= 'a' && byte <= 'f' ? byte - 'a' + 10 :
        byte >= 'A' && byte <= 'F' ? byte - 'A' + 10 : UINT8_MAX;
}
static bool decode_hex(const uint8_t *input, uint8_t output[32]) {
    for (size_t i = 0; i < 32; ++i) {
        uint8_t high = hex_value(input[i * 2]), low = hex_value(input[i * 2 + 1]);
        if (high == UINT8_MAX || low == UINT8_MAX) return false;
        output[i] = (uint8_t)((high << 4) | low);
    }
    return true;
}
static uint8_t permission(const ghostos_monitor_command *command) {
    if (command->kind == 0) return 1;
    if (command->kind == 2) return 16;
    if (command->kind == 3) return 32;
    return command->topic == 1 ? 2 : command->topic == 2 ? 4 : command->topic == 4 ? 8 : 1;
}
uint32_t ghostos_monitor_authenticate(ghostos_monitor_nonces *state, uint8_t permissions,
    const uint8_t *input, size_t length, const ghostos_monitor_auth_io *io, ghostos_monitor_auth_result *out) {
    *out = (ghostos_monitor_auth_result){0};
    if (!utf8(input, length)) return 7;
    size_t cursor = 0, start, size, timestamp_start, timestamp_size, nonce_start, nonce_size, tag_start, tag_size;
    if (!word(input, length, &cursor, &start, &size)) return 1;
    if (!equal(input + start, size, "auth")) return 2;
    if (!word(input, length, &cursor, &timestamp_start, &timestamp_size)) return 3;
    if (!word(input, length, &cursor, &nonce_start, &nonce_size)) return 4;
    if (!word(input, length, &cursor, &tag_start, &tag_size)) return 5;
    size_t command_start = cursor, command_length = length - cursor;
    trim(input, &command_start, &command_length);
    if (!command_length) return 6;
    out->command_start = command_start; out->command_length = command_length;
    uint64_t timestamp, now;
    if (!number(input + timestamp_start, timestamp_size, &timestamp)) return 7;
    if (!io->now(io->context, &now)) return 8;
    if ((now > timestamp ? now - timestamp : timestamp - now) > 300) return 9;
    uint8_t nonce[32], tag[32];
    if (nonce_size != 64) return 10;
    if (!decode_hex(input + nonce_start, nonce)) return 11;
    if (tag_size != 64) return 12;
    if (!decode_hex(input + tag_start, tag)) return 13;
    for (size_t i = 0; i < state->length; ++i) if (!memcmp(state->nonces[(state->head + i) % 1024], nonce, 32)) return 14;
    if (!io->verify(io->context, timestamp, nonce, input + command_start, command_length, tag)) return 15;
    out->parse_error = ghostos_monitor_parse(input + command_start, command_length, &out->command);
    if (out->parse_error) return 16;
    uint8_t required = permission(&out->command);
    if (!(permissions & required)) { out->missing_permission = required; return 17; }
    if (out->command.sensitive && !(permissions & 64)) return 18;
    if (state->length == 1024) { state->head = (state->head + 1) % 1024; --state->length; }
    memcpy(state->nonces[(state->head + state->length) % 1024], nonce, 32); ++state->length;
    return 0;
}

#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(ghostos_monitor_buffer) == 4104, "monitor buffer ABI");
_Static_assert(sizeof(ghostos_monitor_command) == 32, "monitor command ABI");
_Static_assert(sizeof(ghostos_monitor_auth_result) == 56, "monitor auth result ABI");
#endif
