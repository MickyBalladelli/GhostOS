#include "ghostos/coherence.h"
#include "ghostos/fabric_lease.h"
#include "ghostos/fabric_recover.h"
#include <assert.h>
#include <string.h>
static void recovery_waits_for_a_fence_before_releasing_memory(void) {
    ghostos_fabric_pool pools[1];
    ghostos_fabric_lease leases[1];
    ghostos_coherence_page pages[2];
    uint64_t failed_nodes = 0;
    uint32_t node = 0;
    bool failed_over = false, has_owner = false;
    size_t released = 9, recovered = 9;
    memset(pools, 0, sizeof pools);
    memset(leases, 0, sizeof leases);
    pools[0].occupied = true;
    pools[0].has_mirror = true;
    pools[0].id = 1;
    pools[0].node = 2;
    pools[0].mirror = 1;
    pools[0].global_start = 4096;
    pools[0].global_length = 4096;
    pools[0].backing_start = 4096;
    leases[0].occupied = true;
    leases[0].pool = 1;
    leases[0].owner = 2;
    ghostos_coherence_init(pages, 2);
    assert(!ghostos_coherence_grant(pages, 2, 4096, 2, 1, 1, 1000));
    assert(!ghostos_coherence_arrive(pages, 2, 4096, 2, true));
    assert(ghostos_fabric_recover(false, 2, &failed_nodes, leases, 1, pages, 2, &released, &recovered) == 1);
    assert(!failed_nodes && ghostos_fabric_active_leases(leases, 1, 1) == 1 && pages[0].has_owner && pages[0].owner == 2);
    assert(!ghostos_fabric_recover(true, 2, &failed_nodes, leases, 1, pages, 2, &released, &recovered));
    assert(released == 1 && recovered == 1 && failed_nodes != 0);
    assert(!ghostos_fabric_resolve(pools, 1, failed_nodes, 4096, &node, &failed_over));
    assert(node == 1 && failed_over);
    assert(!ghostos_fabric_active_leases(leases, 1, 1));
    assert(!ghostos_coherence_first(pages, 2, &has_owner));
    assert(!pages[0].has_owner && !pages[0].has_lease && !has_owner);
}
int main(void) {
    recovery_waits_for_a_fence_before_releasing_memory();
    return 0;
}
