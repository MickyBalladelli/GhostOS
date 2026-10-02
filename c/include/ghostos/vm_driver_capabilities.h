#ifndef GHOSTOS_VM_DRIVER_CAPABILITIES_H
#define GHOSTOS_VM_DRIVER_CAPABILITIES_H

#include <stdbool.h>
#include <stdint.h>

#define GHOSTOS_VM_DRIVER_CAPABILITY_COUNT 10u
#define GHOSTOS_VM_DRIVER_ACCELERATION 0u
#define GHOSTOS_VM_DRIVER_STORAGE 1u
#define GHOSTOS_VM_DRIVER_NIC_OFFLOAD 2u
#define GHOSTOS_VM_DRIVER_GPU 3u
#define GHOSTOS_VM_DRIVER_FIRMWARE 4u
#define GHOSTOS_VM_DRIVER_TIMER 5u

#define GHOSTOS_VM_DRIVER_NETWORK_DETERMINISTIC 0u
#define GHOSTOS_VM_DRIVER_NETWORK_NAT 1u
#define GHOSTOS_VM_DRIVER_NETWORK_BRIDGED 2u

typedef struct {
    uint32_t kind;
    bool available;
    const char *feature, *selected, *fallback, *semantics;
} ghostos_vm_driver_capability;

/* All text points to immutable static storage. Network values outside the
 * constants above return false without writing the output report. */
bool ghostos_vm_driver_discover(bool uefi, uint32_t network,
    bool requested_native, bool native_execution,
    ghostos_vm_driver_capability output[GHOSTOS_VM_DRIVER_CAPABILITY_COUNT]);
const char *ghostos_vm_driver_kind_name(uint32_t kind);

#endif
