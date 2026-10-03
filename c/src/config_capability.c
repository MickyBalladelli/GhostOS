#include "ghostos/config_capability.h"
static bool space(uint8_t byte) { return byte == ' ' || byte == '\t' || byte == '\r'; }
static void trim(const uint8_t **bytes, size_t *length) {
    while (*length && space((*bytes)[0])) { *bytes += 1; *length -= 1; }
    while (*length && space((*bytes)[*length - 1])) *length -= 1;
}
static bool text_is(const uint8_t *bytes, size_t length, const char *text) {
    size_t i;
    for (i = 0; text[i]; ++i) if (i == length || bytes[i] != (uint8_t)text[i]) return false;
    return i == length;
}
static bool section_name(const uint8_t *line, size_t length, const char *name) {
    size_t name_length = 0, i;
    while (name[name_length]) name_length += 1;
    if (length != name_length + 4 || line[0] != '[' || line[1] != '[' || line[length - 2] != ']' || line[length - 1] != ']') return false;
    for (i = 0; i < name_length; ++i) if (line[i + 2] != (uint8_t)name[i]) return false;
    return true;
}
static void word(const uint8_t *bytes, size_t length, const uint8_t **text, size_t *text_length) {
    if (length >= 2 && ((bytes[0] == '"' && bytes[length - 1] == '"') || (bytes[0] == '\'' && bytes[length - 1] == '\''))) {
        *text = bytes + 1;
        *text_length = length - 2;
    } else {
        *text = bytes;
        *text_length = length;
    }
}
static int quoted(const uint8_t *bytes, size_t length, size_t maximum, const uint8_t **text, size_t *text_length) {
    size_t i;
    if (length < 2) return 11;
    if (bytes[0] == '"' && bytes[length - 1] == '"') {
        for (i = 1; i + 1 < length; ++i) if (bytes[i] == '\\') return 11;
    } else if (!(bytes[0] == '\'' && bytes[length - 1] == '\'')) return 11;
    *text = bytes + 1;
    *text_length = length - 2;
    if (!*text_length || *text_length > maximum) return 11;
    for (i = 0; i < *text_length; ++i) if (!(*text)[i]) return 11;
    return 0;
}
static int copy_text(uint8_t *destination, size_t maximum, const uint8_t *text, size_t length, uint8_t *stored) {
    size_t i;
    if (!length || length > maximum) return 11;
    for (i = 0; i < length; ++i) destination[i] = text[i];
    *stored = (uint8_t)length;
    return 0;
}
static bool same_name(const uint8_t *left, size_t left_length, const uint8_t *right, size_t right_length) {
    size_t i;
    if (left_length != right_length) return false;
    for (i = 0; i < left_length; ++i) if (left[i] != right[i]) return false;
    return true;
}
static int boolean(const uint8_t *bytes, size_t length, bool *value) {
    if (text_is(bytes, length, "true")) { *value = true; return 0; }
    if (text_is(bytes, length, "false")) { *value = false; return 0; }
    return 8;
}
static int right_bit(const uint8_t *bytes, size_t length, uint16_t *bit) {
    const uint8_t *text;
    size_t text_length;
    word(bytes, length, &text, &text_length);
    if (text_is(text, text_length, "read")) *bit = 1;
    else if (text_is(text, text_length, "write")) *bit = 2;
    else if (text_is(text, text_length, "execute")) *bit = 4;
    else if (text_is(text, text_length, "map")) *bit = 8;
    else if (text_is(text, text_length, "bind")) *bit = 16;
    else if (text_is(text, text_length, "connect")) *bit = 32;
    else if (text_is(text, text_length, "send")) *bit = 64;
    else if (text_is(text, text_length, "receive")) *bit = 128;
    else if (text_is(text, text_length, "admin")) *bit = 256;
    else return 2;
    return 0;
}
static int add_right(uint16_t *rights, const uint8_t *item, size_t length) {
    uint16_t bit = 0;
    int status;
    trim(&item, &length);
    if (!length) return 13;
    status = right_bit(item, length, &bit);
    if (status) return status;
    *rights = (uint16_t)(*rights | bit);
    return 0;
}
static int parse_rights(const uint8_t *bytes, size_t length, uint16_t *rights) {
    uint16_t value = 0;
    trim(&bytes, &length);
    if (length && bytes[0] == '[') {
        size_t cursor = 1;
        if (bytes[length - 1] != ']') return 13;
        if (length == 2) return 13;
        while (cursor + 1 < length) {
            size_t end = cursor;
            int status;
            while (end + 1 < length && bytes[end] != ',') end += 1;
            status = add_right(&value, bytes + cursor, end - cursor);
            if (status) return status;
            if (bytes[end] != ',') break;
            cursor = end + 1;
            if (cursor + 1 >= length) return 13;
        }
    } else {
        const uint8_t *text;
        size_t text_length, cursor = 0;
        word(bytes, length, &text, &text_length);
        if (!text_length) return 2;
        while (cursor <= text_length) {
            size_t end = cursor;
            int status;
            while (end < text_length && text[end] != '|') end += 1;
            status = add_right(&value, text + cursor, end - cursor);
            if (status) return status;
            if (end == text_length) break;
            cursor = end + 1;
        }
    }
    if (!value) return 2;
    *rights = value;
    return 0;
}
static int kind_value(const uint8_t *bytes, size_t length, uint8_t *kind) {
    const uint8_t *text;
    size_t text_length;
    word(bytes, length, &text, &text_length);
    if (text_is(text, text_length, "ipc")) *kind = 0;
    else if (text_is(text, text_length, "file")) *kind = 1;
    else if (text_is(text, text_length, "network")) *kind = 2;
    else if (text_is(text, text_length, "memory")) *kind = 3;
    else if (text_is(text, text_length, "device")) *kind = 4;
    else if (text_is(text, text_length, "clock")) *kind = 5;
    else return 2;
    return 0;
}
static int finish(ghostos_config_capability *capabilities, size_t capacity, size_t *count, bool *active,
    bool has_service, bool has_resource, bool has_kind, bool has_rights, bool has_required,
    const uint8_t *service, size_t service_length, const uint8_t *resource, size_t resource_length,
    uint8_t kind, uint16_t rights, bool required) {
    if (!*active) return 0;
    if (!has_service || !has_resource || !has_kind || !has_rights) return 7;
    if (!rights) return 2;
    if (*count == capacity) return 12;
    if (copy_text(capabilities[*count].service, GHOSTOS_CONFIG_CAPABILITY_SERVICE, service, service_length, &capabilities[*count].service_length) ||
        copy_text(capabilities[*count].resource, GHOSTOS_CONFIG_CAPABILITY_RESOURCE, resource, resource_length, &capabilities[*count].resource_length))
        return 11;
    capabilities[*count].kind = kind;
    capabilities[*count].rights = rights;
    capabilities[*count].required = has_required ? required : true;
    *count += 1;
    *active = false;
    return 0;
}
int ghostos_config_parse_capabilities(const uint8_t *source, size_t length, const uint8_t *const *service_names,
    const uint8_t *service_lengths, size_t service_count, ghostos_config_capability *capabilities, size_t capacity,
    size_t *count) {
    bool active = false, has_service = false, has_resource = false, has_kind = false, has_rights = false, has_required = false, required = true;
    uint8_t service[GHOSTOS_CONFIG_CAPABILITY_SERVICE], resource[GHOSTOS_CONFIG_CAPABILITY_RESOURCE], kind = 0;
    size_t service_length = 0, resource_length = 0, cursor = 0, policy;
    uint16_t rights = 0;
    *count = 0;
    while (cursor < length) {
        const uint8_t *line = source + cursor;
        size_t line_length = 0;
        int status;
        while (cursor + line_length < length && source[cursor + line_length] != '\n') line_length += 1;
        cursor += line_length + (cursor + line_length < length ? 1 : 0);
        trim(&line, &line_length);
        if (!line_length || line[0] == '#') continue;
        if (line[0] == '[') {
            status = finish(capabilities, capacity, count, &active, has_service, has_resource, has_kind, has_rights, has_required,
                service, service_length, resource, resource_length, kind, rights, required);
            if (status) return status;
            if (section_name(line, line_length, "capability") || section_name(line, line_length, "capabilities")) {
                active = true;
                has_service = has_resource = has_kind = has_rights = has_required = false;
                required = true;
                rights = 0;
            } else return 1;
            continue;
        }
        {
            size_t equals;
            const uint8_t *key, *value, *text;
            size_t key_length, value_length, text_length = 0;
            for (equals = 0; equals < line_length && line[equals] != '='; ++equals) {}
            if (equals == line_length) return 2;
            key = line; key_length = equals; trim(&key, &key_length);
            value = line + equals + 1; value_length = line_length - equals - 1; trim(&value, &value_length);
            if (!key_length || !active) return !active ? 3 : 2;
            if (text_is(key, key_length, "service")) {
                if (has_service) return 4;
                status = quoted(value, value_length, GHOSTOS_CONFIG_CAPABILITY_SERVICE, &text, &text_length);
                if (status) return status;
                { size_t i; for (i = 0; i < text_length; ++i) service[i] = text[i]; }
                service_length = text_length; has_service = true;
            } else if (text_is(key, key_length, "resource")) {
                if (has_resource) return 4;
                status = quoted(value, value_length, GHOSTOS_CONFIG_CAPABILITY_RESOURCE, &text, &text_length);
                if (status) return status;
                { size_t i; for (i = 0; i < text_length; ++i) resource[i] = text[i]; }
                resource_length = text_length; has_resource = true;
            } else if (text_is(key, key_length, "kind")) {
                if (has_kind) return 4;
                status = kind_value(value, value_length, &kind);
                if (status) return status;
                has_kind = true;
            } else if (text_is(key, key_length, "rights")) {
                if (has_rights) return 4;
                status = parse_rights(value, value_length, &rights);
                if (status) return status;
                has_rights = true;
            } else if (text_is(key, key_length, "required")) {
                if (has_required) return 4;
                status = boolean(value, value_length, &required);
                if (status) return status;
                has_required = true;
            } else return 3;
        }
    }
    {
        int status = finish(capabilities, capacity, count, &active, has_service, has_resource, has_kind, has_rights, has_required,
            service, service_length, resource, resource_length, kind, rights, required);
        if (status) return status;
    }
    for (policy = 0; policy < *count; ++policy) {
        size_t service_index;
        bool found = false;
        for (service_index = 0; service_index < service_count; ++service_index) {
            if (same_name(service_names[service_index], service_lengths[service_index], capabilities[policy].service, capabilities[policy].service_length))
                found = true;
        }
        if (!found) return 2;
    }
    return 0;
}
