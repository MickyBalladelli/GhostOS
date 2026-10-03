#ifndef GHOSTOS_INFERENCE_SCHEDULE_H
#define GHOSTOS_INFERENCE_SCHEDULE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid request, 2 duplicate model, 3 capacity,
   4 model not found, 5 request not found, 6 stale sequence. */
#define GHOSTOS_INFERENCE_SCHEDULE_NAME 96u
typedef struct {
    uint8_t name[GHOSTOS_INFERENCE_SCHEDULE_NAME];
    uint8_t name_length;
    bool occupied;
    uint64_t bytes_per_token, primary_journal, replica_journal;
} ghostos_inference_model;
typedef struct {
    bool occupied;
    uint32_t generation;
    uint16_t model_slot;
    uint64_t request, prompt_tokens, max_generated_tokens;
} ghostos_inference_execution;
int ghostos_inference_register_model(ghostos_inference_model *models, size_t capacity, const uint8_t *name,
    size_t name_length, uint64_t bytes_per_token, uint64_t primary_journal, uint64_t replica_journal, size_t *slot);
int ghostos_inference_unregister_model(ghostos_inference_model *models, size_t model_capacity,
    ghostos_inference_execution *executions, size_t execution_capacity, const uint8_t *name, size_t name_length);
int ghostos_inference_prepare_start(const ghostos_inference_model *models, size_t model_capacity,
    const ghostos_inference_execution *executions, size_t execution_capacity, uint64_t request, const uint8_t *name,
    size_t name_length, uint64_t prompt_tokens, uint64_t max_generated_tokens, uint64_t lease_duration_us,
    size_t *model_slot, size_t *execution_slot, uint64_t *total_tokens);
int ghostos_inference_commit_start(ghostos_inference_execution *executions, size_t capacity, size_t execution_slot,
    size_t model_slot, uint64_t request, uint64_t prompt_tokens, uint64_t max_generated_tokens, uint64_t *handle);
int ghostos_inference_finish(ghostos_inference_execution *executions, size_t capacity, uint64_t handle);
int ghostos_inference_prepare_tokens(const ghostos_inference_execution *executions, size_t capacity, uint64_t handle,
    uint64_t generated_tokens, uint64_t cache_committed, uint64_t cache_reserved, uint64_t *committed_tokens);
#endif
