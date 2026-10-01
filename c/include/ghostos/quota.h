#ifndef GHOSTOS_QUOTA_H
#define GHOSTOS_QUOTA_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_QUOTA_SHARDS 3u
#define GHOSTOS_QUOTA_DURATION_BUCKETS 8u

typedef struct { uint64_t capacity, refill_per_second; } ghostos_quota_bucket_config;
typedef struct {
    ghostos_quota_bucket_config resources[GHOSTOS_QUOTA_SHARDS];
    uint64_t max_memory_bytes;
} ghostos_quota_policy;
typedef struct { uint64_t tokens, last_refill_us, remainder; } ghostos_quota_bucket_state;
typedef struct {
    uint64_t next_ticket, serving_ticket, owner;
    uint64_t acquisitions, contended_acquisitions, spins, releases;
    uint64_t duration_histogram[GHOSTOS_QUOTA_DURATION_BUCKETS];
    uint64_t max_duration;
} ghostos_quota_lock_state;
typedef struct {
    ghostos_quota_policy policy;
    ghostos_quota_lock_state locks[GHOSTOS_QUOTA_SHARDS];
    uint64_t clock;
    ghostos_quota_bucket_state buckets[GHOSTOS_QUOTA_SHARDS];
    uint64_t memory_in_use;
} ghostos_quota_state;
typedef struct {
    uint64_t active_owner, acquisitions, contended_acquisitions, spins, releases;
    uint64_t duration_histogram[GHOSTOS_QUOTA_DURATION_BUCKETS];
    uint64_t max_duration;
} ghostos_quota_lock_report;
typedef struct { uint64_t retry_after_us; uint32_t decision; } ghostos_quota_result;

enum { GHOSTOS_QUOTA_ALLOWED, GHOSTOS_QUOTA_THROTTLED, GHOSTOS_QUOTA_REJECTED };

void ghostos_quota_init(ghostos_quota_state *quota);
void ghostos_quota_configure(ghostos_quota_state *quota, ghostos_quota_policy policy);
ghostos_quota_policy ghostos_quota_get_policy(const ghostos_quota_state *quota);
ghostos_quota_result ghostos_quota_consume(ghostos_quota_state *quota, uint32_t resource,
    uint64_t now_us, uint64_t amount);
void ghostos_quota_refund(ghostos_quota_state *quota, uint32_t resource, uint64_t amount);
void ghostos_quota_release_memory(ghostos_quota_state *quota, uint64_t amount);
uint64_t ghostos_quota_memory_in_use(const ghostos_quota_state *quota);
void ghostos_quota_lock_reports(const ghostos_quota_state *quota,
    ghostos_quota_lock_report reports[GHOSTOS_QUOTA_SHARDS]);

#endif
