#include "ghostos/hot_allocator.h"

static bool valid_kind(ghostos_hot_object_kind kind) { return (unsigned)kind < GHOSTOS_HOT_ALLOCATOR_KIND_COUNT; }
static uint64_t sat_add(uint64_t a, uint64_t b) { return UINT64_MAX - a < b ? UINT64_MAX : a + b; }

void ghostos_hot_pool_init(ghostos_hot_pool *pool, size_t capacity) {
    *pool = (ghostos_hot_pool){0};
    pool->capacity = (uint16_t)capacity;
    size_t full = capacity / 64, remainder = capacity % 64;
    for (size_t i = 0; i < full; ++i) pool->free[i] = UINT64_MAX;
    if (remainder) pool->free[full] = (UINT64_C(1) << remainder) - 1;
}

static bool pool_take(ghostos_hot_pool *pool, uint16_t *slot) {
    for (size_t word = 0; word < 4; ++word) {
        uint64_t available = pool->free[word];
        if (!available) continue;
        unsigned bit = 0;
        while (((available >> bit) & 1u) == 0) ++bit;
        pool->free[word] &= ~(UINT64_C(1) << bit);
        ++pool->used;
        *slot = (uint16_t)(word * 64 + bit);
        return true;
    }
    return false;
}

static ghostos_hot_reclaim_error pool_release(ghostos_hot_pool *pool, uint16_t slot) {
    if (slot >= pool->capacity || slot >= GHOSTOS_HOT_ALLOCATOR_MAX_POOL_SLOTS) return GHOSTOS_HOT_RECLAIM_INVALID_SLOT;
    size_t word = slot / 64, bit = slot % 64;
    uint64_t mask = UINT64_C(1) << bit;
    if (pool->free[word] & mask) return GHOSTOS_HOT_RECLAIM_ALREADY_FREE;
    pool->free[word] |= mask;
    --pool->used;
    return GHOSTOS_HOT_RECLAIM_OK;
}

static void record_allocation(ghostos_hot_allocator_view *a, size_t kind, ghostos_hot_placement placement, uint16_t probes) {
    ghostos_hot_allocator_stats *s = &a->stats[kind];
    s->allocations = sat_add(s->allocations, 1);
    s->total_probe_steps = sat_add(s->total_probe_steps, probes);
    if (probes > s->max_probe_steps) s->max_probe_steps = probes;
    size_t bucket = probes ? probes - 1 : 0;
    if (bucket >= GHOSTOS_HOT_ALLOCATOR_PROBE_BUCKETS) bucket = GHOSTOS_HOT_ALLOCATOR_PROBE_BUCKETS - 1;
    s->probe_buckets[bucket] = sat_add(s->probe_buckets[bucket], 1);
    if (placement == GHOSTOS_HOT_LOCAL_CPU) s->local_cpu_allocations = sat_add(s->local_cpu_allocations, 1);
    else if (placement == GHOSTOS_HOT_LOCAL_NODE) s->local_node_allocations = sat_add(s->local_node_allocations, 1);
    else s->remote_node_allocations = sat_add(s->remote_node_allocations, 1);
}

