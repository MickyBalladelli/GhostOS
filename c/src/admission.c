#include "ghostos/admission.h"
#include "ghostos/memory.h"

static uint16_t increment16(uint16_t value) { return value == UINT16_MAX ? value : (uint16_t)(value + 1); }
static uint16_t decrement16(uint16_t value) { return value ? (uint16_t)(value - 1) : 0; }
static uint16_t add16(uint16_t left, uint16_t right) {
    uint32_t sum = (uint32_t)left + right;
    return sum > UINT16_MAX ? UINT16_MAX : (uint16_t)sum;
}
static void increment32(uint32_t *value) { if (*value != UINT32_MAX) ++*value; }
static uint64_t sequence(ghostos_admission_controller *state) {
    uint64_t result = state->next_sequence;
    if (state->next_sequence != UINT64_MAX) ++state->next_sequence;
    if (!state->next_sequence) state->next_sequence = 1;
    return result;
}
static ghostos_admission_tenant *tenant_state(ghostos_admission_controller *state, uint64_t id) {
    for (size_t i = 0; i < GHOSTOS_ADMISSION_TENANTS; ++i) {
        if (state->tenants[i].occupied && state->tenants[i].policy.tenant == id) return &state->tenants[i];
    }
    return NULL;
}
static const ghostos_admission_tenant *tenant_view(const ghostos_admission_controller *state, uint64_t id) {
    for (size_t i = 0; i < GHOSTOS_ADMISSION_TENANTS; ++i) {
        if (state->tenants[i].occupied && state->tenants[i].policy.tenant == id) return &state->tenants[i];
    }
    return NULL;
}

ghostos_admission_policy ghostos_admission_default_policy(void) {
    ghostos_admission_policy policy = {16, 32, 4, {16, 16, 16, 16, 16, 16}};
    return policy;
}

uint32_t ghostos_admission_init(ghostos_admission_controller *state,
    ghostos_admission_slot *slots, size_t capacity, const ghostos_admission_policy *policy) {
    if (!capacity || !policy->active_capacity || policy->active_capacity > capacity ||
        policy->recovery_reserve > policy->active_capacity || capacity > SIZE_MAX / sizeof(*slots)) return 1;
    for (size_t i = 0; i < GHOSTOS_ADMISSION_CLASSES; ++i) if (!policy->class_limits[i]) return 1;
    ghostos_admission_policy configured = *policy;
    ghostos_memory_zero(state, sizeof(*state));
    ghostos_memory_zero(slots, capacity * sizeof(*slots));
    state->policy = configured;
    state->next_sequence = 1;
    return 0;
}

void ghostos_admission_get_report(const ghostos_admission_controller *state,
    ghostos_admission_report *report) {
    *report = (ghostos_admission_report){.version = GHOSTOS_ADMISSION_VERSION,
        .policy = state->policy, .active = state->active,
        .recovery_active = state->recovery_active, .queued = state->queued};
    ghostos_memory_copy(report->classes, state->stats, sizeof(report->classes));
}

uint32_t ghostos_admission_configure_tenant(ghostos_admission_controller *state,
    size_t capacity, const ghostos_admission_tenant_policy *policy) {
    if (!policy->tenant || !policy->active_limit || policy->recovery_reserve > policy->active_limit ||
        policy->active_limit > capacity || (policy->has_parent && policy->parent == policy->tenant)) return 1;
    if (policy->has_parent) {
        if (!tenant_state(state, policy->parent)) return 1;
        uint64_t current = policy->parent;
        bool has_current = true;
        for (size_t i = 0; i < GHOSTOS_ADMISSION_TENANTS && has_current; ++i) {
            if (current == policy->tenant) return 1;
            ghostos_admission_tenant *parent = tenant_state(state, current);
            has_current = parent && parent->policy.has_parent;
            if (has_current) current = parent->policy.parent;
        }
    }
    ghostos_admission_tenant *existing = tenant_state(state, policy->tenant);
    if (existing) {
        if (existing->active > policy->active_limit || existing->recovery_active > policy->active_limit) return 1;
        existing->policy = *policy;
        return 0;
    }
    for (size_t i = 0; i < GHOSTOS_ADMISSION_TENANTS; ++i) {
        if (!state->tenants[i].occupied) {
            state->tenants[i] = (ghostos_admission_tenant){.policy = *policy, .occupied = true};
            return 0;
        }
    }
    return 1;
}

static uint8_t block_reason(const ghostos_admission_controller *state, uint64_t tenant,
    uint8_t class_id, uint8_t priority) {
    if (tenant) {
        uint64_t current = tenant;
        bool has_current = true, found = false;
        for (size_t i = 0; i < GHOSTOS_ADMISSION_TENANTS && has_current; ++i) {
            const ghostos_admission_tenant *entry = tenant_view(state, current);
            if (!entry) return 5;
            found = true;
            if (entry->active >= entry->policy.active_limit || (priority != 3 &&
                add16(entry->active, entry->policy.recovery_reserve) >= entry->policy.active_limit)) return 5;
            has_current = entry->policy.has_parent;
            current = entry->policy.parent;
        }
        if (!found) return 5;
    }
    if (priority != 3 && state->active_by_class[class_id] >= state->policy.class_limits[class_id]) return 3;
    if (state->active >= state->policy.active_capacity) return 1;
    if (priority != 3 && add16(state->active, state->policy.recovery_reserve) >= state->policy.active_capacity) return 2;
    return 0;
}

