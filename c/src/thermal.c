#include "ghostos/thermal.h"
_Static_assert(sizeof(ghostos_thermal_trips) == 16, "thermal trip ABI");
_Static_assert(offsetof(ghostos_thermal_trips, present) == 12, "thermal mask ABI");
_Static_assert(offsetof(ghostos_thermal_log, dropped) == 2 * sizeof(size_t), "thermal log ABI");
uint8_t ghostos_thermal_decide(const ghostos_thermal_trips *trips,
    uint32_t hysteresis, uint32_t temperature, uint8_t previous,
    uint8_t previous_percent, uint8_t *action, uint8_t *percent) {
    *action = previous;
    *percent = previous_percent;
    if ((trips->present & 4) && temperature >= trips->critical) {
        *action = 2; *percent = 100;
    } else if ((trips->present & 2) && temperature >= trips->hot) {
        *action = 1; *percent = 75;
    } else if (trips->present & 1) {
        if (temperature >= trips->passive) {
            uint32_t above = (temperature - trips->passive) / 2;
            *action = 1; *percent = (uint8_t)(25 + (above < 50 ? above : 50));
        } else {
            uint32_t cooled = UINT32_MAX - temperature < hysteresis ?
                UINT32_MAX : temperature + hysteresis;
            if (cooled < trips->passive) { *action = 0; *percent = 0; }
        }
    } else { *action = 0; *percent = 0; }
    if (*action == previous && *percent == previous_percent) return 0;
    if (*action == 2) return 4;
    if (*action == 0) return 3;
    return previous == 0 ? 1 : 2;
}
bool ghostos_thermal_push(ghostos_thermal_log *log, size_t capacity, size_t *slot) {
    if (!capacity || log->len == capacity) {
        if (log->dropped != UINT64_MAX) ++log->dropped;
    } else ++log->len;
    if (!capacity) return false;
    *slot = log->next;
    log->next = log->next + 1 == capacity ? 0 : log->next + 1;
    return true;
}
size_t ghostos_thermal_drain(ghostos_thermal_log *log, size_t capacity,
    size_t destination_length, size_t *first) {
    if (!capacity) return 0;
    size_t count = log->len < destination_length ? log->len : destination_length;
    *first = log->len == capacity ? log->next : log->next >= log->len ?
        log->next - log->len : capacity - (log->len - log->next);
    log->len -= count;
    return count;
}
