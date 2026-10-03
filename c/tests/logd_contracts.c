#include "ghostos/logd.h"
#include <assert.h>
static void subscribers_update_before_capacity_and_filter_delivery(void) {
    ghostos_log_subscription slots[2] = {0};
    size_t index = 9;
    assert(ghostos_log_subscribe(slots, 2, 7, 3, &index) == 1 && index == 0);
    slots[0].occupied = true;
    slots[0].terminal = 7;
    slots[0].minimum_level = 3;
    assert(ghostos_log_subscribe(slots, 2, 7, 1, &index) == 0 && index == 0);
    assert(ghostos_log_subscribe(slots, 1, 8, 1, &index) == 2);
    assert(ghostos_log_unsubscribe(slots, 2, 8, &index) == 1);
    assert(ghostos_log_unsubscribe(slots, 2, 7, &index) == 0 && index == 0);
    assert(ghostos_log_deliver(3, 3));
    assert(!ghostos_log_deliver(1, 3));
    assert(ghostos_log_operator(3, 2));
    assert(ghostos_log_operator(1, 8));
    assert(!ghostos_log_operator(1, 2));
    assert(ghostos_log_dropped_delta(4, 9) == 0);
    assert(ghostos_log_dropped_delta(9, 4) == 5);
    assert(ghostos_log_remaining(3, 8) == 0);
    assert(ghostos_log_remaining(8, 3) == 5);
}
int main(void) {
    subscribers_update_before_capacity_and_filter_delivery();
    return 0;
}
