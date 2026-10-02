#ifndef GHOSTOS_VM_MAC_H
#define GHOSTOS_VM_MAC_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_VM_MAC_ADDRESS_LEN 6u
#define GHOSTOS_VM_MAC_TEXT_LEN 18u

typedef struct {
    uint8_t bytes[GHOSTOS_VM_MAC_ADDRESS_LEN];
} ghostos_vm_mac_address;

extern const ghostos_vm_mac_address ghostos_vm_mac_broadcast;

bool ghostos_vm_mac_from_bytes(const uint8_t *bytes, size_t length,
    ghostos_vm_mac_address *out);
bool ghostos_vm_mac_is_broadcast(const ghostos_vm_mac_address *address);
bool ghostos_vm_mac_is_unicast(const ghostos_vm_mac_address *address);
bool ghostos_vm_mac_is_multicast(const ghostos_vm_mac_address *address);
bool ghostos_vm_mac_matches(const uint8_t *destination, size_t length,
    const ghostos_vm_mac_address *own, bool promiscuous);
bool ghostos_vm_mac_format(const ghostos_vm_mac_address *address,
    char output[GHOSTOS_VM_MAC_TEXT_LEN]);

#endif
