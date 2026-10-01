#ifndef GHOSTOS_RUNTIME_H
#define GHOSTOS_RUNTIME_H

#include "ghostos/abi.h"

#define GHOSTOS_RUNTIME_MAX_FILESYSTEM_PROCESSES 64u

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

typedef struct { uint64_t process, authority; } ghostos_runtime_identity;

typedef enum {
    GHOSTOS_RUNTIME_OK,
    GHOSTOS_RUNTIME_ABI_MISMATCH,
    GHOSTOS_RUNTIME_INVALID_REQUEST,
    GHOSTOS_RUNTIME_PROCESS_NOT_REGISTERED,
    GHOSTOS_RUNTIME_CAPACITY
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
ghostos_runtime_result ghostos_runtime_validate_request(const ghostos_request *request);
bool ghostos_runtime_operation_delegated(uint16_t operation);

#endif
