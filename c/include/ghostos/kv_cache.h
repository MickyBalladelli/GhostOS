#ifndef GHOSTOS_KV_CACHE_H
#define GHOSTOS_KV_CACHE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 allocation not found, 2 cache not found, 3 capacity,
   4 invalid handle, 5 invalid range. Page size is 4096. */
typedef struct {
    bool occupied;
    uint32_t generation;
    uint64_t start, length;
} ghostos_kv_allocation;
typedef struct {
    bool occupied;
    uint32_t generation;
    uint64_t request, bytes_per_token, reserved_tokens, committed_tokens;
} ghostos_kv_cache;
int ghostos_kv_allocate(ghostos_kv_allocation *allocations, size_t capacity, uint64_t window_start, uint64_t window_length,
    uint64_t bytes, uint64_t alignment, uint64_t *handle, uint64_t *start, uint64_t *length);
int ghostos_kv_resolve(const ghostos_kv_allocation *allocations, size_t capacity, uint64_t handle, uint64_t offset);
int ghostos_kv_release(ghostos_kv_allocation *allocations, size_t capacity, uint64_t handle);
int ghostos_kv_info(const ghostos_kv_allocation *allocations, size_t capacity, uint64_t handle, uint64_t *length);
int ghostos_kv_open(ghostos_kv_cache *caches, size_t capacity, uint64_t request, uint64_t bytes_per_token,
    uint64_t initial_tokens, uint64_t *handle, uint64_t *reserved_tokens);
int ghostos_kv_commit(ghostos_kv_cache *caches, size_t capacity, uint64_t handle, uint64_t total_tokens);
int ghostos_kv_token(const ghostos_kv_cache *caches, size_t capacity, uint64_t handle, uint64_t token, uint64_t byte_in_token);
#endif
