#include "ghostos/inference.h"
static bool text_is(const uint8_t *bytes, size_t length, const char *text) {
    size_t i;
    for (i = 0; text[i]; ++i) if (i == length || bytes[i] != (uint8_t)text[i]) return false;
    return i == length;
}
static bool space(uint8_t byte) { return byte == ' ' || byte == '\t' || byte == '\n' || byte == '\r'; }
static bool utf8_ok(const uint8_t *bytes, size_t length) {
    size_t i = 0;
    while (i < length) {
        uint8_t byte = bytes[i];
        size_t need = 1, j;
        if (byte < 0x80) need = 1;
        else if (byte >= 0xc2 && byte <= 0xdf) need = 2;
        else if (byte >= 0xe0 && byte <= 0xef) need = 3;
        else if (byte >= 0xf0 && byte <= 0xf4) need = 4;
        else return false;
        if (i + need > length) return false;
        for (j = 1; j < need; ++j) if ((bytes[i + j] & 0xc0) != 0x80) return false;
        i += need;
    }
    return true;
}
static void skip_space(const uint8_t *bytes, size_t length, size_t *cursor) {
    while (*cursor < length && space(bytes[*cursor])) *cursor += 1;
}
static bool find_key(const uint8_t *body, size_t length, const char *key, size_t *value_at, bool last) {
    size_t key_length = 0, cursor = 0;
    bool found = false;
    while (key[key_length]) key_length += 1;
    while (cursor + key_length + 2 <= length) {
        size_t i, value;
        bool match = body[cursor] == '"' && body[cursor + key_length + 1] == '"';
        for (i = 0; match && i < key_length; ++i) if (body[cursor + 1 + i] != (uint8_t)key[i]) match = false;
        if (!match) { cursor += 1; continue; }
        value = cursor + key_length + 2;
        skip_space(body, length, &value);
        if (value >= length || body[value] != ':') { cursor += 1; continue; }
        value += 1;
        skip_space(body, length, &value);
        *value_at = value;
        found = true;
        if (!last) return true;
        cursor = value;
    }
    return found;
}
static int json_string(const uint8_t *body, size_t length, const char *key, bool last, const uint8_t **text, size_t *text_length) {
    size_t value = 0, start;
    bool escaped = false;
    if (!find_key(body, length, key, &value, last) || value >= length || body[value] != '"') return 2;
    start = value + 1;
    value = start;
    while (value < length) {
        if (body[value] == '"' && !escaped) {
            *text = body + start;
            *text_length = value - start;
            return 0;
        }
        escaped = !escaped && body[value] == '\\';
        value += 1;
    }
    return 2;
}
static bool json_u32(const uint8_t *body, size_t length, const char *key, uint32_t *value) {
    size_t cursor = 0;
    uint32_t parsed = 0;
    bool any = false;
    if (!find_key(body, length, key, &cursor, false)) return false;
    while (cursor < length && body[cursor] >= '0' && body[cursor] <= '9') {
        uint32_t digit = (uint32_t)(body[cursor] - '0');
        if (parsed > (UINT32_MAX - digit) / 10) return false;
        parsed = parsed * 10 + digit;
        any = true;
        cursor += 1;
    }
    if (!any) return false;
    *value = parsed;
    return true;
}
static bool json_bool(const uint8_t *body, size_t length, const char *key, bool *value) {
    size_t cursor = 0;
    if (!find_key(body, length, key, &cursor, false)) return false;
    if (cursor + 4 <= length && text_is(body + cursor, 4, "true")) { *value = true; return true; }
    if (cursor + 5 <= length && text_is(body + cursor, 5, "false")) { *value = false; return true; }
    return false;
}
static int push(uint8_t *destination, size_t capacity, size_t *written, const uint8_t *bytes, size_t length) {
    size_t i;
    if (*written > capacity || length > capacity - *written) return 4;
    for (i = 0; i < length; ++i) destination[*written + i] = bytes[i];
    *written += length;
    return 0;
}
static int push_text(uint8_t *destination, size_t capacity, size_t *written, const char *text) {
    size_t length = 0;
    while (text[length]) length += 1;
    return push(destination, capacity, written, (const uint8_t *)text, length);
}
static int push_escaped(uint8_t *destination, size_t capacity, size_t *written, const uint8_t *bytes, size_t length) {
    size_t i;
    for (i = 0; i < length; ++i) {
        int status = 0;
        if (bytes[i] == '"') status = push_text(destination, capacity, written, "\\\"");
        else if (bytes[i] == '\\') status = push_text(destination, capacity, written, "\\\\");
        else if (bytes[i] == '\n') status = push_text(destination, capacity, written, "\\n");
        else if (bytes[i] == '\r') status = push_text(destination, capacity, written, "\\r");
        else if (bytes[i] == '\t') status = push_text(destination, capacity, written, "\\t");
        else if (bytes[i] < 32) return 2;
        else status = push(destination, capacity, written, bytes + i, 1);
        if (status) return status;
    }
    return 0;
}
static int push_u64(uint8_t *destination, size_t capacity, size_t *written, uint64_t value) {
    uint8_t digits[20];
    size_t length = 0, i;
    if (!value) return push_text(destination, capacity, written, "0");
    while (value) { digits[length] = (uint8_t)('0' + value % 10); length += 1; value /= 10; }
    for (i = 0; i < length / 2; ++i) {
        uint8_t swap = digits[i];
        digits[i] = digits[length - 1 - i];
        digits[length - 1 - i] = swap;
    }
    return push(destination, capacity, written, digits, length);
}
static int varint(uint8_t *destination, size_t capacity, size_t *written, uint64_t value) {
    do {
        uint8_t byte = (uint8_t)(value & 0x7f);
        value >>= 7;
        if (value) byte = (uint8_t)(byte | 0x80);
        if (push(destination, capacity, written, &byte, 1)) return 4;
    } while (value);
    return 0;
}
int ghostos_inference_model_name(const uint8_t *value, size_t length) {
    size_t i;
    if (!length || length > GHOSTOS_INFERENCE_MODEL) return 2;
    for (i = 0; i < length; ++i) if (!value[i]) return 2;
    return 0;
}
int ghostos_inference_openai_decode(const uint8_t *path, size_t path_length, const uint8_t *body, size_t body_length,
    bool *list_models, ghostos_inference_completion *completion) {
    bool completions = text_is(path, path_length, "/v1/completions");
    bool chat = text_is(path, path_length, "/v1/chat/completions");
    uint32_t max_tokens = 256;
    bool saw_tokens = false, stream = false;
    *list_models = false;
    if (text_is(path, path_length, "/v1/models") && !body_length) { *list_models = true; return 0; }
    if (!completions && !chat) return 1;
    if (!utf8_ok(body, body_length)) return 2;
    if (json_string(body, body_length, "model", false, &completion->model, &completion->model_length)) return 2;
    if (json_string(body, body_length, chat ? "content" : "prompt", chat, &completion->prompt, &completion->prompt_length)) return 2;
    if (json_u32(body, body_length, "max_completion_tokens", &max_tokens)) saw_tokens = true;
    else if (json_u32(body, body_length, "max_tokens", &max_tokens)) saw_tokens = true;
    if (!saw_tokens) max_tokens = 256;
    if (!max_tokens) return 2;
    if (!json_bool(body, body_length, "stream", &stream)) stream = false;
    completion->max_tokens = max_tokens;
    completion->stream = stream;
    completion->kind = chat ? 1 : 0;
    return 0;
}
int ghostos_inference_openai_encode(uint64_t id, const uint8_t *model, size_t model_length, const uint8_t *text,
    size_t text_length, uint8_t kind, uint64_t created_at, uint32_t prompt_tokens, uint32_t completion_tokens,
    bool finished, uint8_t *destination, size_t capacity, size_t *written) {
    int status;
    *written = 0;
    status = push_text(destination, capacity, written, kind == 1 ? "{\"id\":\"chatcmpl-" : "{\"id\":\"cmpl-");
    if (!status) status = push_u64(destination, capacity, written, id);
    if (!status) status = push_text(destination, capacity, written, "\",\"object\":\"");
    if (!status && kind == 0) status = push_text(destination, capacity, written, "text_completion");
    if (!status && kind == 1) status = push_text(destination, capacity, written, finished ? "chat.completion" : "chat.completion.chunk");
    if (!status) status = push_text(destination, capacity, written, "\",\"created\":");
    if (!status) status = push_u64(destination, capacity, written, created_at);
    if (!status) status = push_text(destination, capacity, written, ",\"model\":\"");
    if (!status) status = push_escaped(destination, capacity, written, model, model_length);
    if (status) return status;
    if (finished && kind == 1) {
        status = push_text(destination, capacity, written, "\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"");
        if (!status) status = push_escaped(destination, capacity, written, text, text_length);
        if (!status) status = push_text(destination, capacity, written, "\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":");
    } else if (finished) {
        status = push_text(destination, capacity, written, "\",\"choices\":[{\"index\":0,\"text\":\"");
        if (!status) status = push_escaped(destination, capacity, written, text, text_length);
        if (!status) status = push_text(destination, capacity, written, "\",\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":");
    } else if (kind == 1) {
        status = push_text(destination, capacity, written, "\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"");
        if (!status) status = push_escaped(destination, capacity, written, text, text_length);
        if (!status) status = push_text(destination, capacity, written, "\"},\"finish_reason\":null}]}");
    } else {
        status = push_text(destination, capacity, written, "\",\"choices\":[{\"index\":0,\"text\":\"");
        if (!status) status = push_escaped(destination, capacity, written, text, text_length);
        if (!status) status = push_text(destination, capacity, written, "\",\"finish_reason\":null}]}");
    }
    if (status || !finished) return status;
    if (!status) status = push_u64(destination, capacity, written, prompt_tokens);
    if (!status) status = push_text(destination, capacity, written, ",\"completion_tokens\":");
    if (!status) status = push_u64(destination, capacity, written, completion_tokens);
    if (!status) status = push_text(destination, capacity, written, ",\"total_tokens\":");
    if (!status) status = push_u64(destination, capacity, written, (uint64_t)prompt_tokens + completion_tokens);
    if (!status) status = push_text(destination, capacity, written, "}}");
    return status;
}
int ghostos_inference_grpc_encode(uint64_t id, const uint8_t *model, size_t model_length, const uint8_t *text,
    size_t text_length, uint32_t prompt_tokens, uint32_t completion_tokens, bool finished, uint8_t *destination,
    size_t capacity, size_t *written) {
    size_t message = 0;
    uint8_t finished_byte = finished ? 1 : 0;
    if (capacity < 5) return 4;
    destination[0] = 0;
    if (varint(destination + 5, capacity - 5, &message, 1u << 3) || varint(destination + 5, capacity - 5, &message, id) ||
        varint(destination + 5, capacity - 5, &message, (2u << 3) | 2) || varint(destination + 5, capacity - 5, &message, model_length) ||
        push(destination + 5, capacity - 5, &message, model, model_length) ||
        varint(destination + 5, capacity - 5, &message, (3u << 3) | 2) || varint(destination + 5, capacity - 5, &message, text_length) ||
        push(destination + 5, capacity - 5, &message, text, text_length) ||
        varint(destination + 5, capacity - 5, &message, 4u << 3) || varint(destination + 5, capacity - 5, &message, prompt_tokens) ||
        varint(destination + 5, capacity - 5, &message, 5u << 3) || varint(destination + 5, capacity - 5, &message, completion_tokens) ||
        varint(destination + 5, capacity - 5, &message, 6u << 3) || varint(destination + 5, capacity - 5, &message, finished_byte))
        return 4;
    destination[1] = (uint8_t)(message >> 24);
    destination[2] = (uint8_t)(message >> 16);
    destination[3] = (uint8_t)(message >> 8);
    destination[4] = (uint8_t)message;
    *written = message + 5;
    return 0;
}
int ghostos_inference_grpc_decode(const uint8_t *frame, size_t length, ghostos_inference_completion *completion) {
    uint32_t message_length;
    (void)completion;
    if (length < 5 || frame[0] != 0) return 3;
    message_length = ((uint32_t)frame[1] << 24) | ((uint32_t)frame[2] << 16) | ((uint32_t)frame[3] << 8) | frame[4];
    if (length != (size_t)message_length + 5) return 2;
    return 2;
}
