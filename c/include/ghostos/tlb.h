#ifndef GHOSTOS_TLB_H
#define GHOSTOS_TLB_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_TLB_MAX_SHOOTDOWNS 64u
#define GHOSTOS_TLB_MAX_CPUS 128u
#define GHOSTOS_TLB_PAGE_SIZE UINT64_C(4096)

typedef struct {
    uint32_t used;
    uint32_t address_space;
    uint64_t id;
    uint64_t start;
    uint64_t length;
    uint64_t targets[2];
    uint64_t acknowledged[2];
} ghostos_tlb_request;

typedef struct {
    size_t capacity;
    uint64_t next_id;
    ghostos_tlb_request requests[GHOSTOS_TLB_MAX_SHOOTDOWNS];
} ghostos_tlb_state;

typedef enum {
    GHOSTOS_TLB_OK,
    GHOSTOS_TLB_INVALID_RANGE,
    GHOSTOS_TLB_MISSING_INITIATOR,
    GHOSTOS_TLB_CAPACITY,
    GHOSTOS_TLB_NOT_FOUND,
    GHOSTOS_TLB_INVALID_TARGET,
    GHOSTOS_TLB_ALREADY_ACKNOWLEDGED,
    GHOSTOS_TLB_NOT_COMPLETE
} ghostos_tlb_result;

typedef void (*ghostos_tlb_invalidate_fn)(void *context, uint64_t start, uint64_t length);
typedef void (*ghostos_tlb_send_ipi_fn)(void *context, uint8_t cpu);

_Static_assert(sizeof(ghostos_tlb_request) == 64, "TLB request layout");

void ghostos_tlb_init(ghostos_tlb_state *state, size_t capacity);
ghostos_tlb_result ghostos_tlb_begin(ghostos_tlb_state *state, uint32_t address_space,
    uint64_t start, uint64_t length, const uint64_t targets[2], uint8_t initiator,
    ghostos_tlb_invalidate_fn invalidate, ghostos_tlb_send_ipi_fn send_ipi,
    void *context, uint64_t *id);
ghostos_tlb_result ghostos_tlb_acknowledge(ghostos_tlb_state *state, uint64_t id,
    uint8_t cpu, ghostos_tlb_invalidate_fn invalidate, void *context, bool *complete);
ghostos_tlb_result ghostos_tlb_request_info(const ghostos_tlb_state *state, uint64_t id,
    uint32_t *address_space, uint64_t *start, uint64_t *length, uint64_t targets[2]);
bool ghostos_tlb_pending_targets(const ghostos_tlb_state *state, uint64_t id, uint64_t pending[2]);
bool ghostos_tlb_is_complete(const ghostos_tlb_state *state, uint64_t id);
ghostos_tlb_result ghostos_tlb_retire(ghostos_tlb_state *state, uint64_t id);

#endif
