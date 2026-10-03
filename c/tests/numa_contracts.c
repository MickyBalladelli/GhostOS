#include "ghostos/numa.h"
#include <assert.h>

/* Ports of the two existing Rust NUMA cases. */
static void invalid_target_is_bounded_and_reported_as_remote(void)
{
    const uint8_t mapping[] = {0, 0, 1, 1};
    ghostos_numa_topology topology;
    assert(ghostos_numa_topology_init(&topology, mapping, 4) == GHOSTOS_NUMA_OK);
    ghostos_numa_placement placement;
    ghostos_numa_placement_init(&placement, &topology);
    ghostos_numa_decision decision;
    ghostos_numa_place(&placement, GHOSTOS_NUMA_PROCESS, false, 0, true, 9, &decision);
    assert(decision.selected_node == 0);
    assert(decision.locality == GHOSTOS_NUMA_REMOTE_NODE);
}

static void uma_hosts_use_safe_fallback_and_never_report_remote_bytes(void)
{
    const uint8_t mapping[] = {1, 1};
    ghostos_numa_topology topology;
    assert(ghostos_numa_topology_init(&topology, mapping, 2) == GHOSTOS_NUMA_OK);
    ghostos_numa_placement placement;
    ghostos_numa_placement_init(&placement, &topology);
    ghostos_numa_decision decision;
    ghostos_numa_place(&placement, GHOSTOS_NUMA_MEMORY, true, 0, true, 8, &decision);
    ghostos_numa_record_memory(&placement, 1, 8, 4096);
    assert(decision.locality == GHOSTOS_NUMA_UMA_FALLBACK);
    assert(placement.counters.remote_memory_bytes == 0);
}

int main(void)
{
    invalid_target_is_bounded_and_reported_as_remote();
    uma_hosts_use_safe_fallback_and_never_report_remote_bytes();
    return 0;
}