ghostos_hot_config_error ghostos_hot_allocator_init(ghostos_hot_allocator *a,
    const uint8_t *cpu_to_node, size_t cpu_count, size_t node_count,
    size_t per_cpu_capacity, size_t fallbacks) {
    if (!cpu_count) return GHOSTOS_HOT_NO_CPUS;
    if (!node_count) return GHOSTOS_HOT_NO_NODES;
    if (cpu_count > GHOSTOS_HOT_ALLOCATOR_MAX_CPUS || node_count > GHOSTOS_HOT_ALLOCATOR_MAX_NODES || !cpu_to_node)
        return GHOSTOS_HOT_INVALID_NODE;
    if (!per_cpu_capacity || per_cpu_capacity > GHOSTOS_HOT_ALLOCATOR_MAX_POOL_SLOTS)
        return GHOSTOS_HOT_INVALID_CAPACITY;
    if (fallbacks > node_count - 1 || fallbacks > UINT8_MAX) return GHOSTOS_HOT_INVALID_FALLBACK_LIMIT;
    for (size_t cpu = 0; cpu < cpu_count; ++cpu) if (cpu_to_node[cpu] >= node_count) return GHOSTOS_HOT_INVALID_NODE;
    *a = (ghostos_hot_allocator){0};
    a->cpu_count = cpu_count;
    a->node_capacity = node_count;
    a->node_count = (uint8_t)node_count;
    a->max_cross_node_fallbacks = (uint8_t)fallbacks;
    for (size_t cpu = 0; cpu < cpu_count; ++cpu) a->cpu_to_node[cpu] = cpu_to_node[cpu];
    for (size_t kind = 0; kind < GHOSTOS_HOT_ALLOCATOR_KIND_COUNT; ++kind)
        for (size_t node = 0; node < node_count; ++node)
            for (size_t cpu = 0; cpu < cpu_count; ++cpu) ghostos_hot_pool_init(&a->pools[kind][node][cpu], per_cpu_capacity);
    return GHOSTOS_HOT_CONFIG_OK;
}

ghostos_hot_allocation_error ghostos_hot_view_allocate(ghostos_hot_allocator_view *a,
    ghostos_hot_object_kind kind, size_t cpu, ghostos_hot_allocation *out) {
    if (!valid_kind(kind)) return GHOSTOS_HOT_EXHAUSTED;
    if (cpu >= a->cpu_count) return GHOSTOS_HOT_INVALID_CPU;
    size_t node = a->cpu_to_node[cpu], k = kind;
    uint16_t probes = 1, slot;
    if (pool_take(&a->pools[((k) * a->node_stride + (node)) * a->cpu_stride + (cpu)], &slot)) {
        record_allocation(a, k, GHOSTOS_HOT_LOCAL_CPU, probes);
        if (out) *out = (ghostos_hot_allocation){kind, (uint8_t)node, (uint16_t)cpu, slot, GHOSTOS_HOT_LOCAL_CPU};
        return GHOSTOS_HOT_ALLOC_OK;
    }
    for (size_t distance = 1; distance < a->cpu_count; ++distance) {
        size_t candidate = (cpu + distance) % a->cpu_count;
        if (probes != UINT16_MAX) ++probes;
        if (a->cpu_to_node[candidate] == node && pool_take(&a->pools[((k) * a->node_stride + (node)) * a->cpu_stride + (candidate)], &slot)) {
            record_allocation(a, k, GHOSTOS_HOT_LOCAL_NODE, probes);
            if (out) *out = (ghostos_hot_allocation){kind, (uint8_t)node, (uint16_t)candidate, slot, GHOSTOS_HOT_LOCAL_NODE};
            return GHOSTOS_HOT_ALLOC_OK;
        }
    }
    if (a->max_cross_node_fallbacks && !a->node_count) return (ghostos_hot_allocation_error)3;
    for (size_t fallback = 0; fallback < a->max_cross_node_fallbacks; ++fallback) {
        size_t remote_node = (node + fallback + 1) % a->node_count;
        for (size_t remote_cpu = 0; remote_cpu < a->cpu_count; ++remote_cpu) {
            if (probes != UINT16_MAX) ++probes;
            if (a->cpu_to_node[remote_cpu] == remote_node && pool_take(&a->pools[((k) * a->node_stride + (remote_node)) * a->cpu_stride + (remote_cpu)], &slot)) {
                record_allocation(a, k, GHOSTOS_HOT_REMOTE_NODE, probes);
                if (out) *out = (ghostos_hot_allocation){kind, (uint8_t)remote_node, (uint16_t)remote_cpu, slot, GHOSTOS_HOT_REMOTE_NODE};
                return GHOSTOS_HOT_ALLOC_OK;
            }
        }
    }
    a->stats[k].allocation_failures = sat_add(a->stats[k].allocation_failures, 1);
    return GHOSTOS_HOT_EXHAUSTED;
}

