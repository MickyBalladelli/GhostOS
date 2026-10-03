#ifndef GHOSTOS_THERMAL_H
#define GHOSTOS_THERMAL_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
typedef struct { uint32_t passive, hot, critical; uint8_t present; } ghostos_thermal_trips;
typedef struct { size_t next, len; uint64_t dropped; } ghostos_thermal_log;
/* Action: normal=0, throttle=1, shutdown=2. Event: unchanged=0,
 * started=1, changed=2, recovered=3, critical=4. */
uint8_t ghostos_thermal_decide(const ghostos_thermal_trips *trips,
    uint32_t hysteresis, uint32_t temperature, uint8_t previous,
    uint8_t previous_percent, uint8_t *action, uint8_t *percent);
bool ghostos_thermal_push(ghostos_thermal_log *log, size_t capacity, size_t *slot);
size_t ghostos_thermal_drain(ghostos_thermal_log *log, size_t capacity,
    size_t destination_length, size_t *first);
#endif
