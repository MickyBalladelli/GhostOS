#include "ghostos/remote_debug.h"
static uint64_t load_be64(const uint8_t *input) {
    uint64_t value = 0;
    size_t i;
    for (i = 0; i < 8; ++i) value = (value << 8) | input[i];
    return value;
}
int ghostos_remote_debug_open(const uint8_t *wire, size_t length, uint64_t target,
    uint64_t *resource, uint64_t *nonce) {
    uint64_t wire_resource;
    uint64_t wire_nonce;
    if (length != GHOSTOS_REMOTE_DEBUG_WIRE_BYTES || wire[0] != 'S' || wire[1] != 'Y' ||
        wire[2] != 'C' || wire[3] != 'A' || wire[4] != 1) return 1;
    wire_resource = load_be64(wire + 16);
    wire_nonce = load_be64(wire + 56);
    if (wire_resource != target || !wire_nonce) return 2;
    *resource = wire_resource;
    *nonce = wire_nonce;
    return 0;
}
uint16_t ghostos_remote_debug_rights(uint8_t operation) {
    uint16_t debug = (uint16_t)(1u << 12);
    if (operation == 0) return (uint16_t)(debug | 1u);
    if (operation == 1) return (uint16_t)(debug | 2u);
    return debug;
}
bool ghostos_remote_debug_permit(uint64_t token, uint64_t nonce, bool authorized) {
    return token == nonce && authorized;
}
