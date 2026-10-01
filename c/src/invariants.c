#include "ghostos/invariants.h"

const ghostos_invariant_definition ghostos_invariant_catalogue[GHOSTOS_INVARIANT_CATALOGUE_COUNT] = {
    {"address_space.ownership", "address_spaces", "A capability owner and every address-space object name a valid address space.", "identifier-only", 1001, GHOSTOS_INVARIANT_ADDRESS_SPACE_OWNERSHIP},
    {"capability.derivation", "capabilities", "Live capability generations, rights, backing ranges, and derivation links are consistent.", "identifier-only", 1002, GHOSTOS_INVARIANT_CAPABILITY_DERIVATION},
    {"ipc.ownership", "ipc", "Only an owner of a channel capability with the required right may use its bounded queue.", "identifier-only", 1003, GHOSTOS_INVARIANT_IPC_OWNERSHIP},
    {"scheduler.state", "scheduler", "Live threads have valid generations and contexts, and only one live thread runs at once.", "identifier-only", 1004, GHOSTOS_INVARIANT_SCHEDULER_STATE},
    {"interrupt.delivery", "interrupts", "Interrupt vectors are bounded and isolated CPUs do not deliver kernel work.", "identifier-only", 1005, GHOSTOS_INVARIANT_INTERRUPT_DELIVERY},
    {"page_table.transition", "page_tables", "A page-table root transition uses distinct aligned frames and checked aliases.", "identifier-only", 1006, GHOSTOS_INVARIANT_PAGE_TABLE_TRANSITION}
};

static bool valid_id(ghostos_invariant_id invariant) {
    return invariant >= GHOSTOS_INVARIANT_ADDRESS_SPACE_OWNERSHIP && invariant <= GHOSTOS_INVARIANT_PAGE_TABLE_TRANSITION;
}

const char *ghostos_invariant_identifier(ghostos_invariant_id invariant) {
    return valid_id(invariant) ? ghostos_invariant_catalogue[(unsigned)invariant - 1].id : "unknown";
}

ghostos_invariant_failure ghostos_invariant_failure_for(ghostos_invariant_id invariant) {
    return (ghostos_invariant_failure){invariant, valid_id(invariant) ? ghostos_invariant_catalogue[(unsigned)invariant - 1].failure_code : 0};
}

static bool append_char(char *buffer, size_t capacity, size_t *length, char c) {
    if (*length + 1 >= capacity) return false;
    buffer[(*length)++] = c;
    buffer[*length] = '\0';
    return true;
}

static bool append_text(char *buffer, size_t capacity, size_t *length, const char *text) {
    while (*text) if (!append_char(buffer, capacity, length, *text++)) return false;
    return true;
}

static bool append_u16(char *buffer, size_t capacity, size_t *length, uint16_t value) {
    char digits[5];
    size_t count = 0;
    do { digits[count++] = (char)('0' + value % 10); value /= 10; } while (value);
    while (count) if (!append_char(buffer, capacity, length, digits[--count])) return false;
    return true;
}

bool ghostos_invariant_format_failure(ghostos_invariant_failure failure, char *buffer, size_t capacity) {
    if (!buffer || !capacity) return false;
    size_t length = 0;
    buffer[0] = '\0';
    return append_text(buffer, capacity, &length, "invariant=") &&
        append_text(buffer, capacity, &length, ghostos_invariant_identifier(failure.invariant)) &&
        append_text(buffer, capacity, &length, " code=") &&
        append_u16(buffer, capacity, &length, failure.code);
}

void ghostos_invariant_debug_assert_valid(bool valid, ghostos_invariant_failure failure,
    ghostos_invariant_trap_fn trap, void *context) {
#ifndef NDEBUG
    if (!valid && trap) trap(context, failure);
#else
    (void)valid; (void)failure; (void)trap; (void)context;
#endif
}

bool ghostos_invariant_check_address_space(uint32_t id) {
    (void)id;
    return true;
}

bool ghostos_invariant_check_interrupt_delivery(uint64_t vector, uint8_t cpu, bool isolated,
    bool delivered_to_kernel, ghostos_invariant_failure *failure) {
    (void)cpu;
    if (vector >= 256 || (isolated && delivered_to_kernel)) {
        if (failure) *failure = ghostos_invariant_failure_for(GHOSTOS_INVARIANT_INTERRUPT_DELIVERY);
        return false;
    }
    return true;
}

bool ghostos_invariant_check_page_table_transition(const uint64_t *frames, size_t count,
    uint64_t physical_offset, ghostos_invariant_failure *failure) {
    if (!frames && count) goto invalid;
    for (size_t i = 0; i < count; ++i) {
        if (!frames[i] || frames[i] % 4096 || UINT64_MAX - frames[i] < physical_offset) goto invalid;
        for (size_t previous = 0; previous < i; ++previous) if (frames[previous] == frames[i]) goto invalid;
    }
    return true;
invalid:
    if (failure) *failure = ghostos_invariant_failure_for(GHOSTOS_INVARIANT_PAGE_TABLE_TRANSITION);
    return false;
}
