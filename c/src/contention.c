#include "ghostos/contention.h"

static uint64_t saturating_add(uint64_t a, uint64_t b)
{
    return UINT64_MAX - a < b ? UINT64_MAX : a + b;
}

static void atomic_max(atomic_uint_fast64_t *value, uint64_t candidate)
{
    uint_fast64_t current = atomic_load_explicit(value, memory_order_relaxed);
    while (current < candidate &&
           !atomic_compare_exchange_weak_explicit(value, &current, candidate,
                                                  memory_order_relaxed,
                                                  memory_order_relaxed)) {}
}

size_t ghostos_lock_duration_bucket(uint64_t duration)
{
    if (duration == 0) return 0;
    if (duration == 1) return 1;
    if (duration <= 3) return 2;
    if (duration <= 7) return 3;
    if (duration <= 15) return 4;
    if (duration <= 31) return 5;
    if (duration <= 63) return 6;
    return 7;
}

void ghostos_ticket_lock_init(ghostos_ticket_lock *lock)
{
    atomic_init(&lock->next_ticket, 0);
    atomic_init(&lock->serving_ticket, 0);
    atomic_init(&lock->owner, 0);
    atomic_init(&lock->acquisitions, 0);
    atomic_init(&lock->contended_acquisitions, 0);
    atomic_init(&lock->spins, 0);
    atomic_init(&lock->releases, 0);
    for (size_t i = 0; i < GHOSTOS_LOCK_DURATION_BUCKETS; ++i)
        atomic_init(&lock->duration_histogram[i], 0);
    atomic_init(&lock->max_duration, 0);
}

ghostos_ticket_lock_guard ghostos_ticket_lock_acquire(ghostos_ticket_lock *lock,
                                                      uint64_t now)
{
    uint64_t ticket = atomic_fetch_add_explicit(&lock->next_ticket, 1,
                                                memory_order_relaxed);
    uint64_t spins = 0;
    while (atomic_load_explicit(&lock->serving_ticket, memory_order_acquire) != ticket) {
        spins = saturating_add(spins, 1);
        atomic_signal_fence(memory_order_seq_cst);
    }
    atomic_fetch_add_explicit(&lock->acquisitions, 1, memory_order_relaxed);
    if (spins != 0) {
        atomic_fetch_add_explicit(&lock->contended_acquisitions, 1,
                                  memory_order_relaxed);
        atomic_fetch_add_explicit(&lock->spins, spins, memory_order_relaxed);
    }
    atomic_store_explicit(&lock->owner, ticket + 1, memory_order_release);
    return (ghostos_ticket_lock_guard){ lock, ticket, now, spins, false };
}

void ghostos_ticket_lock_release(ghostos_ticket_lock_guard *guard, uint64_t ended_at)
{
    uint64_t duration;
    ghostos_ticket_lock *lock;
    if (!guard || guard->released || !guard->lock) return;
    lock = guard->lock;
    duration = ended_at < guard->started_at ? 0 : ended_at - guard->started_at;
    if (duration == 0) duration = guard->spins;
    atomic_fetch_add_explicit(&lock->duration_histogram[
                                  ghostos_lock_duration_bucket(duration)],
                              1, memory_order_relaxed);
    atomic_max(&lock->max_duration, duration);
    atomic_fetch_add_explicit(&lock->releases, 1, memory_order_relaxed);
    atomic_store_explicit(&lock->owner, 0, memory_order_release);
    atomic_store_explicit(&lock->serving_ticket, guard->ticket + 1,
                          memory_order_release);
    guard->released = true;
}

void ghostos_ticket_lock_guard_drop(ghostos_ticket_lock_guard *guard)
{
    if (guard) ghostos_ticket_lock_release(guard, guard->started_at);
}

ghostos_lock_shard_report ghostos_ticket_lock_report(const ghostos_ticket_lock *lock)
{
    ghostos_lock_shard_report report;
    report.active_owner = atomic_load_explicit(&lock->owner, memory_order_acquire);
    report.acquisitions = atomic_load_explicit(&lock->acquisitions, memory_order_relaxed);
    report.contended_acquisitions = atomic_load_explicit(
        &lock->contended_acquisitions, memory_order_relaxed);
    report.spins = atomic_load_explicit(&lock->spins, memory_order_relaxed);
    report.releases = atomic_load_explicit(&lock->releases, memory_order_relaxed);
    for (size_t i = 0; i < GHOSTOS_LOCK_DURATION_BUCKETS; ++i)
        report.duration_histogram[i] = atomic_load_explicit(
            &lock->duration_histogram[i], memory_order_relaxed);
    report.max_duration = atomic_load_explicit(&lock->max_duration,
                                               memory_order_relaxed);
    return report;
}

bool ghostos_sharded_ticket_lock_init(ghostos_sharded_ticket_lock *lock, size_t shards)
{
    if (!lock || shards == 0 || shards > GHOSTOS_CONTENTION_MAX_SHARDS) return false;
    lock->shard_count = shards;
    for (size_t i = 0; i < shards; ++i) ghostos_ticket_lock_init(&lock->shards[i]);
    return true;
}

ghostos_ticket_lock_guard ghostos_sharded_ticket_lock_acquire(
    ghostos_sharded_ticket_lock *lock, size_t shard, uint64_t now)
{
    if (!lock || lock->shard_count == 0) return (ghostos_ticket_lock_guard){0};
    return ghostos_ticket_lock_acquire(&lock->shards[shard % lock->shard_count], now);
}

ghostos_lock_shard_report ghostos_sharded_ticket_lock_report(
    const ghostos_sharded_ticket_lock *lock, size_t shard)
{
    if (!lock || lock->shard_count == 0) return (ghostos_lock_shard_report){0};
    return ghostos_ticket_lock_report(&lock->shards[shard % lock->shard_count]);
}

bool ghostos_sharded_ticket_lock_reports(const ghostos_sharded_ticket_lock *lock,
                                         ghostos_lock_shard_report *reports,
                                         size_t capacity)
{
    if (!lock || !reports || lock->shard_count == 0 || capacity < lock->shard_count)
        return false;
    for (size_t i = 0; i < lock->shard_count; ++i)
        reports[i] = ghostos_ticket_lock_report(&lock->shards[i]);
    return true;
}
