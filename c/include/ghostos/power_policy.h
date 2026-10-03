#ifndef GHOSTOS_POWER_POLICY_H
#define GHOSTOS_POWER_POLICY_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
typedef struct {
    uint64_t cpus[2];
    uint32_t idle_power_mw;
    uint8_t id, load, throttle;
    bool valid;
} ghostos_power_candidate;
uint32_t ghostos_power_frequency(uint32_t minimum, uint32_t maximum, uint8_t load, uint8_t throttle);
uint8_t ghostos_power_idle(uint64_t now, uint64_t wake, uint64_t budget);
uint8_t ghostos_power_device(uint64_t now, uint64_t active, uint64_t idle_after, uint64_t suspend_after);
/* 0 selected, 1 no cluster, 2 score overflow under checked arithmetic. */
int ghostos_power_place(const ghostos_power_candidate *clusters, size_t count,
    const uint64_t affinity[2], uint8_t class_id, bool preferred, uint8_t preferred_id,
    bool checked, size_t *selected);
#endif
