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
