#include "ghostos/config_network.h"
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
static int integer(const uint8_t *bytes, size_t length, uint64_t *value) {
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
static bool same_name(const uint8_t *left, size_t left_length, const uint8_t *right, size_t right_length) {
    size_t i;
    if (left_length != right_length) return false;
    for (i = 0; i < left_length; ++i) if (left[i] != right[i]) return false;
    return true;
}
static int copy_text(uint8_t *destination, size_t maximum, const uint8_t *text, size_t length, uint8_t *stored) {
    size_t i;
    if (!length || length > maximum) return 11;
    for (i = 0; i < length; ++i) destination[i] = text[i];
    *stored = (uint8_t)length;
    return 0;
}
static int finish_interface(ghostos_config_interface *interfaces, size_t capacity, size_t *count, bool *active,
    bool has_name, bool has_address, uint8_t mode, const uint8_t *name, size_t name_length, const uint8_t *address,
    size_t address_length, bool has_mtu, uint32_t mtu, bool has_enabled, bool enabled) {
    size_t i;
    if (!*active) return 0;
    if (!has_name) return 7;
    if (!has_address && mode == 0) return 7;
    if (!has_mtu) mtu = 1500;
    if (mtu < 576 || mtu > 65535) return 2;
    for (i = 0; i < *count; ++i) if (same_name(interfaces[i].name, interfaces[i].name_length, name, name_length)) return 10;
    if (*count == capacity) return 12;
    if (copy_text(interfaces[*count].name, GHOSTOS_CONFIG_INTERFACE_NAME, name, name_length, &interfaces[*count].name_length)) return 11;
    if (!has_address) {
        const uint8_t fallback[] = {'0', '.', '0', '.', '0', '.', '0'};
        if (copy_text(interfaces[*count].address, GHOSTOS_CONFIG_ADDRESS, fallback, 7, &interfaces[*count].address_length)) return 11;
    } else if (copy_text(interfaces[*count].address, GHOSTOS_CONFIG_ADDRESS, address, address_length, &interfaces[*count].address_length)) return 11;
    interfaces[*count].mtu = mtu;
    interfaces[*count].enabled = has_enabled ? enabled : true;
    interfaces[*count].mode = mode;
    *count += 1;
    *active = false;
    return 0;
}
static int finish_route(ghostos_config_route *routes, size_t capacity, size_t *count, bool *active, bool has_destination,
    bool has_gateway, bool has_interface, const uint8_t *destination, size_t destination_length, const uint8_t *gateway,
    size_t gateway_length, const uint8_t *interface_name, size_t interface_length, bool has_metric, uint32_t metric) {
    if (!*active) return 0;
    if (!has_destination || !has_gateway || !has_interface) return 7;
    if (*count == capacity) return 12;
    if (copy_text(routes[*count].destination, GHOSTOS_CONFIG_ADDRESS, destination, destination_length, &routes[*count].destination_length) ||
        copy_text(routes[*count].gateway, GHOSTOS_CONFIG_ADDRESS, gateway, gateway_length, &routes[*count].gateway_length) ||
        copy_text(routes[*count].interface_name, GHOSTOS_CONFIG_INTERFACE_NAME, interface_name, interface_length, &routes[*count].interface_length))
        return 11;
    routes[*count].metric = has_metric ? metric : 100;
    *count += 1;
    *active = false;
    return 0;
}
int ghostos_config_parse_network(const uint8_t *source, size_t length, uint8_t *hostname, size_t hostname_capacity,
    size_t *hostname_length, ghostos_config_interface *interfaces, size_t interface_capacity, size_t *interface_count,
    ghostos_config_route *routes, size_t route_capacity, size_t *route_count) {
    bool interface_active = false, route_active = false, has_hostname = false;
    bool has_name = false, has_address = false, has_mtu = false, has_enabled = false, enabled = true;
    bool has_destination = false, has_gateway = false, has_route_interface = false, has_metric = false;
    uint8_t section = 0, mode = 0;
    uint8_t name[GHOSTOS_CONFIG_INTERFACE_NAME], address[GHOSTOS_CONFIG_ADDRESS];
    uint8_t destination[GHOSTOS_CONFIG_ADDRESS], gateway[GHOSTOS_CONFIG_ADDRESS], route_interface[GHOSTOS_CONFIG_INTERFACE_NAME];
    size_t name_length = 0, address_length = 0, destination_length = 0, gateway_length = 0, route_interface_length = 0, cursor = 0;
    uint32_t mtu = 1500, metric = 100;
    *hostname_length = 0;
    *interface_count = 0;
    *route_count = 0;
    while (cursor < length) {
        const uint8_t *line = source + cursor;
        size_t line_length = 0;
        while (cursor + line_length < length && source[cursor + line_length] != '\n') ++line_length;
        cursor += line_length + (cursor + line_length < length ? 1 : 0);
        trim(&line, &line_length);
        if (!line_length || line[0] == '#') continue;
        if (line[0] == '[') {
            int status = finish_interface(interfaces, interface_capacity, interface_count, &interface_active, has_name, has_address, mode, name, name_length, address, address_length, has_mtu, mtu, has_enabled, enabled);
            if (!status) status = finish_route(routes, route_capacity, route_count, &route_active, has_destination, has_gateway, has_route_interface, destination, destination_length, gateway, gateway_length, route_interface, route_interface_length, has_metric, metric);
            if (status) return status;
            if (line_length == 9 && text_is(line, line_length, "[network]")) section = 1;
            else if (line_length == 20 && text_is(line, line_length, "[[network.interface]]")) {
                section = 2;
                interface_active = true;
                has_name = has_address = has_mtu = has_enabled = false;
                mode = 0;
                mtu = 1500;
                enabled = true;
            } else if (line_length == 17 && text_is(line, line_length, "[[network.route]]")) {
                section = 3;
                route_active = true;
                has_destination = has_gateway = has_route_interface = has_metric = false;
                metric = 100;
            } else if (line_length >= 2 && line[0] == '[') return 9;
            else return 1;
            continue;
        }
        {
            size_t equals;
            const uint8_t *key, *value, *text;
            size_t key_length, value_length, text_length = 0;
            for (equals = 0; equals < line_length && line[equals] != '='; ++equals) {}
            if (equals == line_length) return 2;
            key = line;
            key_length = equals;
            trim(&key, &key_length);
            value = line + equals + 1;
            value_length = line_length - equals - 1;
            trim(&value, &value_length);
            if (!key_length) return 2;
            if (section == 1) {
                int status;
                if (!text_is(key, key_length, "hostname")) return 3;
                if (has_hostname) return 4;
                status = quoted(value, value_length, GHOSTOS_CONFIG_ADDRESS, &text, &text_length);
                if (status) return status;
                if (text_length > hostname_capacity) return 11;
                {
                    size_t i;
                    for (i = 0; i < text_length; ++i) hostname[i] = text[i];
                }
                *hostname_length = text_length;
                has_hostname = true;
            } else if (section == 2 || section == 3) {
                bool interface_section = section == 2;
                if (interface_section && text_is(key, key_length, "name")) {
                    int status = quoted(value, value_length, GHOSTOS_CONFIG_INTERFACE_NAME, &text, &text_length);
                    size_t i;
                    if (status) return status;
                    if (has_name) return 4;
                    for (i = 0; i < text_length; ++i) name[i] = text[i];
                    name_length = text_length;
                    has_name = true;
                } else if (interface_section && text_is(key, key_length, "address")) {
                    int status = quoted(value, value_length, GHOSTOS_CONFIG_ADDRESS, &text, &text_length);
                    size_t i;
                    if (status) return status;
                    if (has_address) return 4;
                    for (i = 0; i < text_length; ++i) address[i] = text[i];
                    address_length = text_length;
                    has_address = true;
                } else if (interface_section && text_is(key, key_length, "mtu")) {
                    uint64_t parsed = 0;
                    int status = integer(value, value_length, &parsed);
                    if (status) return status;
                    if (parsed > UINT32_MAX) return 5;
                    if (has_mtu) return 4;
                    mtu = (uint32_t)parsed;
                    has_mtu = true;
                } else if (section == 3 && (text_is(key, key_length, "destination") || text_is(key, key_length, "gateway") || text_is(key, key_length, "interface"))) {
                    int status = quoted(value, value_length, text_is(key, key_length, "interface") ? GHOSTOS_CONFIG_INTERFACE_NAME : GHOSTOS_CONFIG_ADDRESS, &text, &text_length);
                    uint8_t *slot = text_is(key, key_length, "destination") ? destination : text_is(key, key_length, "gateway") ? gateway : route_interface;
                    size_t *slot_length = text_is(key, key_length, "destination") ? &destination_length : text_is(key, key_length, "gateway") ? &gateway_length : &route_interface_length;
                    bool *seen = text_is(key, key_length, "destination") ? &has_destination : text_is(key, key_length, "gateway") ? &has_gateway : &has_route_interface;
                    size_t i;
                    if (status) return status;
                    if (*seen) return 4;
                    for (i = 0; i < text_length; ++i) slot[i] = text[i];
                    *slot_length = text_length;
                    *seen = true;
                } else return 3;
            } else return 3;
        }
    }
    {
        int status = finish_interface(interfaces, interface_capacity, interface_count, &interface_active, has_name, has_address, mode, name, name_length, address, address_length, has_mtu, mtu, has_enabled, enabled);
        size_t i;
        if (!status) status = finish_route(routes, route_capacity, route_count, &route_active, has_destination, has_gateway, has_route_interface, destination, destination_length, gateway, gateway_length, route_interface, route_interface_length, has_metric, metric);
        if (status) return status;
        for (i = 0; i < *route_count; ++i) {
            size_t j;
            bool found = false;
            for (j = 0; j < *interface_count; ++j) {
                if (same_name(interfaces[j].name, interfaces[j].name_length, routes[i].interface_name, routes[i].interface_length)) found = true;
            }
            if (!found) return 2;
        }
    }
    return 0;
}
