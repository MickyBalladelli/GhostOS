#include "ghostos/vm_power.h"

#define SLP_EN UINT16_C(1 << 13)

bool ghostos_vm_power_access_valid(uint16_t port, uint8_t size) {
    return port == GHOSTOS_VM_POWER_CONTROL_PORT && (size == 2 || size == 4);
}

uint32_t ghostos_vm_power_state_from_write(uint16_t value) {
    if ((value & SLP_EN) == 0) return GHOSTOS_VM_POWER_RUNNING;
    uint16_t sleep_type = (uint16_t)((value >> 10) & 0x07u);
    return sleep_type == 5 ? GHOSTOS_VM_POWER_SHUTDOWN : GHOSTOS_VM_POWER_REBOOT;
}
