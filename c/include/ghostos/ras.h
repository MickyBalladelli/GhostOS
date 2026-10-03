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
#endif
