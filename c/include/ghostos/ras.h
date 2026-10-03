#ifndef GHOSTOS_RAS_H
#define GHOSTOS_RAS_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#define GHOSTOS_RAS_PAGE_SIZE UINT64_C(4096)
typedef struct {
    uint64_t corrected_ecc, uncorrected_ecc, cxl_poisoned_flits;
    uint64_t aer_correctable, aer_non_fatal, aer_fatal;
} ghostos_ras_counters;
typedef struct {
    uint64_t sequence, timestamp_us;
    uint32_t node, source, severity;
    uint64_t device, address;
    uint32_t detail;
} ghostos_ras_event;
typedef struct { ghostos_ras_event event; bool occupied; } ghostos_ras_event_slot;
typedef struct {
    size_t cursor, count;
    uint64_t dropped, next_sequence;
    ghostos_ras_counters counters;
} ghostos_ras_telemetry;
typedef struct {
    uint32_t node;
    uint64_t device, start, length, sequence;
    bool occupied;
} ghostos_ras_poison;
/* Telemetry capacity must be positive; initialize next_sequence to 1 and
 * all other state/slots to zero. Poison slots start with occupied=false. */
uint64_t ghostos_ras_record(ghostos_ras_telemetry *state,
    ghostos_ras_event_slot *slots, size_t capacity, ghostos_ras_event *event);
bool ghostos_ras_event_get(const ghostos_ras_telemetry *state,
    const ghostos_ras_event_slot *slots, size_t capacity, size_t offset, ghostos_ras_event *event);
/* Quarantine: 0 success, 1 invalid alignment, 2 overlap, 3 full, -1 bounds overflow.
 * Admission: 0 success, 1 poisoned, -1 bounds overflow. Checked mirrors Rust debug arithmetic. */
int ghostos_ras_quarantine(ghostos_ras_poison *slots, size_t capacity,
    ghostos_ras_poison poison, bool checked);
int ghostos_ras_admit(const ghostos_ras_poison *slots, size_t capacity,
    uint32_t node, uint64_t start, uint64_t length, bool checked);
typedef struct {
    uint32_t thermal_soft, thermal_critical, power_soft, power_critical;
    uint64_t horizon_us;
} ghostos_ras_budget_policy;
typedef struct {
    uint64_t timestamp_us;
    uint32_t thermal, power;
    int32_t thermal_rate, power_rate;
} ghostos_ras_budget_reading;
typedef struct { uint32_t mode; uint8_t throttle_percent; } ghostos_ras_budget_plan;
typedef struct { uint64_t id; uint8_t priority; bool active, occupied; } ghostos_ras_workload;
/* Register: 0 success, 1 duplicate, 2 full. Unregister: false means missing. */
int ghostos_ras_workload_register(ghostos_ras_workload *slots, size_t capacity, uint64_t id, uint8_t priority);
bool ghostos_ras_workload_unregister(ghostos_ras_workload *slots, size_t capacity, uint64_t id);
ghostos_ras_budget_plan ghostos_ras_budget_decide(ghostos_ras_budget_policy policy,
    ghostos_ras_budget_reading reading);
/* Select without mutation; commit after controller approval. Capacity means no selection. */
size_t ghostos_ras_workload_next(const ghostos_ras_workload *slots, size_t capacity, size_t start);
void ghostos_ras_workload_evicted(ghostos_ras_workload *slots, size_t index);
#endif
