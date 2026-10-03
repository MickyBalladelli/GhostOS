#include "ghostos/config_parser.h"
#include <stdbool.h>
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
