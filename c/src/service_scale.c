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
