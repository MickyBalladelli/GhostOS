#include "ghostos/inference_schedule.h"
static bool same_name(const uint8_t *left, size_t left_length, const uint8_t *right, size_t right_length) {
    size_t i;
    if (left_length != right_length) return false;
    for (i = 0; i < left_length; ++i) if (left[i] != right[i]) return false;
    return true;
}
static int find_model(const ghostos_inference_model *models, size_t capacity, const uint8_t *name, size_t name_length, size_t *slot) {
    size_t i;
    for (i = 0; i < capacity; ++i) {
        if (models[i].occupied && same_name(models[i].name, models[i].name_length, name, name_length)) {
            *slot = i;
            return 0;
        }
    }
    return 4;
}
static int valid_execution(const ghostos_inference_execution *executions, size_t capacity, uint64_t handle, size_t *slot) {
    uint32_t generation = (uint32_t)(handle >> 32);
    size_t index = (uint32_t)handle;
    if (!generation || index >= capacity || !executions[index].occupied || executions[index].generation != generation) return 5;
    *slot = index;
    return 0;
}
int ghostos_inference_register_model(ghostos_inference_model *models, size_t capacity, const uint8_t *name,
    size_t name_length, uint64_t bytes_per_token, uint64_t primary_journal, uint64_t replica_journal, size_t *slot) {
    size_t existing = 0, free_slot = 0, i;
    bool duplicate, found = false;
    if (!name_length || name_length > GHOSTOS_INFERENCE_SCHEDULE_NAME) return 1;
    duplicate = !find_model(models, capacity, name, name_length, &existing);
    if (!bytes_per_token || primary_journal == replica_journal || duplicate) return duplicate ? 2 : 1;
    for (i = 0; i < capacity; ++i) if (!models[i].occupied) { free_slot = i; found = true; break; }
    if (!found) return 3;
    for (i = 0; i < name_length; ++i) models[free_slot].name[i] = name[i];
    models[free_slot].name_length = (uint8_t)name_length;
    models[free_slot].occupied = true;
    models[free_slot].bytes_per_token = bytes_per_token;
    models[free_slot].primary_journal = primary_journal;
    models[free_slot].replica_journal = replica_journal;
    *slot = free_slot;
    return 0;
}
int ghostos_inference_unregister_model(ghostos_inference_model *models, size_t model_capacity,
    ghostos_inference_execution *executions, size_t execution_capacity, const uint8_t *name, size_t name_length) {
    size_t slot = 0, i;
    int status = find_model(models, model_capacity, name, name_length, &slot);
    if (status) return status;
    for (i = 0; i < execution_capacity; ++i) if (executions[i].occupied && executions[i].model_slot == slot) return 3;
    models[slot].occupied = false;
    return 0;
}
int ghostos_inference_prepare_start(const ghostos_inference_model *models, size_t model_capacity,
    const ghostos_inference_execution *executions, size_t execution_capacity, uint64_t request, const uint8_t *name,
    size_t name_length, uint64_t prompt_tokens, uint64_t max_generated_tokens, uint64_t lease_duration_us,
    size_t *model_slot, size_t *execution_slot, uint64_t *total_tokens) {
    size_t i;
    bool found = false;
    int status;
    if (!prompt_tokens || !max_generated_tokens || !lease_duration_us) return 1;
    for (i = 0; i < execution_capacity; ++i) if (executions[i].occupied && executions[i].request == request) return 1;
    if (prompt_tokens > UINT64_MAX - max_generated_tokens) return 1;
    status = find_model(models, model_capacity, name, name_length, model_slot);
    if (status) return status;
    for (i = 0; i < execution_capacity; ++i) if (!executions[i].occupied) { *execution_slot = i; found = true; break; }
    if (!found) return 3;
    *total_tokens = prompt_tokens + max_generated_tokens;
    return 0;
}
int ghostos_inference_commit_start(ghostos_inference_execution *executions, size_t capacity, size_t execution_slot,
    size_t model_slot, uint64_t request, uint64_t prompt_tokens, uint64_t max_generated_tokens, uint64_t *handle) {
    uint32_t generation;
    if (execution_slot >= capacity || model_slot > UINT16_MAX) return 5;
    if (executions[execution_slot].occupied) return 3;
    generation = executions[execution_slot].generation + 1;
    if (!generation) generation = 1;
    executions[execution_slot].occupied = true;
    executions[execution_slot].generation = generation;
    executions[execution_slot].model_slot = (uint16_t)model_slot;
    executions[execution_slot].request = request;
    executions[execution_slot].prompt_tokens = prompt_tokens;
    executions[execution_slot].max_generated_tokens = max_generated_tokens;
    *handle = ((uint64_t)generation << 32) | execution_slot;
    return 0;
}
int ghostos_inference_finish(ghostos_inference_execution *executions, size_t capacity, uint64_t handle) {
    size_t slot = 0;
    int status = valid_execution(executions, capacity, handle, &slot);
    if (status) return status;
    executions[slot].occupied = false;
    return 0;
}
int ghostos_inference_prepare_tokens(const ghostos_inference_execution *executions, size_t capacity, uint64_t handle,
    uint64_t generated_tokens, uint64_t cache_committed, uint64_t cache_reserved, uint64_t *committed_tokens) {
    size_t slot = 0;
    uint64_t committed;
    int status = valid_execution(executions, capacity, handle, &slot);
    if (status) return status;
    if (generated_tokens > executions[slot].max_generated_tokens) return 1;
    if (executions[slot].prompt_tokens > UINT64_MAX - generated_tokens) return 1;
    committed = executions[slot].prompt_tokens + generated_tokens;
    if (committed < cache_committed || committed > cache_reserved) return 6;
    *committed_tokens = committed;
    return 0;
}
