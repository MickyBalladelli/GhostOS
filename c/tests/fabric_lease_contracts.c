#include "ghostos/fabric_lease.h"
#include <assert.h>
#include <string.h>
static void lease_places_the_next_page_range_and_expires(void) {
    ghostos_fabric_lease leases[2];
    uint64_t first = 0, second = 0, start = 0, free_start = 0, free_length = 0;
    memset(leases, 0, sizeof leases);
    assert(!ghostos_fabric_lease_allocate(leases, 2, true, false, 1, 0x10000000, 0x20000, 7, 8192, 4096, 3, 10, 100, &first, &start));
    assert(start == 0x10000000 && leases[0].expires_at_us == 110);
    assert(!ghostos_fabric_lease_largest_free(leases, 2, true, 1, 0x10000000, 0x20000, 4096, 10, &free_start, &free_length));
    assert(free_start == 0x10002000 && free_length == 0x1e000);
    assert(!ghostos_fabric_lease_allocate(leases, 2, true, false, 1, 0x10000000, 0x20000, 7, 8192, 4096, 3, 10, 100, &second, &start));
    assert(start == 0x10002000);
    assert(!ghostos_fabric_lease_authorize(leases, 2, first, 7, 0x10000000, 0, 10));
    assert(ghostos_fabric_lease_authorize(leases, 2, first, 7, 0x10002000, 0, 10) == 6);
    assert(ghostos_fabric_lease_authorize(leases, 2, first, 7, 0x10000000, 2, 10) == 6);
    assert(ghostos_fabric_lease_authorize(leases, 2, first, 7, 0x10000000, 0, 110) == 7);
    assert(ghostos_fabric_lease_allocate(leases, 2, true, false, 1, 0x10000000, 0x20000, 7, 0, 4096, 3, 10, 100, &first, &start) == 3);
}
int main(void) {
    lease_places_the_next_page_range_and_expires();
    return 0;
}
