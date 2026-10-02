#include "ghostos/vm_dhcp.h"

enum { ethernet_header_length = 14, ipv4_minimum_header_length = 20,
    udp_header_length = 8, dhcp_fixed_length = 240,
    dhcp_client_port = 68, dhcp_server_port = 67,
    dhcp_magic_cookie = 0x63825363, dhcp_option_end = 255,
    dhcp_option_message_type = 53, dhcp_option_requested_ip = 50,
    dhcp_option_server_id = 54 };

static uint16_t read_u16_be(const uint8_t *bytes) {
    return (uint16_t)(((uint16_t)bytes[0] << 8) | bytes[1]);
}

static uint32_t read_u32_be(const uint8_t *bytes) {
    return ((uint32_t)bytes[0] << 24) | ((uint32_t)bytes[1] << 16) |
        ((uint32_t)bytes[2] << 8) | (uint32_t)bytes[3];
}

bool ghostos_vm_dhcp_write_option(uint8_t *output, size_t output_capacity,
    size_t cursor, uint8_t code, const uint8_t *value, size_t value_length,
    size_t *output_cursor) {
    if (!output || !output_cursor || (!value && value_length) || value_length > UINT8_MAX ||
        cursor > output_capacity || output_capacity - cursor < 2 ||
        value_length > output_capacity - cursor - 2) return false;
    output[cursor] = code;
    output[cursor + 1] = (uint8_t)value_length;
    for (size_t index = 0; index < value_length; ++index)
        output[cursor + 2 + index] = value[index];
    *output_cursor = cursor + 2 + value_length;
    return true;
}

bool ghostos_vm_dhcp_decode_request(const uint8_t *frame, size_t frame_length,
    ghostos_vm_dhcp_request *request) {
    if (!frame || !request || frame_length < ethernet_header_length +
        ipv4_minimum_header_length + udp_header_length + dhcp_fixed_length) return false;
    if (frame[12] != 0x08 || frame[13] != 0x00) return false;
    size_t ip = ethernet_header_length;
    if ((frame[ip] >> 4) != 4 || (frame[ip] & 0x0f) < 5) return false;
    size_t ip_header_length = (size_t)(frame[ip] & 0x0f) * 4;
    if (frame_length < ip + ip_header_length + udp_header_length || frame[ip + 9] != 17)
        return false;
    size_t total_length = read_u16_be(frame + ip + 2);
    if (total_length < ip_header_length + udp_header_length || total_length > frame_length - ip)
        return false;
    size_t udp = ip + ip_header_length;
    if (read_u16_be(frame + udp) != dhcp_client_port ||
        read_u16_be(frame + udp + 2) != dhcp_server_port) return false;
    size_t udp_length = read_u16_be(frame + udp + 4);
    if (udp_length < udp_header_length + dhcp_fixed_length ||
        udp_length > ip + total_length - udp) return false;
    const uint8_t *payload = frame + udp + udp_header_length;
    size_t payload_length = udp_length - udp_header_length;
    if (payload[0] != 1 || payload[1] != 1 || payload[2] != 6 ||
        read_u32_be(payload + 236) != dhcp_magic_cookie) return false;

    ghostos_vm_dhcp_request decoded = {0};
    for (size_t index = 0; index < 6; ++index) {
        decoded.source_mac[index] = frame[6 + index];
        decoded.mac[index] = payload[28 + index];
    }
    decoded.xid = read_u32_be(payload + 4);
    decoded.flags = read_u16_be(payload + 10);
    for (size_t index = 0; index < 4; ++index) decoded.ciaddr[index] = payload[12 + index];

    size_t cursor = dhcp_fixed_length;
    while (cursor < payload_length) {
        uint8_t code = payload[cursor++];
        if (code == dhcp_option_end) break;
        if (code == 0) continue;
        if (cursor >= payload_length) return false;
        size_t option_length = payload[cursor++];
        if (option_length > payload_length - cursor) return false;
        if (code == dhcp_option_message_type && option_length == 1)
            decoded.message_type = payload[cursor];
        else if (code == dhcp_option_requested_ip && option_length == 4) {
            decoded.requested_ip_present = 1;
            for (size_t index = 0; index < 4; ++index)
                decoded.requested_ip[index] = payload[cursor + index];
        } else if (code == dhcp_option_server_id && option_length == 4) {
            decoded.server_id_present = 1;
            for (size_t index = 0; index < 4; ++index)
                decoded.server_id[index] = payload[cursor + index];
        }
        cursor += option_length;
    }
    if (decoded.message_type == 0) return false;
    *request = decoded;
    return true;
}
