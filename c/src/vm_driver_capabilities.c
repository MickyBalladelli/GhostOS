#include "ghostos/vm_driver_capabilities.h"

#include <stddef.h>

_Static_assert(sizeof(ghostos_vm_driver_capability) == 40, "VM driver report ABI");
_Static_assert(offsetof(ghostos_vm_driver_capability, feature) == 8, "VM driver feature offset");

const char *ghostos_vm_driver_kind_name(uint32_t kind) {
    static const char *const names[] = {
        "acceleration", "storage", "nic-offload", "gpu", "firmware", "platform-timer"
    };
    return kind < sizeof(names) / sizeof(names[0]) ? names[kind] : NULL;
}

bool ghostos_vm_driver_discover(bool uefi, uint32_t network,
    bool requested_native, bool native_execution,
    ghostos_vm_driver_capability output[GHOSTOS_VM_DRIVER_CAPABILITY_COUNT]) {
    static const char *const network_paths[] = {
        "deterministic-software-ethernet", "userspace-nat-ethernet", "host-bridged-ethernet"
    };
    if (!output || network >= sizeof(network_paths) / sizeof(network_paths[0])) return false;
    const char *firmware = uefi ? "uefi-boot-services" : "legacy-bios-services";
    output[0] = (ghostos_vm_driver_capability){
        GHOSTOS_VM_DRIVER_ACCELERATION, !requested_native || native_execution,
        "native-guest-execution",
        native_execution ? "native-guest-execution" : "portable-cpu-execution",
        requested_native && !native_execution ? "portable-cpu-execution" : "none",
        "instruction results, interrupts, device effects, replay, and snapshots stay equivalent"
    };
    output[1] = (ghostos_vm_driver_capability){
        GHOSTOS_VM_DRIVER_STORAGE, true, "controller-flush", "controller-flush",
        "ordered-image-flush", "flush completion keeps the existing durability boundary"
    };
    output[2] = (ghostos_vm_driver_capability){
        GHOSTOS_VM_DRIVER_STORAGE, false, "storage-discard", "unsupported-status",
        "unsupported-status",
        "a missing discard operation is rejected explicitly; readable data is not changed"
    };
    output[3] = (ghostos_vm_driver_capability){
        GHOSTOS_VM_DRIVER_STORAGE, false, "async-storage-queue", "bounded-synchronous-image-io",
        "bounded-synchronous-image-io", "request ordering and completion status remain deterministic"
    };
    output[4] = (ghostos_vm_driver_capability){
        GHOSTOS_VM_DRIVER_NIC_OFFLOAD, false, "hardware-offloads", "software-packet-processing",
        "software-packet-processing",
        "wire bytes, checksum behavior, delivery order, and queue limits remain unchanged"
    };
    output[5] = (ghostos_vm_driver_capability){
        GHOSTOS_VM_DRIVER_NIC_OFFLOAD, true, "network-backend", network_paths[network],
        "deterministic-software-ethernet", "packet framing and guest-visible link state remain explicit"
    };
    output[6] = (ghostos_vm_driver_capability){
        GHOSTOS_VM_DRIVER_GPU, false, "host-gpu-acceleration", "software-vga-vesa-rendering",
        "software-vga-vesa-rendering", "text and framebuffer pixels are rendered by the VM device model"
    };
    output[7] = (ghostos_vm_driver_capability){
        GHOSTOS_VM_DRIVER_FIRMWARE, true, "firmware-services", firmware,
        "guest-visible-unsupported-status", "unsupported firmware calls return their stable firmware status code"
    };
    output[8] = (ghostos_vm_driver_capability){
        GHOSTOS_VM_DRIVER_FIRMWARE, uefi, "uefi-runtime-services",
        uefi ? "uefi-runtime-services" : "legacy-bios-services",
        uefi ? "none" : "legacy-bios-services",
        "time, variables, and reset use the selected firmware contract"
    };
    output[9] = (ghostos_vm_driver_capability){
        GHOSTOS_VM_DRIVER_TIMER, true, "platform-timer-source", "shared-monotonic-clock",
        "shared-monotonic-clock", "APIC, PIT, HPET, pvclock, and scheduling observe one monotonic time source"
    };
    return true;
}