static void adjust_usage(ghostos_admission_controller *state, uint64_t tenant, uint8_t priority, bool add) {
    if (!tenant) return;
    uint64_t current = tenant;
    for (size_t i = 0; i < GHOSTOS_ADMISSION_TENANTS; ++i) {
        ghostos_admission_tenant *entry = tenant_state(state, current);
        if (!entry) break;
        entry->active = add ? increment16(entry->active) : decrement16(entry->active);
        if (priority == 3) entry->recovery_active = add ? increment16(entry->recovery_active) : decrement16(entry->recovery_active);
        if (!entry->policy.has_parent) break;
        current = entry->policy.parent;
    }
}

static void outcome(const ghostos_admission_controller *state, uint64_t seq,
    uint8_t class_id, uint8_t priority, uint8_t action, uint8_t reason,
    const ghostos_admission_lease *lease, ghostos_admission_outcome *result) {
    *result = (ghostos_admission_outcome){.version = GHOSTOS_ADMISSION_VERSION,
        .sequence = seq, .class_id = class_id, .priority = priority,
        .action = action, .reason = reason, .active = state->active,
        .queued = state->queued, .has_lease = lease != NULL};
    if (lease) result->lease = *lease;
}

uint32_t ghostos_admission_admit(ghostos_admission_controller *state,
    ghostos_admission_slot *slots, size_t capacity, uint64_t tenant,
    uint8_t class_id, uint8_t priority, ghostos_admission_outcome *result) {
    if (class_id >= GHOSTOS_ADMISSION_CLASSES || priority > 3) return 1;
    uint8_t reason = block_reason(state, tenant, class_id, priority);
    if (!reason) {
        size_t slot = 0;
        while (slot < capacity && slots[slot].occupied) ++slot;
        /* A valid policy always leaves a slot when active capacity is available. */
        if (slot == capacity) return 1;
        if (state->queued) --state->queued;
        uint64_t seq = sequence(state);
        ghostos_admission_lease lease = {.slot = (uint16_t)slot, .sequence = seq,
            .class_id = class_id, .priority = priority, .tenant = tenant};
        slots[slot] = (ghostos_admission_slot){.lease = lease, .occupied = true};
        state->active = increment16(state->active);
        state->active_by_class[class_id] = increment16(state->active_by_class[class_id]);
        if (priority == 3) state->recovery_active = increment16(state->recovery_active);
        adjust_usage(state, tenant, priority, true);
        increment32(&state->stats[class_id].admitted);
        outcome(state, seq, class_id, priority, 1, reason, &lease, result);
        return 0;
    }
    uint64_t seq = sequence(state);
    uint8_t action;
    if (state->queued < state->policy.queue_capacity) {
        state->queued = increment16(state->queued);
        increment32(&state->stats[class_id].delayed);
        action = 2;
    } else if (priority == 0) {
        increment32(&state->stats[class_id].dropped);
        action = 3;
    } else {
        increment32(&state->stats[class_id].retried);
        action = 4;
    }
    outcome(state, seq, class_id, priority, action, reason, NULL, result);
    return 0;
}

uint32_t ghostos_admission_record_retry(ghostos_admission_controller *state,
    uint8_t class_id, uint8_t priority, ghostos_admission_outcome *result) {
    if (class_id >= GHOSTOS_ADMISSION_CLASSES || priority > 3) return 1;
    uint64_t seq = sequence(state);
    increment32(&state->stats[class_id].retried);
    outcome(state, seq, class_id, priority, 4, 4, NULL, result);
    return 0;
}

uint32_t ghostos_admission_finish(ghostos_admission_controller *state,
    ghostos_admission_slot *slots, size_t capacity, const ghostos_admission_lease *lease) {
    if (lease->slot >= capacity || lease->class_id >= GHOSTOS_ADMISSION_CLASSES || lease->priority > 3) return 2;
    ghostos_admission_slot *slot = &slots[lease->slot];
    const ghostos_admission_lease *current = &slot->lease;
    if (!slot->occupied || current->slot != lease->slot || current->sequence != lease->sequence ||
        current->class_id != lease->class_id || current->priority != lease->priority || current->tenant != lease->tenant) return 2;
    slot->occupied = false;
    state->active = decrement16(state->active);
    state->active_by_class[lease->class_id] = decrement16(state->active_by_class[lease->class_id]);
    if (lease->priority == 3) state->recovery_active = decrement16(state->recovery_active);
    adjust_usage(state, lease->tenant, lease->priority, false);
    increment32(&state->stats[lease->class_id].completed);
    return 0;
}

const char *ghostos_admission_class_name(uint8_t class_id) {
    static const char *const names[] = {"control-plane-fanout", "membership-change",
        "snapshot", "backup", "package-distribution", "remote-diagnostics"};
    return class_id < GHOSTOS_ADMISSION_CLASSES ? names[class_id] : NULL;
}

#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(ghostos_admission_policy) == 18, "admission policy ABI");
_Static_assert(sizeof(ghostos_admission_lease) == 32, "admission lease ABI");
_Static_assert(sizeof(ghostos_admission_slot) == 40, "admission slot ABI");
_Static_assert(sizeof(ghostos_admission_tenant_policy) == 24, "admission tenant ABI");
_Static_assert(sizeof(ghostos_admission_controller) == 1192, "admission controller ABI");
_Static_assert(offsetof(ghostos_admission_controller, tenants) == 168, "admission tenants ABI");
_Static_assert(sizeof(ghostos_admission_outcome) == 64, "admission outcome ABI");
_Static_assert(sizeof(ghostos_admission_report) == 148, "admission report ABI");
#endif
