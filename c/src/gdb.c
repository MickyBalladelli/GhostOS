#include "ghostos/gdb.h"
enum { GHOSTOS_GDB_IDLE = 0, GHOSTOS_GDB_PAYLOAD = 1, GHOSTOS_GDB_HIGH = 2, GHOSTOS_GDB_LOW = 3 };
static uint8_t hex_digit(uint8_t value) {
    value &= 0x0fu;
    return (uint8_t)(value < 10 ? '0' + value : 'a' + value - 10);
}
static int decode_hex(uint8_t byte, uint8_t *value) {
    if (byte >= '0' && byte <= '9') { *value = (uint8_t)(byte - '0'); return 0; }
    if (byte >= 'a' && byte <= 'f') { *value = (uint8_t)(byte - 'a' + 10); return 0; }
    if (byte >= 'A' && byte <= 'F') { *value = (uint8_t)(byte - 'A' + 10); return 0; }
    return -2;
}
static int reply(uint8_t *output, size_t output_len, const uint8_t *payload, size_t payload_len,
    const uint8_t *sum_bytes, size_t sum_len) {
    size_t required = payload_len + 5;
    uint8_t checksum = 0;
    size_t i;
    if (output_len < required) return -1;
    output[0] = '+';
    output[1] = '$';
    for (i = 0; i < payload_len; ++i) output[2 + i] = payload[i];
    output[2 + payload_len] = '#';
    for (i = 0; i < sum_len; ++i) checksum = (uint8_t)(checksum + sum_bytes[i]);
    output[3 + payload_len] = hex_digit((uint8_t)(checksum >> 4));
    output[4 + payload_len] = hex_digit(checksum);
    return (int)required;
}
static int error_reply(uint8_t code, uint8_t *output, size_t output_len) {
    uint8_t payload[3] = {'E', hex_digit((uint8_t)(code >> 4)), hex_digit(code)};
    return reply(output, output_len, payload, 3, payload, 3);
}
static int parse_hex(const uint8_t *bytes, size_t length, uint64_t *value) {
    size_t i;
    *value = 0;
    if (!length) return -3;
    for (i = 0; i < length; ++i) {
        uint8_t digit = 0;
        if (decode_hex(bytes[i], &digit) || *value > (UINT64_MAX - digit) / 16) return -3;
        *value = *value * 16 + digit;
    }
    return 0;
}
static int parse_range(const uint8_t *bytes, size_t length, uint64_t *address, size_t *count) {
    size_t separator = 0;
    uint64_t parsed = 0;
    while (separator < length && bytes[separator] != ',') ++separator;
    if (separator == length || parse_hex(bytes, separator, address) ||
        parse_hex(bytes + separator + 1, length - separator - 1, &parsed) || parsed > SIZE_MAX) return -3;
    *count = (size_t)parsed;
    return 0;
}
static uint8_t operation_for(uint8_t command) {
    if (command == 'G' || command == 'M') return 1;
    if (command == 'c' || command == 's' || command == 'b' || command == 'k') return 2;
    return 0;
}
static int handle_packet(ghostos_gdb *stub, uint8_t *output, size_t output_len) {
    const uint8_t *payload = stub->payload;
    size_t length = stub->payload_len;
    uint8_t operation = length ? operation_for(payload[0]) : 0;
    if (!length) return reply(output, output_len, (const uint8_t *)"", 0, (const uint8_t *)"", 0);
    if (!stub->runtime.permit || !stub->runtime.permit(stub->runtime.context, stub->token, operation))
        return error_reply(3, output, output_len);
    if (payload[0] == 'm') {
        uint64_t address = 0;
        size_t count = 0;
        uint8_t memory[(GHOSTOS_GDB_PACKET_BYTES - 1) / 2];
        uint8_t encoded[GHOSTOS_GDB_PACKET_BYTES];
        size_t read = 0;
        size_t i;
        int status = parse_range(payload + 1, length - 1, &address, &count);
        if (status) return status;
        if (count > sizeof memory) return error_reply(2, output, output_len);
        if (!stub->runtime.read_memory ||
            stub->runtime.read_memory(stub->runtime.context, address, memory, count, &read) || read > count)
            return -4;
        for (i = 0; i < read; ++i) {
            encoded[i * 2] = hex_digit((uint8_t)(memory[i] >> 4));
            encoded[i * 2 + 1] = hex_digit(memory[i]);
        }
        return reply(output, output_len, encoded, read * 2, memory, read);
    }
    if (payload[0] == 'M') return error_reply(1, output, output_len);
    if (payload[0] == '?' ) {
        uint8_t encoded[3] = {'S', hex_digit((uint8_t)(stub->runtime.stop_signal >> 4)), hex_digit(stub->runtime.stop_signal)};
        return reply(output, output_len, encoded, 3, encoded, 3);
    }
    return error_reply(1, output, output_len);
}
void ghostos_gdb_init(ghostos_gdb *stub, uint64_t token, ghostos_gdb_runtime runtime) {
    size_t i;
    stub->runtime = runtime;
    stub->token = token;
    for (i = 0; i < GHOSTOS_GDB_PACKET_BYTES; ++i) stub->payload[i] = 0;
    stub->payload_len = 0;
    stub->checksum = 0;
    stub->received_checksum = 0;
    stub->input_state = GHOSTOS_GDB_IDLE;
    stub->detached = false;
}
int ghostos_gdb_ingest(ghostos_gdb *stub, const uint8_t *input, size_t input_len, uint8_t *output, size_t output_len) {
    size_t i;
    for (i = 0; i < input_len; ++i) {
        uint8_t byte = input[i];
        if (stub->input_state == GHOSTOS_GDB_IDLE) {
            if (byte == '$') {
                stub->payload_len = 0;
                stub->checksum = 0;
                stub->input_state = GHOSTOS_GDB_PAYLOAD;
            }
        } else if (stub->input_state == GHOSTOS_GDB_PAYLOAD) {
            if (byte == '#') stub->input_state = GHOSTOS_GDB_HIGH;
            else if (stub->payload_len == GHOSTOS_GDB_PACKET_BYTES) {
                stub->payload_len = 0;
                stub->input_state = GHOSTOS_GDB_IDLE;
                if (!output_len) return -1;
                output[0] = '-';
                return 1;
            } else {
                stub->payload[stub->payload_len++] = byte;
                stub->checksum = (uint8_t)(stub->checksum + byte);
            }
        } else if (stub->input_state == GHOSTOS_GDB_HIGH) {
            uint8_t digit = 0;
            if (decode_hex(byte, &digit)) return -2;
            stub->received_checksum = (uint8_t)(digit << 4);
            stub->input_state = GHOSTOS_GDB_LOW;
        } else {
            uint8_t digit = 0;
            int length;
            if (decode_hex(byte, &digit)) return -2;
            stub->received_checksum |= digit;
            stub->input_state = GHOSTOS_GDB_IDLE;
            if (stub->received_checksum != stub->checksum) {
                stub->payload_len = 0;
                if (!output_len) return -1;
                output[0] = '-';
                return 1;
            }
            length = handle_packet(stub, output, output_len);
            stub->payload_len = 0;
            return length;
        }
    }
    return 0;
}
