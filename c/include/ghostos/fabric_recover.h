#ifndef GHOSTOS_FABRIC_RECOVER_H
#define GHOSTOS_FABRIC_RECOVER_H
#include "ghostos/coherence.h"
#include "ghostos/fabric_lease.h"
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 node not fenced, 2 node failed, 3 capacity, 4 invalid address. */
typedef struct {
    bool occupied, has_mirror;
    uint32_t id, node, mirror;
    uint64_t global_start, global_length, backing_start;
} ghostos_fabric_pool;
int ghostos_fabric_recover(bool isolated, uint32_t failed_node, uint64_t *failed_nodes, ghostos_fabric_lease *leases,
    size_t lease_capacity, ghostos_coherence_page *pages, size_t page_capacity, size_t *leases_released,
    size_t *pages_recovered);
int ghostos_fabric_resolve(const ghostos_fabric_pool *pools, size_t capacity, uint64_t failed_nodes, uint64_t address,
    uint32_t *node, bool *failed_over);
size_t ghostos_fabric_active_leases(const ghostos_fabric_lease *leases, size_t capacity, uint32_t pool);
#endif
