#include "ghostos/service_scale.h"
_Static_assert(sizeof(ghostos_scale_instance) == 24, "scale instance ABI");
_Static_assert(offsetof(ghostos_scale_instance, occupied) == 17, "scale occupancy ABI");
int ghostos_scale_membership(const ghostos_scale_instance *instances, size_t count,
    uint32_t id, uint64_t generation, uint8_t operation, size_t *index) {
    if (operation == 0 && !generation) return 5;
    size_t free_slot = count;
    for (size_t i = 0; i < count; ++i) {
        const ghostos_scale_instance *record = &instances[i];
        if (!record->occupied) { if (free_slot == count) free_slot = i; continue; }
        if (record->id != id) continue;
        *index = i;
        if (operation == 0) return record->state != 3 || generation <= record->generation ||
            record->sessions || record->in_flight ? 2 : 0;
        if (record->generation != generation) return 5;
        if (operation == 1) return record->state == 0 || record->state == 1 ? 0 : 4;
        if (operation == 2) return record->state == 1 ? 0 : 4;
        if (operation == 3) return record->state != 2 || record->sessions || record->in_flight ? 6 : 0;
        if (operation == 5) return record->state == 1 ? 0 : 4;
        return 0;
    }
    if (operation != 0) return 3;
    if (free_slot == count) return 1;
    *index = free_slot;
    return 0;
}
int ghostos_scale_target(const ghostos_scale_instance *instances, size_t count,
    uint32_t owner, uint32_t *target) {
    uint32_t best = UINT32_MAX;
    bool found = false;
    for (size_t i = 0; i < count; ++i) {
        const ghostos_scale_instance *record = &instances[i];
        if (!record->occupied || record->id == owner || record->state != 1) continue;
        uint32_t load = (uint32_t)record->sessions + record->in_flight;
        if (load < best) { found = true; best = load; *target = record->id; }
    }
    return found ? 0 : 7;
}
uint64_t ghostos_scale_digest(const uint8_t *bytes, size_t length) {
    uint64_t hash = UINT64_C(0xcbf29ce484222325);
    for (size_t i = 0; i < length; ++i) {
        hash ^= bytes[i];
        hash *= UINT64_C(0x100000001b3);
    }
    return hash;
}

int ghostos_scale_session_close(uint8_t state, uint16_t in_flight) {
    return in_flight || state != 0 ? 6 : 0;
}
int ghostos_scale_prepare(uint8_t state, uint16_t in_flight, uint64_t generation,
    uint64_t requested_generation, uint64_t sequence, uint64_t requested_sequence) {
    if (generation != requested_generation || in_flight || state != 0 || requested_sequence <= sequence)
        return in_flight ? 6 : 4;
    return 0;
}
_Static_assert(sizeof(ghostos_scale_handoff) == 40, "scale handoff ABI");
_Static_assert(offsetof(ghostos_scale_handoff, source) == 32, "scale source ABI");
bool ghostos_scale_same_handoff(const ghostos_scale_handoff *stored, const ghostos_scale_handoff *token) {
    return stored->source == token->source && stored->target == token->target &&
        stored->source_generation == token->source_generation &&
        stored->target_generation == token->target_generation &&
        stored->sequence == token->sequence && stored->digest == token->digest;
}

_Static_assert(sizeof(ghostos_scale_request) == 32, "scale request ABI");
_Static_assert(offsetof(ghostos_scale_request, occupied) == 25, "scale request occupancy ABI");
_Static_assert(sizeof(ghostos_scale_effect) == 16, "scale effect ABI");
int ghostos_scale_route(const ghostos_scale_request *requests, size_t request_count,
    const ghostos_scale_effect *effects, size_t effect_count, uint64_t request,
    uint64_t session, uint64_t effect, uint8_t *decision, size_t *index) {
    if (!effect) return 9;
    for (size_t i = 0; i < request_count; ++i) {
        const ghostos_scale_request *record = &requests[i];
        if (!record->occupied || record->id != request) continue;
        if (record->effect != effect || record->session != session) return 8;
        *index = i;
        *decision = record->state == 2 ? 3 : record->state == 0 ? 2 : 1;
        return 0;
    }
    size_t reservations = 0;
    for (size_t i = 0; i < effect_count; ++i) {
        if (!effects[i].occupied) continue;
        if (effects[i].effect == effect) { *index = i; *decision = 4; return 0; }
        ++reservations;
    }
    for (size_t i = 0; i < request_count; ++i) {
        const ghostos_scale_request *record = &requests[i];
        if (!record->occupied) continue;
        if (record->effect == effect) {
            if (record->state != 0) return 8;
            *index = i; *decision = 2; return 0;
        }
        if (record->state != 2) ++reservations;
    }
    if (reservations >= effect_count) return 1;
    *decision = 0;
    return 0;
}

int ghostos_scale_request_finish(uint32_t owner, uint64_t generation, uint8_t state,
    uint32_t requested_owner, uint64_t requested_generation) {
    if (owner != requested_owner || generation != requested_generation) return 5;
    return state == 0 ? 0 : 4;
}
bool ghostos_scale_increment(uint64_t value, uint64_t maximum, uint64_t *next) {
    if (value >= maximum) return false;
    *next = value + 1;
    return true;
}
uint16_t ghostos_scale_decrement(uint16_t value) { return value ? value - 1 : 0; }

_Static_assert(sizeof(ghostos_scale_snapshot) == 7 * sizeof(size_t), "scale snapshot ABI");
static bool snapshot_add(size_t *value, size_t amount, bool checked) {
    if (checked && SIZE_MAX - *value < amount) return false;
    *value += amount;
    return true;
}
bool ghostos_scale_snapshot_read(const ghostos_scale_instance *instances, size_t instance_count,
    const ghostos_scale_effect *effects, size_t effect_count, bool checked, ghostos_scale_snapshot *snapshot) {
    *snapshot = (ghostos_scale_snapshot){0};
    for (size_t i = 0; i < instance_count; ++i) {
        const ghostos_scale_instance *record = &instances[i];
        if (!record->occupied) continue;
        if (!snapshot_add(&snapshot->instances, 1, checked) ||
            !snapshot_add(&snapshot->sessions, record->sessions, checked) ||
            !snapshot_add(&snapshot->in_flight, record->in_flight, checked)) return false;
        if (record->state == 1 && !snapshot_add(&snapshot->ready, 1, checked)) return false;
        if (record->state == 2 && !snapshot_add(&snapshot->draining, 1, checked)) return false;
        if (record->state == 3 && !snapshot_add(&snapshot->restarting, 1, checked)) return false;
    }
    for (size_t i = 0; i < effect_count; ++i)
        if (effects[i].occupied && !snapshot_add(&snapshot->completed_effects, 1, checked)) return false;
    return true;
}

_Static_assert(sizeof(ghostos_scale_slot) == 16, "scale slot ABI");
_Static_assert(offsetof(ghostos_scale_slot, occupied) == 8, "scale slot occupancy ABI");
bool ghostos_scale_find(const ghostos_scale_slot *slots, size_t count,
    uint64_t id, bool free_slot, size_t *index) {
    for (size_t i = 0; i < count; ++i) {
        if (free_slot ? !slots[i].occupied : slots[i].occupied && slots[i].id == id) {
            *index = i;
            return true;
        }
    }
    return false;
}
