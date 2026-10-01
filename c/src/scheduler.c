#include "ghostos/scheduler.h"

static void effective_key(const ghostos_scheduler_thread_view *thread,
    uint8_t *priority, uint64_t *deadline) {
    uint8_t base_priority = thread->policy == GHOSTOS_SCHEDULER_POLICY_REALTIME ?
        thread->priority : 0;
    uint64_t base_deadline = thread->policy == GHOSTOS_SCHEDULER_POLICY_REALTIME ?
        thread->deadline : UINT64_MAX;
    if (thread->inherited_priority > base_priority) {
        *priority = thread->inherited_priority;
        *deadline = thread->inherited_deadline;
    } else {
        *priority = base_priority;
        *deadline = base_deadline;
    }
}

void ghostos_scheduler_effective_key(const ghostos_scheduler_thread_view *thread,
    uint8_t *priority, uint64_t *deadline) {
    if (!thread || !priority || !deadline) return;
    effective_key(thread, priority, deadline);
}

static bool eligible(const ghostos_scheduler_thread_view *thread, const uint64_t cpus[2]) {
    return thread->state == GHOSTOS_SCHEDULER_THREAD_READY &&
        ((thread->affinity[0] & cpus[0]) || (thread->affinity[1] & cpus[1]));
}

int32_t ghostos_scheduler_pick_realtime(const ghostos_scheduler_thread_view *threads,
    size_t thread_count, const uint64_t cpus[2]) {
    if (!threads || !cpus) return -1;
    int32_t selected = -1;
    uint8_t selected_priority = 0;
    uint64_t selected_deadline = UINT64_MAX;
    uint32_t selected_id = UINT32_MAX;
    for (size_t index = 0; index < thread_count; ++index) {
        const ghostos_scheduler_thread_view *thread = &threads[index];
        if (!eligible(thread, cpus)) continue;
        uint8_t priority;
        uint64_t deadline;
        effective_key(thread, &priority, &deadline);
        if (!priority) continue;
        if (selected < 0 || priority > selected_priority ||
            (priority == selected_priority && (deadline < selected_deadline ||
            (deadline == selected_deadline && thread->id < selected_id)))) {
            selected = (int32_t)index;
            selected_priority = priority;
            selected_deadline = deadline;
            selected_id = thread->id;
        }
    }
    return selected;
}

int32_t ghostos_scheduler_pick_next(const ghostos_scheduler_thread_view *threads,
    size_t thread_count, const uint64_t cpus[2], size_t *cooperative_cursor) {
    if (!threads || !cpus || !cooperative_cursor || !thread_count) return -1;
    int32_t realtime = ghostos_scheduler_pick_realtime(threads, thread_count, cpus);
    if (realtime >= 0) return realtime;
    for (size_t offset = 1; offset <= thread_count; ++offset) {
        size_t slot = (*cooperative_cursor + offset) % thread_count;
        const ghostos_scheduler_thread_view *thread = &threads[slot];
        if (thread->state == GHOSTOS_SCHEDULER_THREAD_READY &&
            thread->policy == GHOSTOS_SCHEDULER_POLICY_COOPERATIVE && eligible(thread, cpus)) {
            *cooperative_cursor = slot;
            return (int32_t)slot;
        }
    }
    return -1;
}

bool ghostos_scheduler_outranks(const ghostos_scheduler_thread_view *candidate,
    const ghostos_scheduler_thread_view *current) {
    if (!candidate || !current) return false;
    uint8_t candidate_priority, current_priority;
    uint64_t candidate_deadline, current_deadline;
    effective_key(candidate, &candidate_priority, &candidate_deadline);
    effective_key(current, &current_priority, &current_deadline);
    return candidate_priority > current_priority ||
        (candidate_priority == current_priority && candidate_deadline < current_deadline);
}

void ghostos_scheduler_recompute_inheritance(ghostos_scheduler_thread_view *threads,
    size_t thread_count, const ghostos_scheduler_wait_edge *edges, size_t edge_count) {
    if (!threads) return;
    for (size_t index = 0; index < thread_count; ++index) {
        threads[index].inherited_priority = 0;
        threads[index].inherited_deadline = UINT64_MAX;
    }
    if (!edges) edge_count = 0;
    for (size_t pass = 0; pass < thread_count; ++pass) {
        bool changed = false;
        for (size_t index = 0; index < edge_count; ++index) {
            const ghostos_scheduler_wait_edge *edge = &edges[index];
            if (edge->owner_slot >= thread_count || edge->waiter_slot >= thread_count) continue;
            uint8_t priority;
            uint64_t deadline;
            effective_key(&threads[edge->waiter_slot], &priority, &deadline);
            ghostos_scheduler_thread_view *owner = &threads[edge->owner_slot];
            if (priority > owner->inherited_priority ||
                (priority == owner->inherited_priority && deadline < owner->inherited_deadline)) {
                owner->inherited_priority = priority;
                owner->inherited_deadline = deadline;
                changed = true;
            }
        }
        if (!changed) break;
    }
}
