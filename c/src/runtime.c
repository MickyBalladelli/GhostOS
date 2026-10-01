#include "ghostos/runtime.h"

void ghostos_runtime_init(ghostos_runtime_state *state) {
    if (state) *state = (ghostos_runtime_state){0};
}

uint64_t ghostos_runtime_clock(const ghostos_runtime_state *state) {
    return state ? state->clock : 0;
}

void ghostos_runtime_advance_clock(ghostos_runtime_state *state, uint64_t elapsed) {
    if (!state) return;
    state->clock = UINT64_MAX - state->clock < elapsed ? UINT64_MAX : state->clock + elapsed;
}

static size_t bounded_capacity(size_t capacity) {
    return capacity < GHOSTOS_RUNTIME_MAX_FILESYSTEM_PROCESSES ?
        capacity : GHOSTOS_RUNTIME_MAX_FILESYSTEM_PROCESSES;
}

ghostos_runtime_result ghostos_runtime_register(ghostos_runtime_state *state,
    uint32_t caller, ghostos_runtime_identity identity, size_t capacity) {
    if (!state) return GHOSTOS_RUNTIME_INVALID_REQUEST;
    capacity = bounded_capacity(capacity);
    ghostos_runtime_process_slot *free_slot = NULL;
    for (size_t index = 0; index < capacity; ++index) {
        ghostos_runtime_process_slot *slot = &state->processes[index];
        if (slot->used && slot->caller == caller) {
            slot->process = identity.process;
            slot->authority = identity.authority;
            return GHOSTOS_RUNTIME_OK;
        }
        if (!slot->used && !free_slot) free_slot = slot;
    }
    if (!free_slot) return GHOSTOS_RUNTIME_CAPACITY;
    *free_slot = (ghostos_runtime_process_slot){caller, 1, identity.process, identity.authority};
    return GHOSTOS_RUNTIME_OK;
}

ghostos_runtime_result ghostos_runtime_unregister(ghostos_runtime_state *state, uint32_t caller,
    size_t capacity) {
    if (!state) return GHOSTOS_RUNTIME_INVALID_REQUEST;
    for (size_t index = 0, count = bounded_capacity(capacity); index < count; ++index) {
        ghostos_runtime_process_slot *slot = &state->processes[index];
        if (slot->used && slot->caller == caller) {
            *slot = (ghostos_runtime_process_slot){0};
            return GHOSTOS_RUNTIME_OK;
        }
    }
    return GHOSTOS_RUNTIME_PROCESS_NOT_REGISTERED;
}

ghostos_runtime_result ghostos_runtime_lookup(const ghostos_runtime_state *state,
    uint32_t caller, ghostos_runtime_identity *identity, size_t capacity) {
    if (!state || !identity) return GHOSTOS_RUNTIME_INVALID_REQUEST;
    for (size_t index = 0, count = bounded_capacity(capacity); index < count; ++index) {
        const ghostos_runtime_process_slot *slot = &state->processes[index];
        if (slot->used && slot->caller == caller) {
            *identity = (ghostos_runtime_identity){slot->process, slot->authority};
            return GHOSTOS_RUNTIME_OK;
        }
    }
    return GHOSTOS_RUNTIME_PROCESS_NOT_REGISTERED;
}

ghostos_runtime_result ghostos_runtime_validate_request(const ghostos_request *request) {
    if (!request) return GHOSTOS_RUNTIME_INVALID_REQUEST;
    if (request->abi_version != GHOSTOS_ABI_SCHEMA_VERSION) return GHOSTOS_RUNTIME_ABI_MISMATCH;
    if (request->operation < GHOSTOS_OP_YIELD || request->operation > GHOSTOS_OP_LOGIN_BRIDGE_READ
        || request->reserved != 0)
        return GHOSTOS_RUNTIME_INVALID_REQUEST;
    return GHOSTOS_RUNTIME_OK;
}

bool ghostos_runtime_operation_delegated(uint16_t operation) {
    switch (operation) {
        case GHOSTOS_OP_CLOCK_NOW: case GHOSTOS_OP_REALTIME_NOW: case GHOSTOS_OP_YIELD:
        case GHOSTOS_OP_SYN_FS_OPEN: case GHOSTOS_OP_SYN_FS_CLOSE: case GHOSTOS_OP_SYN_FS_READ:
        case GHOSTOS_OP_SYN_FS_WRITE: case GHOSTOS_OP_SYN_FS_MAP: case GHOSTOS_OP_SYN_FS_UNMAP:
        case GHOSTOS_OP_SYN_FS_METADATA: case GHOSTOS_OP_SYN_FS_LIST: case GHOSTOS_OP_SYN_FS_MKDIR:
        case GHOSTOS_OP_SYN_FS_RMDIR: case GHOSTOS_OP_SYN_FS_LINK: case GHOSTOS_OP_SYN_FS_LINKS:
        case GHOSTOS_OP_SYN_FS_DELETE:
            return false;
        default:
            return true;
    }
}
