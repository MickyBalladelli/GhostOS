#include "ghostos/fabric_recover.h"
static int node_bit(uint32_t node, uint64_t *bit) {
    if (!node || node > 64) return 3;
    *bit = 1ull << (node - 1);
    return 0;
}
static bool node_failed(uint64_t failed_nodes, uint32_t node) {
    uint64_t bit = 0;
    return !node_bit(node, &bit) && (failed_nodes & bit) != 0;
}
int ghostos_fabric_recover(bool isolated, uint32_t failed_node, uint64_t *failed_nodes, ghostos_fabric_lease *leases,
    size_t lease_capacity, ghostos_coherence_page *pages, size_t page_capacity, size_t *leases_released,
    size_t *pages_recovered) {
    uint64_t bit = 0;
    size_t i, released = 0;
    int status;
    if (!isolated) return 1;
    status = node_bit(failed_node, &bit);
    if (status) return status;
    *failed_nodes |= bit;
    for (i = 0; i < lease_capacity; ++i) {
        if (leases[i].occupied && leases[i].owner == failed_node) {
            leases[i].occupied = false;
            released += 1;
        }
    }
    *leases_released = released;
    return ghostos_coherence_fail_node(pages, page_capacity, failed_node, pages_recovered);
}
int ghostos_fabric_resolve(const ghostos_fabric_pool *pools, size_t capacity, uint64_t failed_nodes, uint64_t address,
    uint32_t *node, bool *failed_over) {
    size_t i;
    for (i = 0; i < capacity; ++i) {
        uint64_t end;
        if (!pools[i].occupied) continue;
        if (pools[i].global_start > UINT64_MAX - pools[i].global_length) return 4;
        end = pools[i].global_start + pools[i].global_length;
        if (address < pools[i].global_start || address >= end) continue;
        if (!node_failed(failed_nodes, pools[i].node)) {
            *node = pools[i].node;
            *failed_over = false;
            return 0;
        }
        if (!pools[i].has_mirror || node_failed(failed_nodes, pools[i].mirror)) return 2;
        *node = pools[i].mirror;
        *failed_over = true;
        return 0;
    }
    return 4;
}
size_t ghostos_fabric_active_leases(const ghostos_fabric_lease *leases, size_t capacity, uint32_t pool) {
    size_t i, active = 0;
    for (i = 0; i < capacity; ++i) if (leases[i].occupied && leases[i].pool == pool) active += 1;
    return active;
}
