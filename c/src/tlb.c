#include "ghostos/tlb.h"

static bool contains(const uint64_t mask[2], uint8_t cpu) {
    return cpu < GHOSTOS_TLB_MAX_CPUS &&
        (mask[cpu / 64] & (UINT64_C(1) << (cpu % 64))) != 0;
}

static ghostos_tlb_request *find_request(ghostos_tlb_state *state, uint64_t id) {
    if (!state || id == 0) return NULL;
    for (size_t index = 0; index < state->capacity; ++index) {
        ghostos_tlb_request *request = &state->requests[index];
        if (request->used && request->id == id) return request;
    }
    return NULL;
}

static const ghostos_tlb_request *find_request_const(const ghostos_tlb_state *state, uint64_t id) {
    if (!state || id == 0) return NULL;
    for (size_t index = 0; index < state->capacity; ++index) {
        const ghostos_tlb_request *request = &state->requests[index];
        if (request->used && request->id == id) return request;
    }
    return NULL;
}

static bool complete(const ghostos_tlb_request *request) {
    return request->acknowledged[0] == request->targets[0] &&
        request->acknowledged[1] == request->targets[1];
}

void ghostos_tlb_init(ghostos_tlb_state *state, size_t capacity) {
    if (!state) return;
    if (capacity > GHOSTOS_TLB_MAX_SHOOTDOWNS) capacity = GHOSTOS_TLB_MAX_SHOOTDOWNS;
    state->capacity = capacity;
    state->next_id = 1;
    for (size_t index = 0; index < GHOSTOS_TLB_MAX_SHOOTDOWNS; ++index)
        state->requests[index] = (ghostos_tlb_request){0};
}

ghostos_tlb_result ghostos_tlb_begin(ghostos_tlb_state *state, uint32_t address_space,
    uint64_t start, uint64_t length, const uint64_t targets[2], uint8_t initiator,
    ghostos_tlb_invalidate_fn invalidate, ghostos_tlb_send_ipi_fn send_ipi,
    void *context, uint64_t *id) {
    if (!state || !targets || !id || !invalidate || !send_ipi ||
        start % GHOSTOS_TLB_PAGE_SIZE != 0 || length == 0 ||
        length % GHOSTOS_TLB_PAGE_SIZE != 0 || UINT64_MAX - start < length)
        return GHOSTOS_TLB_INVALID_RANGE;
    if ((targets[0] | targets[1]) == 0) return GHOSTOS_TLB_INVALID_TARGET;
    if (!contains(targets, initiator)) return GHOSTOS_TLB_MISSING_INITIATOR;
    size_t slot = state->capacity;
    for (size_t index = 0; index < state->capacity; ++index) {
        if (!state->requests[index].used) {
            slot = index;
            break;
        }
    }
    if (slot == state->capacity) return GHOSTOS_TLB_CAPACITY;

    uint64_t request_id = state->next_id;
    state->next_id = request_id + 1;
    if (state->next_id == 0) state->next_id = 1;
    invalidate(context, start, length);
    for (uint16_t raw = 0; raw < GHOSTOS_TLB_MAX_CPUS; ++raw) {
        uint8_t cpu = (uint8_t)raw;
        if (cpu != initiator && contains(targets, cpu)) send_ipi(context, cpu);
    }
    uint64_t acknowledged[2] = {0, 0};
    acknowledged[initiator / 64] = UINT64_C(1) << (initiator % 64);
    state->requests[slot] = (ghostos_tlb_request){
        .used = 1,
        .address_space = address_space,
        .id = request_id,
        .start = start,
        .length = length,
        .targets = {targets[0], targets[1]},
        .acknowledged = {acknowledged[0], acknowledged[1]},
    };
    *id = request_id;
    return GHOSTOS_TLB_OK;
}

ghostos_tlb_result ghostos_tlb_acknowledge(ghostos_tlb_state *state, uint64_t id,
    uint8_t cpu, ghostos_tlb_invalidate_fn invalidate, void *context, bool *is_complete) {
    ghostos_tlb_request *request = find_request(state, id);
    if (!request) return GHOSTOS_TLB_NOT_FOUND;
    if (!contains(request->targets, cpu)) return GHOSTOS_TLB_INVALID_TARGET;
    if (contains(request->acknowledged, cpu)) return GHOSTOS_TLB_ALREADY_ACKNOWLEDGED;
    if (!invalidate || !is_complete) return GHOSTOS_TLB_INVALID_TARGET;
    invalidate(context, request->start, request->length);
    request->acknowledged[cpu / 64] |= UINT64_C(1) << (cpu % 64);
    *is_complete = complete(request);
    return GHOSTOS_TLB_OK;
}

ghostos_tlb_result ghostos_tlb_request_info(const ghostos_tlb_state *state, uint64_t id,
    uint32_t *address_space, uint64_t *start, uint64_t *length, uint64_t targets[2]) {
    const ghostos_tlb_request *request = find_request_const(state, id);
    if (!request) return GHOSTOS_TLB_NOT_FOUND;
    if (!address_space || !start || !length || !targets) return GHOSTOS_TLB_NOT_FOUND;
    *address_space = request->address_space;
    *start = request->start;
    *length = request->length;
    targets[0] = request->targets[0];
    targets[1] = request->targets[1];
    return GHOSTOS_TLB_OK;
}

bool ghostos_tlb_pending_targets(const ghostos_tlb_state *state, uint64_t id, uint64_t pending[2]) {
    const ghostos_tlb_request *request = find_request_const(state, id);
    if (!request || !pending) return false;
    pending[0] = request->targets[0] & ~request->acknowledged[0];
    pending[1] = request->targets[1] & ~request->acknowledged[1];
    return true;
}

bool ghostos_tlb_is_complete(const ghostos_tlb_state *state, uint64_t id) {
    const ghostos_tlb_request *request = find_request_const(state, id);
    return request && complete(request);
}

ghostos_tlb_result ghostos_tlb_retire(ghostos_tlb_state *state, uint64_t id) {
    ghostos_tlb_request *request = find_request(state, id);
    if (!request) return GHOSTOS_TLB_NOT_FOUND;
    if (!complete(request)) return GHOSTOS_TLB_NOT_COMPLETE;
    *request = (ghostos_tlb_request){0};
    return GHOSTOS_TLB_OK;
}
