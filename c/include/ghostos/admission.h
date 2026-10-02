#ifndef GHOSTOS_ADMISSION_H
#define GHOSTOS_ADMISSION_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_ADMISSION_VERSION 1
#define GHOSTOS_ADMISSION_CLASSES 6
#define GHOSTOS_ADMISSION_TENANTS 32
#define GHOSTOS_ADMISSION_DEFAULT_ACTIVE 16
#define GHOSTOS_ADMISSION_DEFAULT_QUEUE 32
#define GHOSTOS_ADMISSION_DEFAULT_RESERVE 4

/* Classes: fanout, membership, snapshot, backup, packages, diagnostics (0..5).
 * Priority: optional, normal, critical, recovery (0..3).
 * Actions: admitted=1, delayed=2, dropped=3, retried=4.
 * Reasons: none=0, active capacity=1, recovery reserve=2, class limit=3,
 * queue full=4, tenant limit=5. Errors: success=0, policy=1, lease=2. */
typedef struct {
    uint16_t active_capacity, queue_capacity, recovery_reserve;
    uint16_t class_limits[GHOSTOS_ADMISSION_CLASSES];
} ghostos_admission_policy;
typedef struct { uint32_t admitted, delayed, dropped, retried, completed; } ghostos_admission_stats;
typedef struct {
    uint16_t slot;
    uint64_t sequence;
    uint8_t class_id, priority;
    uint64_t tenant;
} ghostos_admission_lease;
typedef struct { ghostos_admission_lease lease; bool occupied; } ghostos_admission_slot;
typedef struct {
    uint64_t tenant, parent;
    uint16_t active_limit, recovery_reserve;
    bool has_parent;
} ghostos_admission_tenant_policy;
typedef struct {
    ghostos_admission_tenant_policy policy;
    uint16_t active, recovery_active;
    bool occupied;
} ghostos_admission_tenant;
typedef struct {
    ghostos_admission_policy policy;
    uint16_t active_by_class[GHOSTOS_ADMISSION_CLASSES];
    ghostos_admission_stats stats[GHOSTOS_ADMISSION_CLASSES];
    uint16_t active, recovery_active, queued;
    uint64_t next_sequence;
    ghostos_admission_tenant tenants[GHOSTOS_ADMISSION_TENANTS];
} ghostos_admission_controller;
typedef struct {
    uint16_t version;
    uint64_t sequence;
    uint8_t class_id, priority, action, reason;
    uint16_t active, queued;
    ghostos_admission_lease lease;
    bool has_lease;
} ghostos_admission_outcome;
typedef struct {
    uint16_t version;
    ghostos_admission_policy policy;
    uint16_t active, recovery_active, queued;
    ghostos_admission_stats classes[GHOSTOS_ADMISSION_CLASSES];
} ghostos_admission_report;

/* Storage is caller-owned, movable, allocation-free, and never retained by C.
 * Every mutation receives the same slot array/capacity used during init. */
ghostos_admission_policy ghostos_admission_default_policy(void);
uint32_t ghostos_admission_init(ghostos_admission_controller *state,
    ghostos_admission_slot *slots, size_t capacity, const ghostos_admission_policy *policy);
void ghostos_admission_get_report(const ghostos_admission_controller *state,
    ghostos_admission_report *report);
uint32_t ghostos_admission_configure_tenant(ghostos_admission_controller *state,
    size_t capacity, const ghostos_admission_tenant_policy *policy);
void ghostos_admission_admit(ghostos_admission_controller *state,
    ghostos_admission_slot *slots, size_t capacity, uint64_t tenant,
    uint8_t class_id, uint8_t priority, ghostos_admission_outcome *outcome);
void ghostos_admission_record_retry(ghostos_admission_controller *state,
    uint8_t class_id, uint8_t priority, ghostos_admission_outcome *outcome);
uint32_t ghostos_admission_finish(ghostos_admission_controller *state,
    ghostos_admission_slot *slots, size_t capacity, const ghostos_admission_lease *lease);
const char *ghostos_admission_class_name(uint8_t class_id);

#endif
