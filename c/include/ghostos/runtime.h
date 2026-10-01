#ifndef GHOSTOS_RUNTIME_H
#define GHOSTOS_RUNTIME_H

#include "ghostos/abi.h"

#define GHOSTOS_RUNTIME_MAX_FILESYSTEM_PROCESSES 64u
#define GHOSTOS_RUNTIME_MAX_IPC_BUFFER_BYTES 65536u
#define GHOSTOS_RUNTIME_LIST_PATH_REGION_BYTES 192u

typedef struct {
    uint32_t caller;
    uint32_t used;
    uint64_t process;
    uint64_t authority;
} ghostos_runtime_process_slot;

typedef struct {
    uint64_t clock;
    ghostos_runtime_process_slot processes[GHOSTOS_RUNTIME_MAX_FILESYSTEM_PROCESSES];
} ghostos_runtime_state;

_Static_assert(sizeof(ghostos_runtime_process_slot) == 24, "runtime process slot layout");
_Static_assert(sizeof(ghostos_runtime_state) == 1544, "runtime state layout");

typedef struct { uint64_t process, authority; } ghostos_runtime_identity;
typedef struct {
    uint32_t operation;
    uint32_t flags;
    uint64_t process;
    uint64_t capability;
    uint64_t offset;
    uint64_t length;
    uint32_t region;
    uint32_t buffer_offset;
    uint32_t buffer_length;
    uint32_t writable;
    uint32_t has_buffer;
    uint32_t reserved;
} ghostos_runtime_filesystem_request;

typedef enum {
    GHOSTOS_RUNTIME_OK,
    GHOSTOS_RUNTIME_ABI_MISMATCH,
    GHOSTOS_RUNTIME_INVALID_REQUEST,
    GHOSTOS_RUNTIME_PROCESS_NOT_REGISTERED,
    GHOSTOS_RUNTIME_CAPACITY,
    GHOSTOS_RUNTIME_INVALID_CAPABILITY,
    GHOSTOS_RUNTIME_INVALID_BUFFER,
    GHOSTOS_RUNTIME_TRANSPORT_FAILURE
} ghostos_runtime_result;

void ghostos_runtime_init(ghostos_runtime_state *state);
uint64_t ghostos_runtime_clock(const ghostos_runtime_state *state);
void ghostos_runtime_advance_clock(ghostos_runtime_state *state, uint64_t elapsed);
ghostos_runtime_result ghostos_runtime_register(ghostos_runtime_state *state,
    uint32_t caller, ghostos_runtime_identity identity, size_t capacity);
ghostos_runtime_result ghostos_runtime_unregister(ghostos_runtime_state *state, uint32_t caller,
    size_t capacity);
ghostos_runtime_result ghostos_runtime_lookup(const ghostos_runtime_state *state,
    uint32_t caller, ghostos_runtime_identity *identity, size_t capacity);
ghostos_runtime_result ghostos_runtime_prepare_filesystem_request(
    const ghostos_runtime_state *state, uint32_t caller, const ghostos_request *request,
    size_t capacity, ghostos_runtime_filesystem_request *prepared);
ghostos_runtime_result ghostos_runtime_validate_request(const ghostos_request *request);
bool ghostos_runtime_operation_delegated(uint16_t operation);
ghostos_runtime_result ghostos_runtime_validate_empty_request(const ghostos_request *request,
    bool capability_allowed);
ghostos_runtime_result ghostos_runtime_validate_filesystem_request(const ghostos_request *request,
    uint32_t *region, uint32_t *offset, uint32_t *length, bool *writable, bool *has_buffer);
ghostos_runtime_result ghostos_runtime_validate_filesystem_response(uint16_t operation,
    uint32_t response_status, const uint64_t values[4], bool has_buffer, uint32_t buffer_length);

#endif
