#ifndef GHOSTOS_INFERENCE_JOURNAL_H
#define GHOSTOS_INFERENCE_JOURNAL_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 no failover replica, 2 invalid handle, 3 capacity,
   4 request not found, 5 stale checkpoint, 6 corrupt recovery record.
   State: replicating=0, running=1, degraded=2, complete=3. */
#define GHOSTOS_INFERENCE_RECOVERY_BYTES 68u
typedef struct {
    uint64_t request, model, kv_cache, next_token, rng_state, epoch;
    uint32_t primary, replica;
} ghostos_inference_record;
typedef struct {
    bool occupied, has_committed, has_pending;
    uint32_t generation;
    uint8_t state;
    uint64_t failed_nodes;
    ghostos_inference_record committed, pending;
} ghostos_inference_journal;
typedef struct {
    uint32_t primary, replica;
    uint64_t epoch;
    uint8_t bytes[GHOSTOS_INFERENCE_RECOVERY_BYTES];
} ghostos_inference_checkpoint;
int ghostos_inference_journal_begin(ghostos_inference_journal *entries, size_t capacity, uint64_t request, uint64_t model,
    uint64_t kv_cache, uint32_t primary, uint32_t replica, uint64_t rng_state, uint64_t *handle,
    ghostos_inference_checkpoint *checkpoint);
int ghostos_inference_journal_acknowledge(ghostos_inference_journal *entries, size_t capacity, uint64_t handle,
    uint64_t epoch, bool primary_acknowledged, bool replica_acknowledged, uint8_t *state);
int ghostos_inference_journal_prepare(ghostos_inference_journal *entries, size_t capacity, uint64_t handle,
    uint64_t next_token, uint64_t rng_state, ghostos_inference_checkpoint *checkpoint);
int ghostos_inference_journal_fail(ghostos_inference_journal *entries, size_t capacity, uint64_t handle, uint32_t failed,
    uint32_t *journal_node);
int ghostos_inference_journal_repair(ghostos_inference_journal *entries, size_t capacity, uint64_t handle,
    uint32_t replacement, ghostos_inference_checkpoint *checkpoint);
int ghostos_inference_recovery_decode(const uint8_t *bytes, size_t length, uint64_t *rng_state);
#endif
