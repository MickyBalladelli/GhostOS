#ifndef GHOSTOS_INVARIANTS_H
#define GHOSTOS_INVARIANTS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef enum {
    GHOSTOS_INVARIANT_ADDRESS_SPACE_OWNERSHIP = 1,
    GHOSTOS_INVARIANT_CAPABILITY_DERIVATION = 2,
    GHOSTOS_INVARIANT_IPC_OWNERSHIP = 3,
    GHOSTOS_INVARIANT_SCHEDULER_STATE = 4,
    GHOSTOS_INVARIANT_INTERRUPT_DELIVERY = 5,
    GHOSTOS_INVARIANT_PAGE_TABLE_TRANSITION = 6
} ghostos_invariant_id;

typedef struct {
    const char *id, *domain, *statement, *redaction;
    uint16_t failure_code;
    ghostos_invariant_id invariant;
} ghostos_invariant_definition;

typedef struct { ghostos_invariant_id invariant; uint16_t code; } ghostos_invariant_failure;
typedef void (*ghostos_invariant_trap_fn)(void *context, ghostos_invariant_failure failure);

#define GHOSTOS_INVARIANT_CATALOGUE_COUNT 6u
extern const ghostos_invariant_definition ghostos_invariant_catalogue[GHOSTOS_INVARIANT_CATALOGUE_COUNT];

const char *ghostos_invariant_identifier(ghostos_invariant_id invariant);
ghostos_invariant_failure ghostos_invariant_failure_for(ghostos_invariant_id invariant);
bool ghostos_invariant_format_failure(ghostos_invariant_failure failure, char *buffer, size_t capacity);
void ghostos_invariant_debug_assert_valid(bool valid, ghostos_invariant_failure failure,
    ghostos_invariant_trap_fn trap, void *context);
bool ghostos_invariant_check_address_space(uint32_t id);
bool ghostos_invariant_check_interrupt_delivery(uint64_t vector, uint8_t cpu, bool isolated,
    bool delivered_to_kernel, ghostos_invariant_failure *failure);
bool ghostos_invariant_check_page_table_transition(const uint64_t *frames, size_t count,
    uint64_t physical_offset, ghostos_invariant_failure *failure);

#endif
