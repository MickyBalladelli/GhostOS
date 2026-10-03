#include "ghostos/cxl.h"
#include <assert.h>
#include <string.h>
static void discovery_drains_before_removal_and_qos_refills(void) {
    ghostos_cxl_device devices[1];
    ghostos_cxl_budget budgets[1];
    size_t index = 9;
    uint8_t state = 0, decision = 9;
    uint64_t serial = 0, retry = 0;
    memset(devices, 0, sizeof devices);
    memset(budgets, 0, sizeof budgets);
    assert(!ghostos_cxl_discover(devices, 1, 2, 7, 3, 1, 0x100, 0, 256u * 1024u * 1024u, 0, 2, &index));
    assert(index == 0);
    assert(ghostos_cxl_discover(devices, 1, 2, 7, 3, 1, 0x100, 0, 256u * 1024u * 1024u, 0, 0, &index) == 1);
    assert(!ghostos_cxl_begin_remove(devices, 1, 7, &state));
    assert(state == 1);
    assert(ghostos_cxl_begin_remove(devices, 1, 7, &state) == 5);
    assert(!ghostos_cxl_cancel_remove(devices, 1, 7));
    assert(!ghostos_cxl_begin_remove(devices, 1, 7, &state));
    assert(!ghostos_cxl_complete_remove(devices, 1, 7, &serial));
    assert(serial == 7 && !devices[0].occupied);
    assert(!ghostos_cxl_configure(budgets, 1, 2, 0, 100, 100));
    assert(!ghostos_cxl_admit(budgets, 1, 2, 0, 0, 100, &decision, &retry));
    assert(decision == 0);
    assert(!ghostos_cxl_admit(budgets, 1, 2, 0, 0, 1, &decision, &retry));
    assert(decision == 1 && retry == 10000);
    assert(!ghostos_cxl_admit(budgets, 1, 2, 0, 1000000, 1, &decision, &retry));
    assert(decision == 0);
}
int main(void) {
    discovery_drains_before_removal_and_qos_refills();
    return 0;
}