ghostos_hot_reclaim_error ghostos_hot_view_reclaim_on(ghostos_hot_allocator_view *a, size_t current_cpu, ghostos_hot_allocation allocation) {
    if (!valid_kind(allocation.kind)) return GHOSTOS_HOT_RECLAIM_INVALID_SLOT;
    if (current_cpu >= a->cpu_count) return GHOSTOS_HOT_RECLAIM_INVALID_CPU;
    size_t cpu = allocation.cpu, node = allocation.node, kind = allocation.kind;
    if (cpu >= a->cpu_count) goto invalid_cpu;
    if (node >= a->node_count || a->cpu_to_node[cpu] != node) {
        a->stats[kind].invalid_reclaims = sat_add(a->stats[kind].invalid_reclaims, 1);
        return GHOSTOS_HOT_RECLAIM_INVALID_NODE;
    }
    ghostos_hot_reclaim_error result = pool_release(&a->pools[((kind) * a->node_stride + (node)) * a->cpu_stride + (cpu)], allocation.slot);
    if (result != GHOSTOS_HOT_RECLAIM_OK) {
        a->stats[kind].invalid_reclaims = sat_add(a->stats[kind].invalid_reclaims, 1);
        return result;
    }
    a->stats[kind].reclaims = sat_add(a->stats[kind].reclaims, 1);
    if (current_cpu != cpu) a->stats[kind].remote_reclaims = sat_add(a->stats[kind].remote_reclaims, 1);
    return GHOSTOS_HOT_RECLAIM_OK;
invalid_cpu:
    a->stats[kind].invalid_reclaims = sat_add(a->stats[kind].invalid_reclaims, 1);
    return GHOSTOS_HOT_RECLAIM_INVALID_CPU;
}

void ghostos_hot_view_record_remote_memory(ghostos_hot_allocator_view *a, ghostos_hot_object_kind kind, uint64_t bytes) {
    if (valid_kind(kind) && bytes) a->stats[kind].remote_memory_bytes = sat_add(a->stats[kind].remote_memory_bytes, bytes);
}

bool ghostos_hot_view_report(const ghostos_hot_allocator_view *a, ghostos_hot_object_kind kind, ghostos_hot_allocator_report *out) {
    if (!valid_kind(kind) || !out) return false;
    uint32_t capacity = 0, in_use = 0, free_count = 0, largest = 0;
    for (size_t node = 0; node < a->node_capacity; ++node) for (size_t cpu = 0; cpu < a->cpu_count; ++cpu) {
        if (a->cpu_to_node[cpu] != node) continue;
        const ghostos_hot_pool *pool = &a->pools[((kind) * a->node_stride + (node)) * a->cpu_stride + (cpu)];
        if (a->checked && (UINT32_MAX - capacity < pool->capacity ||
            UINT32_MAX - in_use < pool->used ||
            UINT32_MAX - free_count < (uint32_t)(pool->capacity - pool->used))) return false;
        capacity += pool->capacity;
        in_use += pool->used;
        free_count += pool->capacity - pool->used;
        uint32_t run = 0, pool_largest = 0;
        for (size_t slot = 0; slot < pool->capacity; ++slot) {
            if (pool->free[slot / 64] & (UINT64_C(1) << (slot % 64))) { ++run; if (run > pool_largest) pool_largest = run; }
            else run = 0;
        }
        if (a->checked && UINT32_MAX - largest < pool_largest) return false;
        largest += pool_largest;
    }
    uint32_t fragmented = free_count > largest ? free_count - largest : 0;
    uint16_t fragmentation = free_count ? (uint16_t)(((uint64_t)fragmented * 1000) / free_count) : 0;
    *out = (ghostos_hot_allocator_report){kind, capacity, in_use, free_count, largest, fragmentation, a->stats[kind]};
    return true;
}

bool ghostos_hot_cpu_node(const ghostos_hot_allocator *a, size_t cpu, uint8_t *node) {
    if (cpu >= a->cpu_count) return false;
    if (node) *node = a->cpu_to_node[cpu];
    return true;
}
size_t ghostos_hot_node_count(const ghostos_hot_allocator *a) { return a->node_count; }
size_t ghostos_hot_max_cross_node_fallbacks(const ghostos_hot_allocator *a) { return a->max_cross_node_fallbacks; }

