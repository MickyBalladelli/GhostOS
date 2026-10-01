#ifndef GHOSTOS_SYSCALL_H
#define GHOSTOS_SYSCALL_H

#include <stdbool.h>
#include <stdint.h>

bool ghostos_syscall_valid_user_range(uint64_t address, uint64_t length,
    uint64_t alignment);
bool ghostos_syscall_validate_request_shape(uint16_t operation,
    uint16_t abi_version, uint16_t reserved);
uint64_t ghostos_syscall_sleep_hint(uint16_t operation, uint32_t status,
    uint64_t deadline, uint64_t now);

#endif
