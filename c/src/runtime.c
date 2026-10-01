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

ghostos_runtime_result ghostos_runtime_validate_empty_request(const ghostos_request *request,
    bool capability_allowed) {
    if (!request) return GHOSTOS_RUNTIME_INVALID_REQUEST;
    if ((!capability_allowed && request->capability != 0) || request->flags != 0) {
        return GHOSTOS_RUNTIME_INVALID_REQUEST;
    }
    for (size_t index = 0; index < 6; ++index) {
        if (request->arguments[index] != 0) return GHOSTOS_RUNTIME_INVALID_REQUEST;
    }
    return GHOSTOS_RUNTIME_OK;
}

static bool needs_buffer(uint16_t operation) {
    switch (operation) {
        case GHOSTOS_OP_SYN_FS_OPEN: case GHOSTOS_OP_SYN_FS_READ: case GHOSTOS_OP_SYN_FS_WRITE:
        case GHOSTOS_OP_SYN_FS_LIST: case GHOSTOS_OP_SYN_FS_MKDIR: case GHOSTOS_OP_SYN_FS_RMDIR:
        case GHOSTOS_OP_SYN_FS_LINK: case GHOSTOS_OP_SYN_FS_LINKS: case GHOSTOS_OP_SYN_FS_DELETE:
            return true;
        default:
            return false;
    }
}

ghostos_runtime_result ghostos_runtime_validate_filesystem_request(const ghostos_request *request,
    uint32_t *region, uint32_t *offset, uint32_t *length, bool *writable, bool *has_buffer) {
    if (!request || !region || !offset || !length || !writable || !has_buffer)
        return GHOSTOS_RUNTIME_INVALID_REQUEST;
    *region = *offset = *length = 0;
    *writable = false;
    *has_buffer = false;
    uint16_t operation = request->operation;
    bool uses_buffer = needs_buffer(operation);
    uint64_t raw_region = request->arguments[0];
    uint64_t raw_offset = request->arguments[1];
    uint64_t raw_length = request->arguments[2];
    bool raw_writable = request->arguments[3] != 0;
    bool descriptor_present = raw_region || raw_offset || raw_length || request->arguments[3];
    if (!uses_buffer) {
        if (descriptor_present) return GHOSTOS_RUNTIME_INVALID_BUFFER;
    } else {
        if (raw_region == 0 || raw_region > UINT32_MAX || raw_offset > UINT32_MAX ||
            raw_length > UINT32_MAX || request->arguments[3] > 1 ||
            raw_offset + raw_length > UINT32_MAX || raw_length > GHOSTOS_RUNTIME_MAX_IPC_BUFFER_BYTES)
            return GHOSTOS_RUNTIME_INVALID_BUFFER;
        bool expected_writable = operation == GHOSTOS_OP_SYN_FS_READ ||
            operation == GHOSTOS_OP_SYN_FS_LIST || operation == GHOSTOS_OP_SYN_FS_LINKS;
        if (raw_writable != expected_writable) return GHOSTOS_RUNTIME_INVALID_BUFFER;
        *region = (uint32_t)raw_region;
        *offset = (uint32_t)raw_offset;
        *length = (uint32_t)raw_length;
        *writable = raw_writable;
        *has_buffer = true;
    }

    bool ambient_capability = operation == GHOSTOS_OP_SYN_FS_OPEN || operation == GHOSTOS_OP_SYN_FS_LIST ||
        operation == GHOSTOS_OP_SYN_FS_MKDIR || operation == GHOSTOS_OP_SYN_FS_RMDIR ||
        operation == GHOSTOS_OP_SYN_FS_LINKS || operation == GHOSTOS_OP_SYN_FS_DELETE;
    if (ambient_capability) {
        if (request->capability != 0) return GHOSTOS_RUNTIME_INVALID_REQUEST;
    } else if ((request->capability >> 32) == 0) {
        return GHOSTOS_RUNTIME_INVALID_CAPABILITY;
    }

    switch (operation) {
        case GHOSTOS_OP_SYN_FS_OPEN:
            if ((request->flags & ~UINT16_C(0x21f)) != 0 || request->arguments[4] || request->arguments[5])
                return GHOSTOS_RUNTIME_INVALID_REQUEST;
            break;
        case GHOSTOS_OP_SYN_FS_MKDIR:
            if ((request->flags & ~UINT16_C(1u << 8)) != 0 || request->arguments[4] || request->arguments[5])
                return GHOSTOS_RUNTIME_INVALID_REQUEST;
            break;
        case GHOSTOS_OP_SYN_FS_MAP:
            if ((request->flags & ~UINT16_C(2)) != 0 || request->arguments[5] == 0 ||
                (request->arguments[5] >> 48) != 0)
                return GHOSTOS_RUNTIME_INVALID_REQUEST;
            break;
        case GHOSTOS_OP_SYN_FS_RMDIR: case GHOSTOS_OP_SYN_FS_LINK: case GHOSTOS_OP_SYN_FS_LINKS:
        case GHOSTOS_OP_SYN_FS_DELETE:
            if (request->flags != 0 || request->arguments[4] || request->arguments[5])
                return GHOSTOS_RUNTIME_INVALID_REQUEST;
            break;
        case GHOSTOS_OP_SYN_FS_READ: case GHOSTOS_OP_SYN_FS_WRITE:
            if (request->flags != 0 || request->arguments[5] != 0)
                return GHOSTOS_RUNTIME_INVALID_REQUEST;
            break;
        case GHOSTOS_OP_SYN_FS_LIST:
            if (request->flags != 0 || request->arguments[5] == 0 ||
                request->arguments[5] > GHOSTOS_RUNTIME_LIST_PATH_REGION_BYTES ||
                *length <= GHOSTOS_RUNTIME_LIST_PATH_REGION_BYTES)
                return GHOSTOS_RUNTIME_INVALID_REQUEST;
            break;
        case GHOSTOS_OP_SYN_FS_CLOSE: case GHOSTOS_OP_SYN_FS_UNMAP: case GHOSTOS_OP_SYN_FS_METADATA:
            if (request->flags != 0) return GHOSTOS_RUNTIME_INVALID_REQUEST;
            for (size_t index = 4; index < 6; ++index) {
                if (request->arguments[index] != 0) return GHOSTOS_RUNTIME_INVALID_REQUEST;
            }
            break;
        default:
            return GHOSTOS_RUNTIME_INVALID_REQUEST;
    }
    return GHOSTOS_RUNTIME_OK;
}

