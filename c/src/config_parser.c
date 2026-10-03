#include "ghostos/config_parser.h"
#include <stdbool.h>
#include <stdint.h>
static bool space(uint8_t byte) { return byte == ' ' || byte == '\t' || byte == '\r'; }
static void trim(const uint8_t **bytes, size_t *length) {
    while (*length && space((*bytes)[0])) { *bytes += 1; *length -= 1; }
    while (*length && space((*bytes)[*length - 1])) *length -= 1;
}
static void strip_comment(const uint8_t **bytes, size_t *length) {
    uint8_t quote = 0;
    size_t i;
    for (i = 0; i < *length; ++i) {
        uint8_t byte = (*bytes)[i];
        if (!quote && (byte == '"' || byte == '\'')) quote = byte;
        else if (quote && byte == quote) quote = 0;
        else if (!quote && byte == '#') { *length = i; return; }
    }
}
static int parse_u64(const uint8_t *bytes, size_t length, uint64_t *value) {
    size_t i = 0;
    int base = 10;
    *value = 0;
    if (length >= 2 && bytes[0] == '0' && bytes[1] == 'x') { base = 16; i = 2; if (length == 2) return 5; }
    if (i == length) return 5;
    for (; i < length; ++i) {
        int digit;
        if (bytes[i] >= '0' && bytes[i] <= '9') digit = bytes[i] - '0';
        else if (base == 16 && bytes[i] >= 'a' && bytes[i] <= 'f') digit = bytes[i] - 'a' + 10;
        else if (base == 16 && bytes[i] >= 'A' && bytes[i] <= 'F') digit = bytes[i] - 'A' + 10;
        else return 5;
        if (*value > (UINT64_MAX - (uint64_t)digit) / (uint64_t)base) return 5;
        *value = *value * (uint64_t)base + (uint64_t)digit;
    }
    return 0;
}
static int section_name(const uint8_t *line, size_t length, const uint8_t **name, size_t *name_length, bool *array) {
    if (length >= 4 && line[0] == '[' && line[1] == '[' && line[length - 2] == ']' && line[length - 1] == ']') {
        *array = true;
        *name = line + 2;
        *name_length = length - 4;
        return 0;
    }
    if (length >= 2 && line[0] == '[' && line[length - 1] == ']') {
        *array = false;
        *name = line + 1;
        *name_length = length - 2;
        return 0;
    }
    return 1;
}
static bool text_is(const uint8_t *bytes, size_t length, const char *text) {
    size_t i;
    for (i = 0; text[i]; ++i) if (i == length || bytes[i] != (uint8_t)text[i]) return false;
    return i == length;
}
static int known_section(const uint8_t *name, size_t length, bool array, bool *system) {
    *system = false;
    if (!array && text_is(name, length, "system")) { *system = true; return 0; }
    if (!array && (text_is(name, length, "network") || text_is(name, length, "cluster") ||
        text_is(name, length, "cluster.quorum") || text_is(name, length, "cluster.security") ||
        text_is(name, length, "cluster.resources") || text_is(name, length, "cluster.federation"))) return 9;
    if (array && (text_is(name, length, "services") || text_is(name, length, "service") ||
        text_is(name, length, "capabilities") || text_is(name, length, "capability") ||
        text_is(name, length, "network.interfaces") || text_is(name, length, "network.interface") ||
        text_is(name, length, "network.routes") || text_is(name, length, "network.route") ||
        text_is(name, length, "cluster.transports") || text_is(name, length, "cluster.transport") ||
        text_is(name, length, "cluster.node_overrides") || text_is(name, length, "cluster.node-override") ||
        text_is(name, length, "cluster.node_override"))) return 9;
    return 1;
}
int ghostos_config_parse_system(const uint8_t *source, size_t length, uint16_t *schema, uint64_t *revision) {
    bool has_schema = false;
    bool has_revision = false;
    bool in_system = true;
    uint64_t schema_value = 0;
    uint64_t revision_value = 0;
    size_t cursor = 0;
    while (cursor < length) {
        const uint8_t *line = source + cursor;
        size_t line_length = 0;
        const uint8_t *name;
        size_t name_length = 0;
        bool array = false;
        size_t equals;
        while (cursor + line_length < length && source[cursor + line_length] != '\n') ++line_length;
        cursor += line_length + (cursor + line_length < length ? 1 : 0);
        strip_comment(&line, &line_length);
        trim(&line, &line_length);
        if (!line_length) continue;
        if (line[0] == '[') {
            int status = section_name(line, line_length, &name, &name_length, &array);
            bool system = false;
            if (status) return status;
            status = known_section(name, name_length, array, &system);
            if (status) return status;
            in_system = system;
            continue;
        }
        if (!in_system) return 3;
        for (equals = 0; equals < line_length && line[equals] != '='; ++equals) {}
        if (equals == line_length) return 2;
        name = line;
        name_length = equals;
        trim(&name, &name_length);
        if (!name_length) return 2;
        line += equals + 1;
        line_length -= equals + 1;
        trim(&line, &line_length);
        if (text_is(name, name_length, "schema") || text_is(name, name_length, "revision")) {
            uint64_t value = 0;
            int status = parse_u64(line, line_length, &value);
            bool schema_key = text_is(name, name_length, "schema");
            if (status) return status;
            if (schema_key && value > UINT16_MAX) return 5;
            if (schema_key) {
                if (has_schema) return 4;
                has_schema = true;
                schema_value = value;
            } else {
                if (has_revision) return 4;
                has_revision = true;
                revision_value = value;
            }
        } else return 3;
    }
    if (!has_schema || !has_revision) return 7;
    if (schema_value != 1) return 6;
    if (!revision_value) return 2;
    *schema = (uint16_t)schema_value;
    *revision = revision_value;
    return 0;
}
static int quoted_name(const uint8_t *bytes, size_t length, const uint8_t **text, size_t *text_length) {
    size_t i;
    if (length < 2) return 11;
    if (bytes[0] == '"' && bytes[length - 1] == '"') {
        for (i = 1; i + 1 < length; ++i) if (bytes[i] == '\\') return 11;
    } else if (!(bytes[0] == '\'' && bytes[length - 1] == '\'')) return 11;
    *text = bytes + 1;
    *text_length = length - 2;
    if (!*text_length || *text_length > GHOSTOS_CONFIG_SERVICE_NAME) return 11;
    for (i = 0; i < *text_length; ++i) if (!(*text)[i]) return 11;
    return 0;
}
static int unquoted_word(const uint8_t *bytes, size_t length, const uint8_t **text, size_t *text_length) {
    if (length >= 2 && ((bytes[0] == '"' && bytes[length - 1] == '"') || (bytes[0] == '\'' && bytes[length - 1] == '\''))) {
        *text = bytes + 1;
        *text_length = length - 2;
    } else {
        *text = bytes;
        *text_length = length;
    }
    return 0;
}
static int finish_service(ghostos_config_service *services, size_t capacity, size_t *count, bool *active,
    bool has_name, bool has_image, bool has_kind, bool has_enabled, bool has_restart, const uint8_t *name,
    size_t name_length, uint64_t image, uint8_t kind, uint8_t restart, bool enabled) {
    size_t i, j;
    if (!*active) return 0;
    if (!has_name || !has_image || !has_kind) return 7;
    if (!image) return 2;
    for (i = 0; i < *count; ++i) {
        if (services[i].name_length != name_length) continue;
        for (j = 0; j < name_length && services[i].name[j] == name[j]; ++j) {}
        if (j == name_length) return 10;
    }
    if (*count == capacity) return 12;
    for (i = 0; i < name_length; ++i) services[*count].name[i] = name[i];
    services[*count].name_length = (uint8_t)name_length;
    services[*count].image = image;
    services[*count].kind = kind;
    services[*count].enabled = has_enabled ? enabled : true;
    services[*count].restart = has_restart ? restart : 0;
    *count += 1;
    *active = false;
    return 0;
}
int ghostos_config_parse_services(const uint8_t *source, size_t length, uint16_t *schema, uint64_t *revision,
    ghostos_config_service *services, size_t capacity, size_t *count) {
    bool has_schema = false, has_revision = false, in_service = false, active = false;
    bool has_name = false, has_image = false, has_kind = false, has_enabled = false, has_restart = false, enabled = true;
    uint64_t schema_value = 0, revision_value = 0, image = 0;
    uint8_t name[GHOSTOS_CONFIG_SERVICE_NAME], kind = 0, restart = 0;
    size_t name_length = 0, cursor = 0;
    *count = 0;
    while (cursor < length) {
        const uint8_t *line = source + cursor;
        size_t line_length = 0, equals, key_length, value_length;
        const uint8_t *key, *value, *word;
        size_t word_length = 0;
        while (cursor + line_length < length && source[cursor + line_length] != '\n') ++line_length;
        cursor += line_length + (cursor + line_length < length ? 1 : 0);
        strip_comment(&line, &line_length);
        trim(&line, &line_length);
        if (!line_length) continue;
        if (line[0] == '[') {
            const uint8_t *section;
            size_t section_length = 0;
            bool array = false;
            int status = finish_service(services, capacity, count, &active, has_name, has_image, has_kind, has_enabled, has_restart, name, name_length, image, kind, restart, enabled);
            if (status) return status;
            if (section_name(line, line_length, &section, &section_length, &array)) return 1;
            if (!array && text_is(section, section_length, "system")) { in_service = false; continue; }
            if (array && (text_is(section, section_length, "service") || text_is(section, section_length, "services"))) {
                in_service = true;
                active = true;
                has_name = has_image = has_kind = has_enabled = has_restart = false;
                enabled = true;
                restart = 0;
                image = 0;
                name_length = 0;
                continue;
            }
            return known_section(section, section_length, array, &array) == 1 ? 1 : 9;
        }
        for (equals = 0; equals < line_length && line[equals] != '='; ++equals) {}
        if (equals == line_length) return 2;
        key = line;
        key_length = equals;
        trim(&key, &key_length);
        if (!key_length) return 2;
        value = line + equals + 1;
        value_length = line_length - equals - 1;
        trim(&value, &value_length);
        if (!in_service) {
            uint64_t parsed = 0;
            int status;
            if (!text_is(key, key_length, "schema") && !text_is(key, key_length, "revision")) return 3;
            status = parse_u64(value, value_length, &parsed);
            if (status) return status;
            if (text_is(key, key_length, "schema")) {
                if (parsed > UINT16_MAX) return 5;
                if (has_schema) return 4;
                has_schema = true;
                schema_value = parsed;
            } else {
                if (has_revision) return 4;
                has_revision = true;
                revision_value = parsed;
            }
            continue;
        }
        if (text_is(key, key_length, "name")) {
            const uint8_t *text;
            size_t text_length = 0;
            int status = quoted_name(value, value_length, &text, &text_length);
            size_t i;
            if (status) return status;
            if (has_name) return 4;
            for (i = 0; i < text_length; ++i) name[i] = text[i];
            name_length = text_length;
            has_name = true;
        } else if (text_is(key, key_length, "image")) {
            int status = parse_u64(value, value_length, &image);
            if (status) return status;
            if (has_image) return 4;
            has_image = true;
        } else if (text_is(key, key_length, "kind") || text_is(key, key_length, "restart")) {
            bool kind_key = text_is(key, key_length, "kind");
            uint8_t parsed;
            unquoted_word(value, value_length, &word, &word_length);
            if (kind_key && text_is(word, word_length, "system")) parsed = 0;
            else if (kind_key && text_is(word, word_length, "network")) parsed = 1;
            else if (kind_key && text_is(word, word_length, "storage")) parsed = 2;
            else if (kind_key && text_is(word, word_length, "compute")) parsed = 3;
            else if (!kind_key && text_is(word, word_length, "never")) parsed = 0;
            else if (!kind_key && text_is(word, word_length, "on-failure")) parsed = 1;
            else if (!kind_key && text_is(word, word_length, "always")) parsed = 2;
            else return 2;
            if (kind_key) {
                if (has_kind) return 4;
                kind = parsed;
                has_kind = true;
            } else {
                if (has_restart) return 4;
                restart = parsed;
                has_restart = true;
            }
        } else if (text_is(key, key_length, "enabled")) {
            if (text_is(value, value_length, "true")) enabled = true;
            else if (text_is(value, value_length, "false")) enabled = false;
            else return 8;
            if (has_enabled) return 4;
            has_enabled = true;
        } else return 3;
    }
    {
        int status = finish_service(services, capacity, count, &active, has_name, has_image, has_kind, has_enabled, has_restart, name, name_length, image, kind, restart, enabled);
        if (status) return status;
    }
    if (!has_schema || !has_revision) return 7;
    if (schema_value != 1) return 6;
    if (!revision_value) return 2;
    *schema = (uint16_t)schema_value;
    *revision = revision_value;
    return 0;
}
