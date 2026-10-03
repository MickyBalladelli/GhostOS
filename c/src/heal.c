#include "ghostos/heal.h"
_Static_assert(sizeof(ghostos_heal_slot) == 24, "heal slot ABI");
_Static_assert(offsetof(ghostos_heal_slot, occupied) == 16, "heal occupancy ABI");
int ghostos_health_service(uint64_t service) {
    return !service || service == UINT64_MAX ? 2 : 0;
}
int ghostos_health_duplicate(const uint64_t *services, size_t count, uint64_t service) {
    size_t i;
    for (i = 0; i < count; ++i) if (services[i] == service) return 1;
    return 0;
}
uint64_t ghostos_health_registration(uint64_t previous) {
    uint64_t next = previous + 1;
    return next < 1 ? 1 : next;
}
bool ghostos_health_mark_time(uint64_t previous, uint64_t progress, bool was_seen) {
    return previous != progress || !was_seen;
}
static uint64_t sat_sub(uint64_t now_us, uint64_t then_us) {
    return now_us < then_us ? 0 : now_us - then_us;
}
int ghostos_health_fault(bool memory_corrupt, bool memory_checked, uint64_t expected_first,
    uint64_t checksum, uint64_t expected_second, bool compared, bool heartbeat_seen,
    uint64_t now_us, uint64_t heartbeat_at_us, uint64_t heartbeat_timeout_us,
    bool have_heartbeat_timeout, bool driver_seen, uint64_t driver_at_us,
    uint64_t driver_timeout_us, bool have_driver, bool progress_seen, uint64_t progress_at_us,
    uint64_t progress_timeout_us, bool have_progress) {
    if (memory_corrupt || (memory_checked && expected_first && compared && checksum != expected_second))
        return 3;
    if (!heartbeat_seen || !have_heartbeat_timeout) return 0;
    if (sat_sub(now_us, heartbeat_at_us) > heartbeat_timeout_us) return 1;
    if (have_driver && driver_seen && sat_sub(now_us, driver_at_us) > driver_timeout_us) return 2;
    if (have_progress && progress_seen && sat_sub(now_us, progress_at_us) > progress_timeout_us) return 1;
    return 0;
}
static int find_slot(const ghostos_heal_slot *slots, size_t count, uint64_t service, size_t *index) {
    size_t i;
    for (i = 0; i < count; ++i) {
        if (slots[i].occupied && slots[i].service == service) {
            *index = i;
            return 0;
        }
    }
    return 4;
}
int ghostos_heal_register(const ghostos_heal_slot *slots, size_t count, uint64_t service,
    uint64_t image_high, uint64_t image_low, uint64_t process, size_t *index) {
    size_t free_slot = count;
    size_t i;
    if (!service) return 2;
    if ((!image_high && !image_low) || !process) return 7;
    for (i = 0; i < count; ++i) {
        if (!slots[i].occupied) {
            if (free_slot == count) free_slot = i;
            continue;
        }
        if (slots[i].service == service) return 3;
    }
    if (free_slot == count) return 1;
    *index = free_slot;
    return 0;
}
int ghostos_heal_find(const ghostos_heal_slot *slots, size_t count, uint64_t service, size_t *index) {
    return find_slot(slots, count, service, index);
}
int ghostos_heal_set_process(const ghostos_heal_slot *slots, size_t count, uint64_t service,
    uint64_t process, size_t *index) {
    if (!process) return 7;
    return find_slot(slots, count, service, index);
}
int ghostos_heal_prepare(const ghostos_heal_slot *slots, size_t count, uint64_t service,
    uint64_t crashed, size_t *index) {
    int status = find_slot(slots, count, service, index);
    if (status) return status;
    if (slots[*index].process != crashed) return 5;
    if (!slots[*index].has_snapshot) return 6;
    return 0;
}
uint32_t ghostos_heal_generation(uint32_t generation) {
    uint32_t next = generation + 1;
    return next < 1 ? 1 : next;
}
int ghostos_heal_accept_process(uint64_t process, uint64_t crashed) {
    return !process || process == crashed ? 7 : 0;
}
