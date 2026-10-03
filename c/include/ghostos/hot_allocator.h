#ifndef GHOSTOS_HOT_ALLOCATOR_H
#define GHOSTOS_HOT_ALLOCATOR_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_HOT_ALLOCATOR_MAX_CPUS 128u
#define GHOSTOS_HOT_ALLOCATOR_MAX_NODES 64u
#define GHOSTOS_HOT_ALLOCATOR_MAX_POOL_SLOTS 256u
#define GHOSTOS_HOT_ALLOCATOR_PROBE_BUCKETS 8u
#define GHOSTOS_HOT_ALLOCATOR_KIND_COUNT 4u

typedef enum { GHOSTOS_HOT_IPC, GHOSTOS_HOT_PACKET, GHOSTOS_HOT_TIMER, GHOSTOS_HOT_SCHEDULER } ghostos_hot_object_kind;
typedef enum { GHOSTOS_HOT_CONFIG_OK, GHOSTOS_HOT_NO_CPUS, GHOSTOS_HOT_NO_NODES,
    GHOSTOS_HOT_INVALID_NODE, GHOSTOS_HOT_INVALID_CAPACITY, GHOSTOS_HOT_INVALID_FALLBACK_LIMIT } ghostos_hot_config_error;
typedef enum { GHOSTOS_HOT_ALLOC_OK, GHOSTOS_HOT_INVALID_CPU, GHOSTOS_HOT_EXHAUSTED } ghostos_hot_allocation_error;
typedef enum { GHOSTOS_HOT_RECLAIM_OK, GHOSTOS_HOT_RECLAIM_INVALID_CPU, GHOSTOS_HOT_RECLAIM_INVALID_NODE,
    GHOSTOS_HOT_RECLAIM_INVALID_SLOT, GHOSTOS_HOT_RECLAIM_ALREADY_FREE } ghostos_hot_reclaim_error;
typedef enum { GHOSTOS_HOT_LOCAL_CPU, GHOSTOS_HOT_LOCAL_NODE, GHOSTOS_HOT_REMOTE_NODE } ghostos_hot_placement;

typedef struct {
    ghostos_hot_object_kind kind;
    uint8_t node;
    uint16_t cpu, slot;
    ghostos_hot_placement placement;
} ghostos_hot_allocation;

typedef struct {
    uint64_t allocations, allocation_failures;
    uint64_t local_cpu_allocations, local_node_allocations, remote_node_allocations;
    uint64_t remote_memory_bytes, total_probe_steps;
    uint16_t max_probe_steps;
    uint64_t probe_buckets[GHOSTOS_HOT_ALLOCATOR_PROBE_BUCKETS];
    uint64_t reclaims, remote_reclaims, invalid_reclaims;
} ghostos_hot_allocator_stats;

typedef struct {
    ghostos_hot_object_kind kind;
    uint32_t capacity, in_use, free, largest_free_run;
    uint16_t fragmentation_per_mille;
    ghostos_hot_allocator_stats stats;
} ghostos_hot_allocator_report;

typedef struct { uint16_t capacity, used; uint64_t free[4]; } ghostos_hot_pool;
typedef struct {
    size_t cpu_count, node_capacity;
    uint8_t node_count, max_cross_node_fallbacks;
    uint8_t cpu_to_node[GHOSTOS_HOT_ALLOCATOR_MAX_CPUS];
    ghostos_hot_pool pools[GHOSTOS_HOT_ALLOCATOR_KIND_COUNT][GHOSTOS_HOT_ALLOCATOR_MAX_NODES][GHOSTOS_HOT_ALLOCATOR_MAX_CPUS];
    ghostos_hot_allocator_stats stats[GHOSTOS_HOT_ALLOCATOR_KIND_COUNT];
} ghostos_hot_allocator;

ghostos_hot_config_error ghostos_hot_allocator_init(ghostos_hot_allocator *allocator,
    const uint8_t *cpu_to_node, size_t cpu_count, size_t node_count,
    size_t per_cpu_capacity, size_t max_cross_node_fallbacks);
ghostos_hot_allocation_error ghostos_hot_allocate(ghostos_hot_allocator *allocator,
    ghostos_hot_object_kind kind, size_t cpu, ghostos_hot_allocation *out);
ghostos_hot_reclaim_error ghostos_hot_reclaim(ghostos_hot_allocator *allocator, ghostos_hot_allocation allocation);
ghostos_hot_reclaim_error ghostos_hot_reclaim_on(ghostos_hot_allocator *allocator,
    size_t current_cpu, ghostos_hot_allocation allocation);
void ghostos_hot_record_remote_memory(ghostos_hot_allocator *allocator, ghostos_hot_object_kind kind, uint64_t bytes);
bool ghostos_hot_report(const ghostos_hot_allocator *allocator, ghostos_hot_object_kind kind, ghostos_hot_allocator_report *out);
bool ghostos_hot_cpu_node(const ghostos_hot_allocator *allocator, size_t cpu, uint8_t *node);
size_t ghostos_hot_node_count(const ghostos_hot_allocator *allocator);
size_t ghostos_hot_max_cross_node_fallbacks(const ghostos_hot_allocator *allocator);

/* Borrowed, contiguous kind/node/CPU storage for generic kernel consumers. */
typedef struct {
    size_t cpu_count, node_capacity, cpu_stride, node_stride;
    uint8_t node_count, max_cross_node_fallbacks;
    const uint8_t *cpu_to_node;
    ghostos_hot_pool *pools;
    ghostos_hot_allocator_stats *stats;
    bool checked;
} ghostos_hot_allocator_view;
void ghostos_hot_pool_init(ghostos_hot_pool *pool, size_t capacity);
ghostos_hot_config_error ghostos_hot_validate(const uint8_t *cpu_to_node,
    size_t cpu_count, size_t node_capacity, size_t node_count,
    size_t capacity, size_t fallbacks);
/* Allocation result 3 reports a zero-divisor caused by the legacy u8 node count. */
ghostos_hot_allocation_error ghostos_hot_view_allocate(ghostos_hot_allocator_view *view,
    ghostos_hot_object_kind kind, size_t cpu, ghostos_hot_allocation *out);
ghostos_hot_reclaim_error ghostos_hot_view_reclaim_on(ghostos_hot_allocator_view *view,
    size_t current_cpu, ghostos_hot_allocation allocation);
void ghostos_hot_view_record_remote_memory(ghostos_hot_allocator_view *view,
    ghostos_hot_object_kind kind, uint64_t bytes);
/* False also reports checked u32 report-counter overflow. */
bool ghostos_hot_view_report(const ghostos_hot_allocator_view *view,
    ghostos_hot_object_kind kind, ghostos_hot_allocator_report *out);

#endif
