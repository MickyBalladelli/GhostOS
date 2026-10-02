#include "ghostos/vm_dhcp.h"
#include "ghostos/vm_net.h"
#include <string.h>

uint32_t ghostos_vm_dhcp_validate_config(const ghostos_vm_dhcp_config *config,
    const ghostos_vm_dhcp_reservation *reservations, size_t count) {
    uint32_t start = ghostos_vm_net_ipv4_to_number(config->pool_start);
    uint32_t end = ghostos_vm_net_ipv4_to_number(config->pool_end);
    if (start > end || !start || end == UINT32_MAX) return 1;
    if ((uint64_t)end - start + 1 > 256) return 2;
    if (config->lease_time_secs < 3) return 3;
    if (count > 32) return 4;
    for (size_t i = 0; i < count; ++i) {
        uint32_t address = ghostos_vm_net_ipv4_to_number(reservations[i].address);
        if (address < start || address > end) return 6;
        for (size_t j = i + 1; j < count; ++j)
            if (!memcmp(reservations[i].mac, reservations[j].mac, 6) ||
                !memcmp(reservations[i].address, reservations[j].address, 4)) return 5;
    }
    return 0;
}

static void write_be(uint8_t *bytes, uint32_t value, unsigned count) {
    for (unsigned i = 0; i < count; ++i) bytes[count - 1 - i] = (uint8_t)(value >> (i * 8));
}

static size_t reply_option(uint8_t bytes[576], size_t cursor, uint8_t code,
    const uint8_t *value, size_t length) {
    size_t next = cursor;
    (void)ghostos_vm_dhcp_write_option(bytes, 576, cursor, code, value, length, &next);
    return next;
}

bool ghostos_vm_dhcp_encode_reply(const ghostos_vm_dhcp_config *config,
    const ghostos_vm_dhcp_request *request, uint8_t message_type,
    const uint8_t address[4], uint8_t *output, size_t capacity, size_t *length) {
    *length = 0;
    uint8_t dhcp[576] = {0};
    dhcp[0] = 2; dhcp[1] = 1; dhcp[2] = 6;
    write_be(dhcp + 4, request->xid, 4);
    write_be(dhcp + 10, request->flags, 2);
    memcpy(dhcp + 16, address, 4);
    memcpy(dhcp + 20, config->server_ip, 4);
    memcpy(dhcp + 28, request->mac, 6);
    write_be(dhcp + 236, UINT32_C(0x63825363), 4);
    size_t cursor = reply_option(dhcp, 240, 53, &message_type, 1);
    if (message_type != 6) {
        cursor = reply_option(dhcp, cursor, 1, config->subnet_mask, 4);
        cursor = reply_option(dhcp, cursor, 3, config->gateway, 4);
        cursor = reply_option(dhcp, cursor, 6, config->dns, 4);
        uint8_t value[4];
        write_be(value, config->lease_time_secs, 4);
        cursor = reply_option(dhcp, cursor, 51, value, 4);
        uint32_t t1 = config->lease_time_secs / 2;
        if (!t1) t1 = 1;
        uint32_t multiplied = config->lease_time_secs > UINT32_MAX / 7 ? UINT32_MAX : config->lease_time_secs * 7;
        uint32_t t2 = multiplied / 8;
        if (t2 < t1 + 1) t2 = t1 + 1;
        uint32_t upper = config->lease_time_secs ? config->lease_time_secs - 1 : 0;
        if (t2 > upper) t2 = upper;
        write_be(value, t1, 4);
        cursor = reply_option(dhcp, cursor, 58, value, 4);
        write_be(value, t2, 4);
        cursor = reply_option(dhcp, cursor, 59, value, 4);
    }
    cursor = reply_option(dhcp, cursor, 54, config->server_ip, 4);
    dhcp[cursor] = 255;
    size_t dhcp_length = cursor + 1, udp_length = 8 + dhcp_length;
    size_t frame_length = 14 + 20 + udp_length;
    if (capacity < frame_length) return false;
    memset(output, 0, frame_length);
    memset(output, 0xff, 6);
    const uint8_t server_mac[6] = {0x52, 0x54, 0, 0x12, 0x34, 0xd0};
    memcpy(output + 6, server_mac, 6);
    output[12] = 8;
    uint8_t *ip = output + 14, *udp = ip + 20;
    ip[0] = 0x45;
    write_be(ip + 2, (uint32_t)(20 + udp_length), 2);
    write_be(ip + 6, 0x4000, 2);
    ip[8] = 64; ip[9] = 17;
    memcpy(ip + 12, config->server_ip, 4);
    memset(ip + 16, 255, 4);
    write_be(ip + 10, ghostos_vm_net_checksum(ip, 20), 2);
    write_be(udp, 67, 2); write_be(udp + 2, 68, 2);
    write_be(udp + 4, (uint32_t)udp_length, 2);
    memcpy(udp + 8, dhcp, dhcp_length);
    uint8_t pseudo[12 + 8 + 576] = {0};
    memcpy(pseudo, config->server_ip, 4);
    memset(pseudo + 4, 255, 4);
    pseudo[9] = 17;
    write_be(pseudo + 10, (uint32_t)udp_length, 2);
    memcpy(pseudo + 12, udp, udp_length);
    write_be(udp + 6, ghostos_vm_net_checksum(pseudo, 12 + udp_length), 2);
    *length = frame_length;
    return true;
}

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
