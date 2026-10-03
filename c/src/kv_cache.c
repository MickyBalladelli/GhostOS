#include "ghostos/kv_cache.h"
enum { GHOSTOS_KV_PAGE = 4096 };
static int align_up(uint64_t value, uint64_t alignment, uint64_t *aligned) {
    if (!alignment || value > UINT64_MAX - (alignment - 1)) return 5;
    *aligned = (value + alignment - 1) & ~(alignment - 1);
    return 0;
}
static int page_align(uint64_t bytes, uint64_t *aligned) {
    if (!bytes) return 5;
    return align_up(bytes, GHOSTOS_KV_PAGE, aligned);
}
static bool power_of_two(uint64_t value) {
    return value && (value & (value - 1)) == 0;
}
static int allocation_slot(const ghostos_kv_allocation *allocations, size_t capacity, uint64_t handle, size_t *slot) {
    uint32_t generation = (uint32_t)(handle >> 32);
    size_t index = (uint32_t)handle;
    if (index >= capacity) return 4;
    if (!generation || !allocations[index].occupied || allocations[index].generation != generation) return 1;
    *slot = index;
    return 0;
}
static int cache_slot(const ghostos_kv_cache *caches, size_t capacity, uint64_t handle, size_t *slot) {
    uint32_t generation = (uint32_t)(handle >> 32);
    size_t index = (uint32_t)handle;
    if (index >= capacity) return 4;
    if (!generation || !caches[index].occupied || caches[index].generation != generation) return 2;
    *slot = index;
    return 0;
}
static int place_range(const ghostos_kv_allocation *allocations, size_t capacity, uint64_t window_start, uint64_t window_length,
    uint64_t length, uint64_t alignment, uint64_t *start) {
    uint64_t window_end, cursor;
    int status = align_up(window_start, alignment, &cursor);
    if (status) return status;
    if (window_start > UINT64_MAX - window_length) return 5;
    window_end = window_start + window_length;
    for (;;) {
        uint64_t end, next;
        size_t i;
        bool conflict = false;
        if (cursor > UINT64_MAX - length) return 5;
        end = cursor + length;
        if (end > window_end) return 3;
        for (i = 0; i < capacity; ++i) {
            uint64_t occupied_end;
            if (!allocations[i].occupied) continue;
            if (allocations[i].start > UINT64_MAX - allocations[i].length) return 5;
            occupied_end = allocations[i].start + allocations[i].length;
            if (allocations[i].start < end && cursor < occupied_end) {
                status = align_up(occupied_end, alignment, &next);
                if (status) return status;
                if (next <= cursor) return 3;
                cursor = next;
                conflict = true;
                break;
            }
        }
        if (!conflict) { *start = cursor; return 0; }
    }
}
int ghostos_kv_allocate(ghostos_kv_allocation *allocations, size_t capacity, uint64_t window_start, uint64_t window_length,
    uint64_t bytes, uint64_t alignment, uint64_t *handle, uint64_t *start, uint64_t *length) {
    uint64_t aligned = 0, placed = 0;
    size_t slot = 0, i;
    uint32_t generation;
    bool found = false;
    int status = page_align(bytes, &aligned);
    if (status) return status;
    if (alignment < GHOSTOS_KV_PAGE || !power_of_two(alignment)) return 5;
    status = place_range(allocations, capacity, window_start, window_length, aligned, alignment, &placed);
    if (status) return status;
    for (i = 0; i < capacity; ++i) if (!allocations[i].occupied) { slot = i; found = true; break; }
    if (!found) return 3;
    generation = allocations[slot].generation + 1;
    if (!generation) generation = 1;
    allocations[slot].occupied = true;
    allocations[slot].generation = generation;
    allocations[slot].start = placed;
    allocations[slot].length = aligned;
    *handle = ((uint64_t)generation << 32) | slot;
    *start = placed;
    *length = aligned;
    return 0;
}
int ghostos_kv_resolve(const ghostos_kv_allocation *allocations, size_t capacity, uint64_t handle, uint64_t offset) {
    size_t slot = 0;
    int status = allocation_slot(allocations, capacity, handle, &slot);
    if (status) return status;
    if (offset >= allocations[slot].length) return 5;
    return 0;
}
int ghostos_kv_release(ghostos_kv_allocation *allocations, size_t capacity, uint64_t handle) {
    size_t slot = 0;
    int status = allocation_slot(allocations, capacity, handle, &slot);
    if (status) return status;
    allocations[slot].occupied = false;
    return 0;
}
int ghostos_kv_info(const ghostos_kv_allocation *allocations, size_t capacity, uint64_t handle, uint64_t *length) {
    size_t slot = 0;
    int status = allocation_slot(allocations, capacity, handle, &slot);
    if (status) return status;
    *length = allocations[slot].length;
    return 0;
}
int ghostos_kv_open(ghostos_kv_cache *caches, size_t capacity, uint64_t request, uint64_t bytes_per_token,
    uint64_t initial_tokens, uint64_t *handle, uint64_t *reserved_tokens) {
    size_t slot = 0, i;
    uint32_t generation;
    bool found = false;
    if (!bytes_per_token || !initial_tokens) return 5;
    for (i = 0; i < capacity; ++i) if (caches[i].occupied && caches[i].request == request) return 4;
    for (i = 0; i < capacity; ++i) if (!caches[i].occupied) { slot = i; found = true; break; }
    if (!found) return 3;
    if (bytes_per_token > UINT64_MAX / initial_tokens) return 5;
    generation = caches[slot].generation + 1;
    if (!generation) generation = 1;
    caches[slot].occupied = true;
    caches[slot].generation = generation;
    caches[slot].request = request;
    caches[slot].bytes_per_token = bytes_per_token;
    caches[slot].reserved_tokens = initial_tokens;
    caches[slot].committed_tokens = 0;
    *handle = ((uint64_t)generation << 32) | slot;
    *reserved_tokens = initial_tokens;
    return 0;
}
int ghostos_kv_commit(ghostos_kv_cache *caches, size_t capacity, uint64_t handle, uint64_t total_tokens) {
    size_t slot = 0;
    int status = cache_slot(caches, capacity, handle, &slot);
    if (status) return status;
    if (total_tokens < caches[slot].committed_tokens || total_tokens > caches[slot].reserved_tokens) return 5;
    caches[slot].committed_tokens = total_tokens;
    return 0;
}
int ghostos_kv_token(const ghostos_kv_cache *caches, size_t capacity, uint64_t handle, uint64_t token, uint64_t byte_in_token) {
    size_t slot = 0;
    int status = cache_slot(caches, capacity, handle, &slot);
    if (status) return status;
    if (token >= caches[slot].reserved_tokens || byte_in_token >= caches[slot].bytes_per_token) return 5;
    return 0;
}
