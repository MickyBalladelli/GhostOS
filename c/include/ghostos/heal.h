#ifndef GHOSTOS_HEAL_H
#define GHOSTOS_HEAL_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Health service: 0 valid, 2 invalid. Duplicate: 0 absent, 1 present.
 * Fault: 0 none, 1 deadlock, 2 driver stall, 3 memory corruption.
 * Recovery: 0 success, 1 capacity, 2 invalid service, 3 already registered,
 * 4 not found, 5 stale process, 6 no snapshot, 7 invalid process. */
int ghostos_health_service(uint64_t service);
int ghostos_health_duplicate(const uint64_t *services, size_t count, uint64_t service);
uint64_t ghostos_health_registration(uint64_t previous);
bool ghostos_health_mark_time(uint64_t previous, uint64_t progress, bool was_seen);
int ghostos_health_fault(bool memory_corrupt, bool memory_checked, uint64_t expected_first,
    uint64_t checksum, uint64_t expected_second, bool compared, bool heartbeat_seen,
    uint64_t now_us, uint64_t heartbeat_at_us, uint64_t heartbeat_timeout_us,
    bool have_heartbeat_timeout, bool driver_seen, uint64_t driver_at_us,
    uint64_t driver_timeout_us, bool have_driver, bool progress_seen, uint64_t progress_at_us,
    uint64_t progress_timeout_us, bool have_progress);
typedef struct {
    uint64_t service;
    uint64_t process;
    bool occupied;
    bool has_snapshot;
} ghostos_heal_slot;
int ghostos_heal_register(const ghostos_heal_slot *slots, size_t count, uint64_t service,
    uint64_t image_high, uint64_t image_low, uint64_t process, size_t *index);
int ghostos_heal_find(const ghostos_heal_slot *slots, size_t count, uint64_t service, size_t *index);
int ghostos_heal_set_process(const ghostos_heal_slot *slots, size_t count, uint64_t service,
    uint64_t process, size_t *index);
int ghostos_heal_prepare(const ghostos_heal_slot *slots, size_t count, uint64_t service,
    uint64_t crashed, size_t *index);
uint32_t ghostos_heal_generation(uint32_t generation);
int ghostos_heal_accept_process(uint64_t process, uint64_t crashed);
#endif
