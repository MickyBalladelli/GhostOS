#include "ghostos/quota.h"

#define MICROS_PER_SECOND UINT64_C(1000000)

static uint64_t atomic_load_relaxed(const uint64_t *value) { return __atomic_load_n(value, __ATOMIC_RELAXED); }
static uint64_t atomic_load_acquire(const uint64_t *value) { return __atomic_load_n(value, __ATOMIC_ACQUIRE); }
static void atomic_store_relaxed(uint64_t *value, uint64_t next) { __atomic_store_n(value, next, __ATOMIC_RELAXED); }
static void atomic_store_release(uint64_t *value, uint64_t next) { __atomic_store_n(value, next, __ATOMIC_RELEASE); }
static uint64_t atomic_fetch_add_relaxed(uint64_t *value, uint64_t add) { return __atomic_fetch_add(value, add, __ATOMIC_RELAXED); }
static uint64_t sat_add(uint64_t a, uint64_t b) { return UINT64_MAX - a < b ? UINT64_MAX : a + b; }
static uint64_t sat_mul(uint64_t a, uint64_t b) { return a && b > UINT64_MAX / a ? UINT64_MAX : a * b; }
static uint64_t sat_sub(uint64_t a, uint64_t b) { return a < b ? 0 : a - b; }

static size_t duration_bucket(uint64_t duration) {
    if (!duration) return 0;
    if (duration == 1) return 1;
    if (duration <= 3) return 2;
    if (duration <= 7) return 3;
    if (duration <= 15) return 4;
    if (duration <= 31) return 5;
    if (duration <= 63) return 6;
    return 7;
}

static uint64_t sat_max(uint64_t *target, uint64_t value) {
    uint64_t old = atomic_load_relaxed(target);
    while (old < value && !__atomic_compare_exchange_n(target, &old, value, true,
            __ATOMIC_RELAXED, __ATOMIC_RELAXED)) { }
    return old < value ? value : old;
}

static uint64_t lock_ticket(ghostos_quota_lock_state *lock, uint64_t now, uint64_t *spins_out) {
    uint64_t ticket = atomic_fetch_add_relaxed(&lock->next_ticket, 1);
    uint64_t spins = 0;
    while (atomic_load_acquire(&lock->serving_ticket) != ticket) {
        spins = sat_add(spins, 1);
#if defined(__x86_64__) || defined(__i386__)
        __asm__ volatile("pause");
#else
        __atomic_signal_fence(__ATOMIC_SEQ_CST);
#endif
    }
    atomic_fetch_add_relaxed(&lock->acquisitions, 1);
    if (spins) {
        atomic_fetch_add_relaxed(&lock->contended_acquisitions, 1);
        atomic_fetch_add_relaxed(&lock->spins, spins);
    }
    atomic_store_release(&lock->owner, ticket + 1);
    *spins_out = spins;
    (void)now;
    return ticket;
}

static void unlock_ticket(ghostos_quota_lock_state *lock, uint64_t ticket,
    uint64_t started, uint64_t spins, uint64_t ended) {
    uint64_t duration = sat_sub(ended, started);
    if (!duration) duration = spins;
    atomic_fetch_add_relaxed(&lock->duration_histogram[duration_bucket(duration)], 1);
    (void)sat_max(&lock->max_duration, duration);
    atomic_fetch_add_relaxed(&lock->releases, 1);
    atomic_store_release(&lock->owner, 0);
    atomic_store_release(&lock->serving_ticket, ticket + 1);
}

static ghostos_quota_policy quota_default_policy(void) {
    return (ghostos_quota_policy){
        {{1024, 4096}, {128, 128}, {UINT64_C(268435456), UINT64_C(536870912)}},
        UINT64_C(1073741824)};
}

void ghostos_quota_init(ghostos_quota_state *quota) {
    if (!quota) return;
    *quota = (ghostos_quota_state){0};
    quota->policy = quota_default_policy();
    for (size_t index = 0; index < GHOSTOS_QUOTA_SHARDS; ++index) {
        quota->buckets[index].tokens = quota->policy.resources[index].capacity;
    }
}

void ghostos_quota_configure(ghostos_quota_state *quota, ghostos_quota_policy policy) {
    if (!quota) return;
    quota->policy = policy;
    for (size_t index = 0; index < GHOSTOS_QUOTA_SHARDS; ++index) {
        quota->buckets[index] = (ghostos_quota_bucket_state){policy.resources[index].capacity, 0, 0};
    }
    atomic_store_relaxed(&quota->memory_in_use, 0);
}

ghostos_quota_policy ghostos_quota_get_policy(const ghostos_quota_state *quota) {
    return quota ? quota->policy : (ghostos_quota_policy){0};
}

static void refill(ghostos_quota_bucket_config config, ghostos_quota_bucket_state *state, uint64_t now) {
    if (now < state->last_refill_us) {
        state->last_refill_us = now;
        state->remainder = 0;
        return;
    }
    uint64_t elapsed = now - state->last_refill_us;
    uint64_t produced = sat_add(sat_mul(elapsed, config.refill_per_second), state->remainder);
    uint64_t added = produced / MICROS_PER_SECOND;
    state->remainder = produced % MICROS_PER_SECOND;
    state->tokens = sat_add(state->tokens, added);
    if (state->tokens > config.capacity) state->tokens = config.capacity;
    state->last_refill_us = now;
}