ghostos_runtime_result ghostos_runtime_prepare_filesystem_request(
    const ghostos_runtime_state *state, uint32_t caller, const ghostos_request *request,
    size_t capacity, ghostos_runtime_filesystem_request *prepared) {
    if (!prepared) return GHOSTOS_RUNTIME_INVALID_REQUEST;
    ghostos_runtime_identity identity = {0};
    ghostos_runtime_result result = ghostos_runtime_lookup(state, caller, &identity, capacity);
    if (result != GHOSTOS_RUNTIME_OK) return result;
    uint32_t region, offset, length;
    bool writable, has_buffer;
    result = ghostos_runtime_validate_filesystem_request(request, &region, &offset, &length,
        &writable, &has_buffer);
    if (result != GHOSTOS_RUNTIME_OK) return result;
    uint32_t fs_operation;
    switch (request->operation) {
        case GHOSTOS_OP_SYN_FS_OPEN: fs_operation = 1; break;
        case GHOSTOS_OP_SYN_FS_CLOSE: fs_operation = 2; break;
        case GHOSTOS_OP_SYN_FS_READ: fs_operation = 3; break;
        case GHOSTOS_OP_SYN_FS_WRITE: fs_operation = 4; break;
        case GHOSTOS_OP_SYN_FS_METADATA: fs_operation = 5; break;
        case GHOSTOS_OP_SYN_FS_DELETE: fs_operation = 6; break;
        case GHOSTOS_OP_SYN_FS_LIST: fs_operation = 8; break;
        case GHOSTOS_OP_SYN_FS_MKDIR: fs_operation = 16; break;
        case GHOSTOS_OP_SYN_FS_RMDIR: fs_operation = 17; break;
        case GHOSTOS_OP_SYN_FS_LINK: fs_operation = 18; break;
        case GHOSTOS_OP_SYN_FS_LINKS: fs_operation = 19; break;
        case GHOSTOS_OP_SYN_FS_MAP: fs_operation = 25; break;
        case GHOSTOS_OP_SYN_FS_UNMAP: fs_operation = 26; break;
        default: return GHOSTOS_RUNTIME_INVALID_REQUEST;
    }
    bool ambient = request->operation == GHOSTOS_OP_SYN_FS_OPEN ||
        request->operation == GHOSTOS_OP_SYN_FS_LIST || request->operation == GHOSTOS_OP_SYN_FS_MKDIR ||
        request->operation == GHOSTOS_OP_SYN_FS_RMDIR || request->operation == GHOSTOS_OP_SYN_FS_LINKS ||
        request->operation == GHOSTOS_OP_SYN_FS_DELETE;
    *prepared = (ghostos_runtime_filesystem_request){
        fs_operation, request->flags, identity.process,
        ambient ? identity.authority : request->capability,
        request->arguments[4], request->arguments[5], region, offset, length,
        writable ? 1u : 0u, has_buffer ? 1u : 0u, 0};
    return GHOSTOS_RUNTIME_OK;
}

