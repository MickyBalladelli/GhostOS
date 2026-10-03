#include "ghostos/heal.h"
#include <assert.h>
static void health_orders_memory_before_driver_stall(void) {
    assert(ghostos_health_fault(false, true, 55, 55, 55, true, true, 12, 10, 10, true,
        true, 10, 5, true, true, 10, 20, true) == 0);
    assert(ghostos_health_fault(false, true, 55, 55, 55, true, true, 20, 10, 10, true,
        true, 10, 5, true, true, 10, 20, true) == 2);
    assert(ghostos_health_fault(false, true, 55, 1, 55, true, true, 20, 10, 10, true,
        true, 10, 5, true, true, 10, 20, true) == 3);
    assert(ghostos_health_fault(false, false, 0, 0, 0, false, false, 0, 0, 0, false,
        false, 0, 0, false, false, 0, 0, false) == 0);
    assert(ghostos_health_registration(UINT64_MAX) == 1);
    assert(ghostos_health_mark_time(1, 1, true) == false);
    assert(ghostos_health_mark_time(1, 2, true) == true);
}
static void recovery_rejects_stale_and_wraps_generation(void) {
    ghostos_heal_slot slots[2] = {0};
    size_t index = 9;
    assert(ghostos_heal_register(slots, 2, 7, 0, 0x55, 10, &index) == 0);
    assert(index == 0);
    slots[0].occupied = true;
    slots[0].service = 7;
    slots[0].process = 10;
    assert(ghostos_heal_register(slots, 2, 7, 0, 0x55, 11, &index) == 3);
    assert(ghostos_heal_prepare(slots, 2, 7, 10, &index) == 6);
    slots[0].has_snapshot = true;
    assert(ghostos_heal_prepare(slots, 2, 7, 11, &index) == 5);
    assert(ghostos_heal_prepare(slots, 2, 7, 10, &index) == 0);
    assert(ghostos_heal_generation(1) == 2);
    assert(ghostos_heal_generation(UINT32_MAX) == 1);
    assert(ghostos_heal_accept_process(0, 10) == 7);
    assert(ghostos_heal_accept_process(10, 10) == 7);
    assert(ghostos_heal_accept_process(11, 10) == 0);
}
int main(void) {
    health_orders_memory_before_driver_stall();
    recovery_rejects_stale_and_wraps_generation();
    return 0;
}
