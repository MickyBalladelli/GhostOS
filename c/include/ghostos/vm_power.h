#ifndef GHOSTOS_VM_POWER_H
#define GHOSTOS_VM_POWER_H

#include <stdbool.h>
#include <stdint.h>

#define GHOSTOS_VM_POWER_CONTROL_PORT UINT16_C(0x0604)
#define GHOSTOS_VM_POWER_RUNNING 0u
#define GHOSTOS_VM_POWER_SHUTDOWN 1u
#define GHOSTOS_VM_POWER_REBOOT 2u

bool ghostos_vm_power_access_valid(uint16_t port, uint8_t size);
uint32_t ghostos_vm_power_state_from_write(uint16_t value);

#endif