ghostos_runtime_result ghostos_runtime_validate_filesystem_response(uint16_t operation,
    uint32_t response_status, const uint64_t values[4], bool has_buffer, uint32_t buffer_length) {
    if (!values) return GHOSTOS_RUNTIME_INVALID_REQUEST;
    if ((response_status & 1u) == 0) return GHOSTOS_RUNTIME_OK;
    uint64_t length = has_buffer ? buffer_length : 0;
    switch (operation) {
        case GHOSTOS_OP_SYN_FS_OPEN:
            return (values[0] >> 32) ? GHOSTOS_RUNTIME_OK : GHOSTOS_RUNTIME_INVALID_CAPABILITY;
        case GHOSTOS_OP_SYN_FS_MAP:
            return (values[0] >> 32) && values[1] % 4096u == 0 && values[2] != 0 &&
                values[2] % 4096u == 0 && values[3] <= 1 ? GHOSTOS_RUNTIME_OK :
                GHOSTOS_RUNTIME_INVALID_CAPABILITY;
        case GHOSTOS_OP_SYN_FS_READ: case GHOSTOS_OP_SYN_FS_WRITE: case GHOSTOS_OP_SYN_FS_LINKS:
            return has_buffer && values[0] <= length ? GHOSTOS_RUNTIME_OK : GHOSTOS_RUNTIME_TRANSPORT_FAILURE;
        case GHOSTOS_OP_SYN_FS_LIST:
            return has_buffer && values[0] <= (length > GHOSTOS_RUNTIME_LIST_PATH_REGION_BYTES ?
                length - GHOSTOS_RUNTIME_LIST_PATH_REGION_BYTES : 0) ?
                GHOSTOS_RUNTIME_OK : GHOSTOS_RUNTIME_TRANSPORT_FAILURE;
        case GHOSTOS_OP_SYN_FS_CLOSE: case GHOSTOS_OP_SYN_FS_UNMAP: case GHOSTOS_OP_SYN_FS_METADATA:
        case GHOSTOS_OP_SYN_FS_MKDIR: case GHOSTOS_OP_SYN_FS_LINK:
            return GHOSTOS_RUNTIME_OK;
        case GHOSTOS_OP_SYN_FS_RMDIR:
            return values[3] <= 1 ? GHOSTOS_RUNTIME_OK : GHOSTOS_RUNTIME_TRANSPORT_FAILURE;
        case GHOSTOS_OP_SYN_FS_DELETE:
            return values[0] <= UINT32_MAX && values[1] <= 3 && values[2] <= UINT32_MAX && values[3] <= 1 ?
                GHOSTOS_RUNTIME_OK : GHOSTOS_RUNTIME_TRANSPORT_FAILURE;
        default:
            return GHOSTOS_RUNTIME_INVALID_REQUEST;
    }
}
