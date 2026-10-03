#include "ghostos/logd.h"
_Static_assert(sizeof(ghostos_log_subscription) == 8, "log subscription ABI");
_Static_assert(offsetof(ghostos_log_subscription, occupied) == 5, "log occupancy ABI");
int ghostos_log_subscribe(const ghostos_log_subscription *slots, size_t count,
    uint32_t terminal, uint8_t minimum_level, size_t *index) {
    size_t free_slot = count;
    size_t i;
    (void)minimum_level;
    for (i = 0; i < count; ++i) {
        if (!slots[i].occupied) {
            if (free_slot == count) free_slot = i;
            continue;
        }
        if (slots[i].terminal == terminal) {
            *index = i;
            return 0;
        }
    }
    if (free_slot == count) return 2;
    *index = free_slot;
    return 1;
}
int ghostos_log_unsubscribe(const ghostos_log_subscription *slots, size_t count,
    uint32_t terminal, size_t *index) {
    size_t i;
    for (i = 0; i < count; ++i) {
        if (slots[i].occupied && slots[i].terminal == terminal) {
            *index = i;
            return 0;
        }
    }
    return 1;
}
bool ghostos_log_deliver(uint8_t level, uint8_t minimum_level) {
    return level >= minimum_level;
}
bool ghostos_log_operator(uint8_t level, uint8_t kind) {
    return level >= 3 || kind == 8;
}
uint64_t ghostos_log_dropped_delta(uint64_t current, uint64_t previous) {
    return current < previous ? 0 : current - previous;
}
size_t ghostos_log_remaining(size_t budget, size_t used) {
    return used > budget ? 0 : budget - used;
}
