#include "ghostos/numa.h"

_Static_assert(sizeof(ghostos_numa_topology) == 144, "topology size");
_Static_assert(offsetof(ghostos_numa_topology, node_mask) == 136, "node mask offset");
_Static_assert(sizeof(ghostos_numa_decision) == 16, "decision size");
_Static_assert(offsetof(ghostos_numa_decision, cpu) == 4, "CPU offset");
_Static_assert(offsetof(ghostos_numa_decision, sequence) == 8, "sequence offset");
_Static_assert(sizeof(ghostos_numa_counters) == 56, "counter size");
_Static_assert(offsetof(ghostos_numa_placement, counters) == 160, "counter offset");

static uint64_t saturating_add(uint64_t value, uint64_t amount)
{
    return amount > UINT64_MAX - value ? UINT64_MAX : value + amount;
}

uint32_t ghostos_numa_topology_init(ghostos_numa_topology *topology,
    const uint8_t *mapping, size_t count)
{
    if (count == 0) return GHOSTOS_NUMA_NO_CPUS;
    if (count > GHOSTOS_MAX_NUMA_CPUS) return GHOSTOS_NUMA_TOO_MANY_CPUS;
    ghostos_numa_topology next = {0};
    for (size_t cpu = 0; cpu < count; ++cpu) {
        uint8_t node = mapping[cpu];
        if (node >= GHOSTOS_MAX_NUMA_NODES) return GHOSTOS_NUMA_INVALID_NODE;
        next.cpu_to_node[cpu] = node;
        next.node_mask |= UINT64_C(1) << node;
    }
    for (uint64_t mask = next.node_mask; mask != 0; mask &= mask - 1)
        ++next.node_count;
    next.cpu_count = (uint16_t)count;
    *topology = next;
    return GHOSTOS_NUMA_OK;
}

void ghostos_numa_topology_uma(ghostos_numa_topology *topology)
{
    *topology = (ghostos_numa_topology){.cpu_count = 1, .node_count = 1, .node_mask = 1};
}

bool ghostos_numa_node_for_cpu(const ghostos_numa_topology *topology,
    size_t cpu, uint8_t *node)
{
    if (cpu >= topology->cpu_count) return false;
    *node = topology->cpu_to_node[cpu];
    return true;
}

bool ghostos_numa_has_node(const ghostos_numa_topology *topology, uint8_t node)
{
    return node < GHOSTOS_MAX_NUMA_NODES &&
        (topology->node_mask & (UINT64_C(1) << node)) != 0;
}

bool ghostos_numa_first_cpu(const ghostos_numa_topology *topology,
    uint8_t node, uint16_t *cpu)
{
    for (uint16_t index = 0; index < topology->cpu_count; ++index) {
        if (topology->cpu_to_node[index] == node) {
            *cpu = index;
            return true;
        }
    }
    return false;
}

bool ghostos_numa_node_at(const ghostos_numa_topology *topology,
    size_t index, uint8_t *node)
{
    if (index >= topology->node_count) return false;
    size_t seen = 0;
    for (uint8_t candidate = 0; candidate < GHOSTOS_MAX_NUMA_NODES; ++candidate) {
        if (ghostos_numa_has_node(topology, candidate)) {
            if (seen == index) {
                *node = candidate;
                return true;
            }
            ++seen;
        }
    }
    return false;
}

void ghostos_numa_placement_init(ghostos_numa_placement *placement,
    const ghostos_numa_topology *topology)
{
    ghostos_numa_topology copy = *topology;
    *placement = (ghostos_numa_placement){.topology = copy};
}

void ghostos_numa_set_topology(ghostos_numa_placement *placement,
    const ghostos_numa_topology *topology)
{
    placement->topology = *topology;
    for (size_t kind = 0; kind < GHOSTOS_NUMA_KIND_COUNT; ++kind)
        placement->cursors[kind] = 0;
}

void ghostos_numa_place(ghostos_numa_placement *placement, uint8_t kind,
    bool has_cpu, uint16_t preferred_cpu, bool has_node, uint8_t preferred_node,
    ghostos_numa_decision *decision)
{
    const ghostos_numa_topology *topology = &placement->topology;
    uint8_t requested = 0;
    if (has_node) requested = preferred_node;
    else if (has_cpu) (void)ghostos_numa_node_for_cpu(topology, preferred_cpu, &requested);
    uint8_t selected = requested;
    if (!ghostos_numa_has_node(topology, requested)) {
        size_t count = topology->node_count == 0 ? 1 : topology->node_count;
        uint16_t *cursor = &placement->cursors[kind - 1];
        selected = 0;
        (void)ghostos_numa_node_at(topology, *cursor % count, &selected);
        *cursor = (uint16_t)(*cursor + 1);
    }
    uint8_t cpu_node = 0;
    uint16_t cpu = 0;
    if (has_cpu && ghostos_numa_node_for_cpu(topology, preferred_cpu, &cpu_node) &&
        cpu_node == selected) cpu = preferred_cpu;
    else (void)ghostos_numa_first_cpu(topology, selected, &cpu);
    uint8_t locality;
    if (topology->node_count <= 1) locality = GHOSTOS_NUMA_UMA_FALLBACK;
    else if (requested != selected) locality = GHOSTOS_NUMA_REMOTE_NODE;
    else if (has_cpu && preferred_cpu == cpu) locality = GHOSTOS_NUMA_LOCAL_CPU;
    else locality = GHOSTOS_NUMA_LOCAL_NODE;
    ghostos_numa_counters *counters = &placement->counters;
    counters->decisions = saturating_add(counters->decisions, 1);
    uint64_t *counter;
    switch (locality) {
        case GHOSTOS_NUMA_LOCAL_CPU: counter = &counters->local_cpu; break;
        case GHOSTOS_NUMA_LOCAL_NODE: counter = &counters->local_node; break;
        case GHOSTOS_NUMA_REMOTE_NODE: counter = &counters->remote_node; break;
        default: counter = &counters->uma_fallback; break;
    }
    *counter = saturating_add(*counter, 1);
    *decision = (ghostos_numa_decision){.kind = kind, .requested_node = requested,
        .selected_node = selected, .cpu = cpu, .locality = locality,
        .sequence = counters->decisions};
}

void ghostos_numa_record_memory(ghostos_numa_placement *placement,
    uint8_t local_node, uint8_t allocation_node, uint64_t bytes)
{
    if (placement->topology.node_count > 1 && local_node != allocation_node) {
        placement->counters.remote_memory_accesses =
            saturating_add(placement->counters.remote_memory_accesses, 1);
        placement->counters.remote_memory_bytes =
            saturating_add(placement->counters.remote_memory_bytes, bytes);
    }
}

void ghostos_numa_report_read(const ghostos_numa_placement *placement,
    ghostos_numa_report *report)
{
    *report = (ghostos_numa_report){placement->topology, placement->counters};
}