static ghostos_quota_result acquire(ghostos_quota_state *quota, uint32_t resource,
    uint64_t now, uint64_t amount) {
    ghostos_quota_bucket_config config = quota->policy.resources[resource];
    ghostos_quota_bucket_state *state = &quota->buckets[resource];
    ghostos_quota_result result = {0, GHOSTOS_QUOTA_ALLOWED};
    if (amount > config.capacity) { result.decision = GHOSTOS_QUOTA_REJECTED; return result; }
    refill(config, state, now);
    if (state->tokens >= amount) {
        state->tokens -= amount;
        if (resource == 2) {
            uint64_t in_use = atomic_load_relaxed(&quota->memory_in_use);
            atomic_store_relaxed(&quota->memory_in_use, sat_add(in_use, amount));
        }
    } else {
        uint64_t missing = amount - state->tokens;
        uint64_t delay = UINT64_MAX;
        if (config.refill_per_second) {
            uint64_t numerator = sat_mul(missing, MICROS_PER_SECOND);
            uint64_t rounded = sat_add(numerator, config.refill_per_second - 1);
            delay = rounded / config.refill_per_second;
        }
        result.decision = GHOSTOS_QUOTA_THROTTLED;
        result.retry_after_us = delay ? delay : 1;
    }
    return result;
}

ghostos_quota_result ghostos_quota_consume(ghostos_quota_state *quota, uint32_t resource,
    uint64_t now, uint64_t amount) {
    ghostos_quota_result result = {0, GHOSTOS_QUOTA_ALLOWED};
    if (!quota || resource >= GHOSTOS_QUOTA_SHARDS || !amount) return result;
    uint64_t clock = atomic_load_relaxed(&quota->clock);
    while (clock < now && !__atomic_compare_exchange_n(&quota->clock, &clock, now, true,
            __ATOMIC_RELAXED, __ATOMIC_RELAXED)) { }
    uint64_t spins = 0;
    uint64_t ticket = lock_ticket(&quota->locks[resource], now, &spins);
    if (resource == 2) {
        uint64_t in_use = atomic_load_relaxed(&quota->memory_in_use);
        if (amount > quota->policy.max_memory_bytes || in_use > quota->policy.max_memory_bytes - amount) {
            result.decision = GHOSTOS_QUOTA_REJECTED;
        } else {
            result = acquire(quota, resource, now, amount);
        }
    } else result = acquire(quota, resource, now, amount);
    uint64_t end = atomic_load_relaxed(&quota->clock);
    unlock_ticket(&quota->locks[resource], ticket, now, spins, end);
    return result;
}

void ghostos_quota_refund(ghostos_quota_state *quota, uint32_t resource, uint64_t amount) {
    if (!quota || resource >= GHOSTOS_QUOTA_SHARDS || !amount) return;
    uint64_t now = atomic_load_relaxed(&quota->clock), spins = 0;
    uint64_t ticket = lock_ticket(&quota->locks[resource], now, &spins);
    ghostos_quota_bucket_config config = quota->policy.resources[resource];
    ghostos_quota_bucket_state *state = &quota->buckets[resource];
    state->tokens = sat_add(state->tokens, amount);
    if (state->tokens > config.capacity) state->tokens = config.capacity;
    if (resource == 2) atomic_store_relaxed(&quota->memory_in_use,
        sat_sub(atomic_load_relaxed(&quota->memory_in_use), amount));
    unlock_ticket(&quota->locks[resource], ticket, now, spins, atomic_load_relaxed(&quota->clock));
}

void ghostos_quota_release_memory(ghostos_quota_state *quota, uint64_t amount) {
    if (!quota || !amount) return;
    uint64_t now = atomic_load_relaxed(&quota->clock), spins = 0;
    uint64_t ticket = lock_ticket(&quota->locks[2], now, &spins);
    atomic_store_relaxed(&quota->memory_in_use,
        sat_sub(atomic_load_relaxed(&quota->memory_in_use), amount));
    unlock_ticket(&quota->locks[2], ticket, now, spins, atomic_load_relaxed(&quota->clock));
}

uint64_t ghostos_quota_memory_in_use(const ghostos_quota_state *quota) {
    return quota ? atomic_load_relaxed(&quota->memory_in_use) : 0;
}

void ghostos_quota_lock_reports(const ghostos_quota_state *quota,
    ghostos_quota_lock_report reports[GHOSTOS_QUOTA_SHARDS]) {
    if (!quota || !reports) return;
    for (size_t index = 0; index < GHOSTOS_QUOTA_SHARDS; ++index) {
        const ghostos_quota_lock_state *lock = &quota->locks[index];
        ghostos_quota_lock_report *report = &reports[index];
        report->active_owner = atomic_load_acquire(&lock->owner);
        report->acquisitions = atomic_load_relaxed(&lock->acquisitions);
        report->contended_acquisitions = atomic_load_relaxed(&lock->contended_acquisitions);
        report->spins = atomic_load_relaxed(&lock->spins);
        report->releases = atomic_load_relaxed(&lock->releases);
        for (size_t bucket = 0; bucket < GHOSTOS_QUOTA_DURATION_BUCKETS; ++bucket) {
            report->duration_histogram[bucket] = atomic_load_relaxed(&lock->duration_histogram[bucket]);
        }
        report->max_duration = atomic_load_relaxed(&lock->max_duration);
    }
}
