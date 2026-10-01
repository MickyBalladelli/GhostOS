#ifndef GHOSTOS_CONTENTION_H
#define GHOSTOS_CONTENTION_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdatomic.h>

#define GHOSTOS_LOCK_DURATION_BUCKETS 8u
#define GHOSTOS_CONTENTION_MAX_SHARDS 64u

typedef struct {
    uint64_t active_owner;
    uint64_t acquisitions;
    uint64_t contended_acquisitions;
    uint64_t spins;
    uint64_t releases;
    uint64_t duration_histogram[GHOSTOS_LOCK_DURATION_BUCKETS];
    uint64_t max_duration;
} ghostos_lock_shard_report;

typedef struct {
    atomic_uint_fast64_t next_ticket;
    atomic_uint_fast64_t serving_ticket;
    atomic_uint_fast64_t owner;
    atomic_uint_fast64_t acquisitions;
    atomic_uint_fast64_t contended_acquisitions;
    atomic_uint_fast64_t spins;
    atomic_uint_fast64_t releases;
    atomic_uint_fast64_t duration_histogram[GHOSTOS_LOCK_DURATION_BUCKETS];
    atomic_uint_fast64_t max_duration;
} ghostos_ticket_lock;

typedef struct {
    ghostos_ticket_lock *lock;
    uint64_t ticket;
    uint64_t started_at;
    uint64_t spins;
    bool released;
} ghostos_ticket_lock_guard;

typedef struct {
    size_t shard_count;
    ghostos_ticket_lock shards[GHOSTOS_CONTENTION_MAX_SHARDS];
} ghostos_sharded_ticket_lock;

size_t ghostos_lock_duration_bucket(uint64_t duration);
void ghostos_ticket_lock_init(ghostos_ticket_lock *lock);
/* Keep each guard in one place; release or drop it exactly once. */
ghostos_ticket_lock_guard ghostos_ticket_lock_acquire(ghostos_ticket_lock *lock,
                                                      uint64_t now);
void ghostos_ticket_lock_release(ghostos_ticket_lock_guard *guard, uint64_t ended_at);
void ghostos_ticket_lock_guard_drop(ghostos_ticket_lock_guard *guard);
ghostos_lock_shard_report ghostos_ticket_lock_report(const ghostos_ticket_lock *lock);
bool ghostos_sharded_ticket_lock_init(ghostos_sharded_ticket_lock *lock, size_t shards);
ghostos_ticket_lock_guard ghostos_sharded_ticket_lock_acquire(
    ghostos_sharded_ticket_lock *lock, size_t shard, uint64_t now);
ghostos_lock_shard_report ghostos_sharded_ticket_lock_report(
    const ghostos_sharded_ticket_lock *lock, size_t shard);
bool ghostos_sharded_ticket_lock_reports(const ghostos_sharded_ticket_lock *lock,
                                         ghostos_lock_shard_report *reports,
                                         size_t capacity);

#endif
