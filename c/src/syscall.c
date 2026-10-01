#include "ghostos/syscall.h"
#include "ghostos/abi.h"
#include <stdatomic.h>

#define GHOSTOS_SYSCALL_USER_START UINT64_C(0x0000008000000000)
#define GHOSTOS_SYSCALL_USER_END UINT64_C(0x00007ffffffff000)

static atomic_uintptr_t dispatcher = ATOMIC_VAR_INIT(0);

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

bool ghostos_syscall_install_dispatcher(uintptr_t handler) {
    if (handler == 0) return false;
    uintptr_t expected = 0;
    return atomic_compare_exchange_strong_explicit(&dispatcher, &expected, handler,
        memory_order_acq_rel, memory_order_acquire);
}

uintptr_t ghostos_syscall_dispatcher(void) {
    return atomic_load_explicit(&dispatcher, memory_order_acquire);
}

bool ghostos_syscall_validate_memory_map(uint16_t flags, uint16_t reserved,
    const uint64_t arguments[6]) {
    if (!arguments || flags != 0 || reserved != 0 || arguments[2] > 1) return false;
    for (size_t index = 3; index < 6; ++index) {
        if (arguments[index] != 0) return false;
    }
    return true;
}

bool ghostos_syscall_validate_memory_unmap(uint16_t flags, uint16_t reserved,
    const uint64_t arguments[6]) {
    if (!arguments || flags != 0 || reserved != 0) return false;
    for (size_t index = 2; index < 6; ++index) {
        if (arguments[index] != 0) return false;
    }
    return true;
}
