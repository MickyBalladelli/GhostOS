#include "ghostos/ras.h"
_Static_assert(sizeof(ghostos_ras_event) == 56, "RAS event ABI");
_Static_assert(sizeof(ghostos_ras_event_slot) == 64, "RAS slot ABI");
_Static_assert(sizeof(ghostos_ras_telemetry) == 80, "RAS state ABI");
_Static_assert(offsetof(ghostos_ras_telemetry, counters) == 32, "RAS counters ABI");
_Static_assert(sizeof(ghostos_ras_poison) == 48, "RAS poison ABI");
_Static_assert(offsetof(ghostos_ras_poison, occupied) == 40, "RAS occupancy ABI");
static void increment(uint64_t *value) { if (*value < UINT64_MAX) ++*value; }
uint64_t ghostos_ras_record(ghostos_ras_telemetry *state,
    ghostos_ras_event_slot *slots, size_t capacity, ghostos_ras_event *event) {
    event->sequence = state->next_sequence;
    if (!++state->next_sequence) state->next_sequence = 1;
    if (state->count == capacity) increment(&state->dropped);
    else ++state->count;
    slots[state->cursor] = (ghostos_ras_event_slot){*event, true};
    state->cursor = (state->cursor + 1) % capacity;
    ghostos_ras_counters *c = &state->counters;
    if (event->source == 0) increment(event->severity == 0 ? &c->corrected_ecc : &c->uncorrected_ecc);
    else if (event->source == 1) increment(&c->cxl_poisoned_flits);
    else if (event->source == 2) increment(event->severity == 0 ? &c->aer_correctable :
        event->severity == 2 ? &c->aer_non_fatal : &c->aer_fatal);
    return event->sequence;
}
bool ghostos_ras_event_get(const ghostos_ras_telemetry *state,
    const ghostos_ras_event_slot *slots, size_t capacity, size_t offset, ghostos_ras_event *event) {
    if (offset >= state->count) return false;
    size_t index = (state->cursor + capacity - 1 - offset) % capacity;
    if (!slots[index].occupied) return false;
    *event = slots[index].event;
    return true;
}
static int overlaps(ghostos_ras_poison left, uint64_t start, uint64_t length, bool checked) {
    if (checked && UINT64_MAX - start < length) return -1;
    if (left.start >= start + length) return 0;
    if (checked && UINT64_MAX - left.start < left.length) return -1;
    return start < left.start + left.length;
}
int ghostos_ras_quarantine(ghostos_ras_poison *slots, size_t capacity,
    ghostos_ras_poison poison, bool checked) {
    if (poison.start % GHOSTOS_RAS_PAGE_SIZE || poison.length % GHOSTOS_RAS_PAGE_SIZE) return 1;
    for (size_t i = 0; i < capacity; ++i) {
        if (!slots[i].occupied || slots[i].node != poison.node) continue;
        int overlap = overlaps(slots[i], poison.start, poison.length, checked);
        if (overlap < 0) return -1;
        if (overlap) return 2;
    }
    for (size_t i = 0; i < capacity; ++i) {
        if (slots[i].occupied) continue;
        poison.occupied = true;
        slots[i] = poison;
        return 0;
    }
    return 3;
}
int ghostos_ras_admit(const ghostos_ras_poison *slots, size_t capacity,
    uint32_t node, uint64_t start, uint64_t length, bool checked) {
    for (size_t i = 0; i < capacity; ++i) {
        if (!slots[i].occupied || slots[i].node != node) continue;
        int overlap = overlaps(slots[i], start, length, checked);
        if (overlap) return overlap;
    }
    return 0;
}

_Static_assert(sizeof(ghostos_ras_budget_policy) == 24, "RAS budget policy ABI");
_Static_assert(sizeof(ghostos_ras_budget_reading) == 24, "RAS budget reading ABI");
_Static_assert(sizeof(ghostos_ras_budget_plan) == 8, "RAS budget plan ABI");
_Static_assert(sizeof(ghostos_ras_workload) == 16, "RAS workload ABI");

int ghostos_ras_workload_register(ghostos_ras_workload *slots, size_t capacity,
    uint64_t id, uint8_t priority) {
    for (size_t i = 0; i < capacity; ++i)
        if (slots[i].occupied && slots[i].id == id) return 1;
    for (size_t i = 0; i < capacity; ++i) {
        if (slots[i].occupied) continue;
        slots[i] = (ghostos_ras_workload){id, priority, true, true};
        return 0;
    }
    return 2;
}
bool ghostos_ras_workload_unregister(ghostos_ras_workload *slots, size_t capacity, uint64_t id) {
    for (size_t i = 0; i < capacity; ++i) {
        if (!slots[i].occupied || slots[i].id != id) continue;
        slots[i] = (ghostos_ras_workload){0};
        return true;
    }
    return false;
}
static uint32_t predict(uint32_t value, int32_t rate, uint64_t horizon) {
    if (horizon > UINT32_MAX) horizon = UINT32_MAX;
    /* i32 * u32 always fits i64, so Rust's saturating multiply cannot clamp. */
    int64_t delta = ((int64_t)rate * (int64_t)horizon) / INT64_C(1000000);
    uint64_t magnitude = (uint64_t)(delta < 0 ? -delta : delta);
    if (magnitude > UINT32_MAX) magnitude = UINT32_MAX;
    if (delta < 0) return magnitude > value ? 0 : value - (uint32_t)magnitude;
    uint64_t sum = (uint64_t)value + magnitude;
    return sum > UINT32_MAX ? UINT32_MAX : (uint32_t)sum;
}
ghostos_ras_budget_plan ghostos_ras_budget_decide(ghostos_ras_budget_policy p,
    ghostos_ras_budget_reading r) {
    uint32_t thermal = predict(r.thermal, r.thermal_rate, p.horizon_us);
    uint32_t power = predict(r.power, r.power_rate, p.horizon_us);
    bool critical = r.thermal >= p.thermal_critical || r.power >= p.power_critical;
    bool predicted = thermal >= p.thermal_critical || power >= p.power_critical;
    bool soft = r.thermal >= p.thermal_soft || r.power >= p.power_soft ||
        thermal >= p.thermal_soft || power >= p.power_soft;
    if (!soft) return (ghostos_ras_budget_plan){0, 0};
    if (critical) return (ghostos_ras_budget_plan){3, 90};
    if (predicted) return (ghostos_ras_budget_plan){2, 75};
    return (ghostos_ras_budget_plan){1, 40};
}
size_t ghostos_ras_workload_next(const ghostos_ras_workload *slots, size_t capacity, size_t start) {
    for (size_t i = start; i < capacity; ++i)
        if (slots[i].occupied && slots[i].active && slots[i].priority < 3) return i;
    return capacity;
}
void ghostos_ras_workload_evicted(ghostos_ras_workload *slots, size_t index) {
    slots[index].active = false;
}
