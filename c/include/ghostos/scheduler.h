#ifndef GHOSTOS_SCHEDULER_H
#define GHOSTOS_SCHEDULER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_SCHEDULER_MAX_THREADS 64u
#define GHOSTOS_SCHEDULER_THREAD_READY 1u
#define GHOSTOS_SCHEDULER_POLICY_COOPERATIVE 0u
#define GHOSTOS_SCHEDULER_POLICY_REALTIME 1u

typedef struct {
    uint32_t id;
    uint8_t state;
    uint8_t policy;
    uint8_t priority;
    uint8_t inherited_priority;
    uint64_t deadline;
    uint64_t inherited_deadline;
    uint64_t affinity[2];
} ghostos_scheduler_thread_view;

typedef struct {
    uint32_t owner_slot;
    uint32_t waiter_slot;
} ghostos_scheduler_wait_edge;

_Static_assert(sizeof(ghostos_scheduler_thread_view) == 40, "scheduler thread view layout");
_Static_assert(sizeof(ghostos_scheduler_wait_edge) == 8, "scheduler wait edge layout");

void ghostos_scheduler_effective_key(const ghostos_scheduler_thread_view *thread,
    uint8_t *priority, uint64_t *deadline);
int32_t ghostos_scheduler_pick_realtime(const ghostos_scheduler_thread_view *threads,
    size_t thread_count, const uint64_t cpus[2]);
int32_t ghostos_scheduler_pick_next(const ghostos_scheduler_thread_view *threads,
    size_t thread_count, const uint64_t cpus[2], size_t *cooperative_cursor);
bool ghostos_scheduler_outranks(const ghostos_scheduler_thread_view *candidate,
    const ghostos_scheduler_thread_view *current);
void ghostos_scheduler_recompute_inheritance(ghostos_scheduler_thread_view *threads,
    size_t thread_count, const ghostos_scheduler_wait_edge *edges, size_t edge_count);

#endif
