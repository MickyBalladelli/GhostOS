#include "ghostos/syscall.h"
#include "ghostos/abi.h"

#define GHOSTOS_SYSCALL_USER_START UINT64_C(0x0000008000000000)
#define GHOSTOS_SYSCALL_USER_END UINT64_C(0x00007ffffffff000)

bool ghostos_syscall_valid_user_range(uint64_t address, uint64_t length,
    uint64_t alignment) {
    if (address == 0 || alignment == 0 || address % alignment != 0 ||
        address < GHOSTOS_SYSCALL_USER_START || address > UINT64_MAX - length)
        return false;
    return address + length <= GHOSTOS_SYSCALL_USER_END;
}

bool ghostos_syscall_validate_request_shape(uint16_t operation,
    uint16_t abi_version, uint16_t reserved) {
    return abi_version == GHOSTOS_ABI_SCHEMA_VERSION && reserved == 0 &&
        operation >= GHOSTOS_OP_YIELD && operation <= GHOSTOS_OP_LOGIN_BRIDGE_READ;
}

uint64_t ghostos_syscall_sleep_hint(uint16_t operation, uint32_t status,
    uint64_t deadline, uint64_t now) {
    if (status != GHOSTOS_STATUS_NORMAL) return 0;
    if (operation == GHOSTOS_OP_SLEEP_UNTIL) {
        uint64_t remaining = deadline > now ? deadline - now : 0;
        return remaining == 0 ? UINT64_MAX - 1 : remaining;
    }
    if (operation == GHOSTOS_OP_YIELD) return UINT64_MAX;
    return 0;
}
