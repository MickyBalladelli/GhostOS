#include "ghostos/embedded_script.h"
#include <assert.h>
static void scripts_gate_requests_and_reject_duplicate_names(void) {
    const uint8_t filesystem[] = "filesystem";
    const uint8_t embedded_nul[] = {'f', 0, 's'};
    const uint8_t *names[] = {filesystem, filesystem};
    size_t lengths[] = {10, 10};
    uint64_t mask = 0;
    uint32_t sequence = 0;
    assert(!ghostos_embedded_limits(0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1));
    assert(ghostos_embedded_limits(1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1));
    assert(ghostos_embedded_capability(filesystem, 0, 0) == 1);
    assert(ghostos_embedded_capability(embedded_nul, 3, 1) == 1);
    assert(ghostos_embedded_capability(filesystem, 10, 1) == 0);
    assert(!ghostos_embedded_operation_mask(64, &mask));
    assert(ghostos_embedded_operation_mask(1, &mask) && mask == 2);
    assert(ghostos_embedded_allows(2, 1));
    assert(!ghostos_embedded_allows(2, 0));
    assert(ghostos_embedded_names(names, lengths, 17, 16) == 1);
    assert(ghostos_embedded_names(names, lengths, 2, 16) == 2);
    lengths[1] = 2;
    assert(ghostos_embedded_names(names, lengths, 2, 16) == 0);
    assert(!ghostos_embedded_source(5, 4));
    assert(ghostos_embedded_source(4, 4));
    assert(ghostos_embedded_request(false, 0, true, 1, 4, 0, 1, &sequence) == 1);
    assert(ghostos_embedded_request(true, 0, true, 5, 4, 0, 1, &sequence) == 2);
    assert(ghostos_embedded_request(true, 0, false, 1, 4, 0, 1, &sequence) == 3);
    assert(ghostos_embedded_request(true, 0, true, 1, 4, 1, 1, &sequence) == 4);
    assert(ghostos_embedded_request(true, 0, true, 1, 4, 0, 1, &sequence) == 0 && sequence == 1);
}
int main(void) {
    scripts_gate_requests_and_reject_duplicate_names();
    return 0;
}
