#ifndef GHOSTOS_NUMA_H
#define GHOSTOS_NUMA_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_MAX_NUMA_NODES 64
#define GHOSTOS_MAX_NUMA_CPUS 128
#define GHOSTOS_NUMA_KIND_COUNT 5

enum ghostos_numa_error {
    GHOSTOS_NUMA_OK, GHOSTOS_NUMA_NO_CPUS, GHOSTOS_NUMA_TOO_MANY_CPUS,
    GHOSTOS_NUMA_INVALID_NODE, GHOSTOS_NUMA_TOO_MANY_NODES
};
enum ghostos_numa_kind {
    GHOSTOS_NUMA_PROCESS = 1, GHOSTOS_NUMA_MEMORY, GHOSTOS_NUMA_QUEUE,
    GHOSTOS_NUMA_STORAGE_WORKER, GHOSTOS_NUMA_NETWORK_INTERRUPT
};
enum ghostos_numa_locality {
    GHOSTOS_NUMA_LOCAL_CPU = 1, GHOSTOS_NUMA_LOCAL_NODE,
    GHOSTOS_NUMA_REMOTE_NODE, GHOSTOS_NUMA_UMA_FALLBACK
};
typedef struct {
    uint8_t cpu_to_node[GHOSTOS_MAX_NUMA_CPUS];
    uint16_t cpu_count;
    uint8_t node_count;
    uint64_t node_mask;
} ghostos_numa_topology;
typedef struct {
    uint8_t kind, requested_node, selected_node;
    uint16_t cpu;
    uint8_t locality;
    uint64_t sequence;
} ghostos_numa_decision;
typedef struct {
    uint64_t decisions, local_cpu, local_node, remote_node, uma_fallback;
    uint64_t remote_memory_accesses, remote_memory_bytes;
} ghostos_numa_counters;
typedef struct {
    ghostos_numa_topology topology;
    ghostos_numa_counters counters;
} ghostos_numa_report;
typedef struct {
    ghostos_numa_topology topology;
    uint16_t cursors[GHOSTOS_NUMA_KIND_COUNT];
    ghostos_numa_counters counters;
} ghostos_numa_placement;

/* Outputs on error are unchanged. Nonempty mappings accept sparse node IDs. */
uint32_t ghostos_numa_topology_init(ghostos_numa_topology *topology,
    const uint8_t *mapping, size_t count);
void ghostos_numa_topology_uma(ghostos_numa_topology *topology);
bool ghostos_numa_node_for_cpu(const ghostos_numa_topology *topology,
    size_t cpu, uint8_t *node);
bool ghostos_numa_has_node(const ghostos_numa_topology *topology, uint8_t node);
bool ghostos_numa_first_cpu(const ghostos_numa_topology *topology,
    uint8_t node, uint16_t *cpu);
bool ghostos_numa_node_at(const ghostos_numa_topology *topology,
    size_t index, uint8_t *node);
void ghostos_numa_placement_init(ghostos_numa_placement *placement,
    const ghostos_numa_topology *topology);
/* Changing topology resets cursors and retains counters. */
void ghostos_numa_set_topology(ghostos_numa_placement *placement,
    const ghostos_numa_topology *topology);
/* kind must be 1..5. Presence flags distinguish absent preferences from zero. */
void ghostos_numa_place(ghostos_numa_placement *placement, uint8_t kind,
    bool has_cpu, uint16_t preferred_cpu, bool has_node, uint8_t preferred_node,
    ghostos_numa_decision *decision);
void ghostos_numa_record_memory(ghostos_numa_placement *placement,
    uint8_t local_node, uint8_t allocation_node, uint64_t bytes);
void ghostos_numa_report_read(const ghostos_numa_placement *placement,
    ghostos_numa_report *report);

#endif
