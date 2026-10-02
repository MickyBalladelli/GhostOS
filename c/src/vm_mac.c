#include "ghostos/vm_mac.h"

const ghostos_vm_mac_address ghostos_vm_mac_broadcast = {
    {0xff, 0xff, 0xff, 0xff, 0xff, 0xff}
};

bool ghostos_vm_mac_from_bytes(const uint8_t *bytes, size_t length,
    ghostos_vm_mac_address *out) {
    if (!bytes || !out || length != GHOSTOS_VM_MAC_ADDRESS_LEN) return false;
    for (size_t i = 0; i < GHOSTOS_VM_MAC_ADDRESS_LEN; ++i) out->bytes[i] = bytes[i];
    return true;
}

bool ghostos_vm_mac_is_broadcast(const ghostos_vm_mac_address *address) {
    if (!address) return false;
    for (size_t i = 0; i < GHOSTOS_VM_MAC_ADDRESS_LEN; ++i) {
        if (address->bytes[i] != 0xff) return false;
    }
    return true;
}

bool ghostos_vm_mac_is_unicast(const ghostos_vm_mac_address *address) {
    return address && (address->bytes[0] & 1u) == 0;
}

bool ghostos_vm_mac_is_multicast(const ghostos_vm_mac_address *address) {
    return address && (address->bytes[0] & 1u) != 0 &&
        !ghostos_vm_mac_is_broadcast(address);
}

bool ghostos_vm_mac_matches(const uint8_t *destination, size_t length,
    const ghostos_vm_mac_address *own, bool promiscuous) {
    if (promiscuous) return true;
    ghostos_vm_mac_address destination_mac;
    if (!own || !ghostos_vm_mac_from_bytes(destination, length, &destination_mac))
        return false;
    if (ghostos_vm_mac_is_broadcast(&destination_mac) ||
        ghostos_vm_mac_is_multicast(&destination_mac)) return true;
    for (size_t i = 0; i < GHOSTOS_VM_MAC_ADDRESS_LEN; ++i) {
        if (destination_mac.bytes[i] != own->bytes[i]) return false;
    }
    return true;
}

bool ghostos_vm_mac_format(const ghostos_vm_mac_address *address,
    char output[GHOSTOS_VM_MAC_TEXT_LEN]) {
    static const char digits[] = "0123456789abcdef";
    if (!address || !output) return false;
    size_t position = 0;
    for (size_t i = 0; i < GHOSTOS_VM_MAC_ADDRESS_LEN; ++i) {
        if (i != 0) output[position++] = ':';
        output[position++] = digits[address->bytes[i] >> 4];
        output[position++] = digits[address->bytes[i] & 0x0f];
    }
    output[position] = '\0';
    return true;
}