_Static_assert(sizeof(ghostos_hot_pool) == 40, "hot pool ABI");
_Static_assert(offsetof(ghostos_hot_pool, free) == 8, "hot bitmap ABI");
_Static_assert(sizeof(ghostos_hot_allocator_stats) == 152, "hot stats ABI");
_Static_assert(offsetof(ghostos_hot_allocator_stats, probe_buckets) == 64, "hot probes ABI");
_Static_assert(sizeof(ghostos_hot_allocation) == 16, "hot token ABI");
_Static_assert(sizeof(ghostos_hot_allocator_report) == 176, "hot report ABI");
_Static_assert(sizeof(ghostos_hot_allocator_view) == 72, "hot view ABI");

ghostos_hot_config_error ghostos_hot_validate(const uint8_t *map,
    size_t cpus, size_t nodes, size_t node_count, size_t capacity, size_t fallbacks) {
    if (!cpus) return GHOSTOS_HOT_NO_CPUS;
    if (!node_count || !nodes) return GHOSTOS_HOT_NO_NODES;
    if (node_count > nodes) return GHOSTOS_HOT_INVALID_NODE;
    if (!capacity || capacity > GHOSTOS_HOT_ALLOCATOR_MAX_POOL_SLOTS) return GHOSTOS_HOT_INVALID_CAPACITY;
    if (fallbacks > node_count - 1) return GHOSTOS_HOT_INVALID_FALLBACK_LIMIT;
    for (size_t cpu = 0; cpu < cpus; ++cpu)
        if (map[cpu] >= node_count) return GHOSTOS_HOT_INVALID_NODE;
    return GHOSTOS_HOT_CONFIG_OK;
}


static ghostos_hot_allocator_view bounded_view(ghostos_hot_allocator *a) {
    return (ghostos_hot_allocator_view){a->cpu_count, a->node_capacity,
        GHOSTOS_HOT_ALLOCATOR_MAX_CPUS, GHOSTOS_HOT_ALLOCATOR_MAX_NODES,
        a->node_count, a->max_cross_node_fallbacks,
        a->cpu_to_node, &a->pools[0][0][0], a->stats, false};
}

ghostos_hot_allocation_error ghostos_hot_allocate(ghostos_hot_allocator *a,
    ghostos_hot_object_kind kind, size_t cpu, ghostos_hot_allocation *out) {
    ghostos_hot_allocator_view view = bounded_view(a);
    return ghostos_hot_view_allocate(&view, kind, cpu, out);
}

ghostos_hot_reclaim_error ghostos_hot_reclaim(ghostos_hot_allocator *a, ghostos_hot_allocation allocation) {
    return ghostos_hot_reclaim_on(a, allocation.cpu, allocation);
}

ghostos_hot_reclaim_error ghostos_hot_reclaim_on(ghostos_hot_allocator *a,
    size_t cpu, ghostos_hot_allocation allocation) {
    ghostos_hot_allocator_view view = bounded_view(a);
    return ghostos_hot_view_reclaim_on(&view, cpu, allocation);
}

void ghostos_hot_record_remote_memory(ghostos_hot_allocator *a, ghostos_hot_object_kind kind, uint64_t bytes) {
    ghostos_hot_allocator_view view = bounded_view(a);
    ghostos_hot_view_record_remote_memory(&view, kind, bytes);
}

bool ghostos_hot_report(const ghostos_hot_allocator *a, ghostos_hot_object_kind kind, ghostos_hot_allocator_report *out) {
    ghostos_hot_allocator_view view = {a->cpu_count, a->node_capacity,
        GHOSTOS_HOT_ALLOCATOR_MAX_CPUS, GHOSTOS_HOT_ALLOCATOR_MAX_NODES,
        a->node_count, a->max_cross_node_fallbacks,
        a->cpu_to_node, (ghostos_hot_pool *)&a->pools[0][0][0],
        (ghostos_hot_allocator_stats *)a->stats, false};
    return ghostos_hot_view_report(&view, kind, out);
}
