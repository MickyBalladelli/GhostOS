#include "ghostos/coherence.h"
#include <assert.h>
static void write_fault_invalidates_the_other_sharer(void) {
    ghostos_coherence_page pages[2];
    uint8_t action = 0;
    bool writable = false, has_lease = true;
    uint64_t nodes = 0;
    size_t changed = 0;
    uint64_t page = 2ull * 4096ull;
    ghostos_coherence_init(pages, 2);
    assert(!ghostos_coherence_grant(pages, 2, page, 1, 1, 8, 100));
    assert(!ghostos_coherence_arrive(pages, 2, page, 2, false));
    assert(!ghostos_coherence_begin_fault(pages, 2, 1, page + 17, 1, false, 10, &action, &writable, &nodes));
    assert(action == 3 && nodes != 0);
    assert(!ghostos_coherence_invalidation_complete(pages, 2, page, 1));
    assert(!ghostos_coherence_begin_fault(pages, 2, 1, page + 17, 1, false, 10, &action, &writable, &nodes));
    assert(action == 0 && writable);
    assert(ghostos_coherence_begin_fault(pages, 2, 1, page + 17, 1, true, 10, &action, &writable, &nodes) == 1);
    assert(!ghostos_coherence_fail_node(pages, 2, 1, &changed));
    assert(changed == 1);
    assert(!ghostos_coherence_first(pages, 2, &has_lease));
    assert(!has_lease);
}
int main(void) {
    write_fault_invalidates_the_other_sharer();
    return 0;
}
