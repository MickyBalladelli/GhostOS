#ifndef GHOSTOS_LITMUS_H
#define GHOSTOS_LITMUS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_LITMUS_MAX_SCHEDULE_STEPS 32u
#define GHOSTOS_LITMUS_COUNT 6u

typedef enum {
    GHOSTOS_LITMUS_IPC_ORDERING = 0,
    GHOSTOS_LITMUS_CAPABILITY_REVOCATION = 1,
    GHOSTOS_LITMUS_MEMORY_VISIBILITY = 2,
    GHOSTOS_LITMUS_INTERRUPT_RACE = 3,
    GHOSTOS_LITMUS_SCHEDULER_PREEMPTION = 4,
    GHOSTOS_LITMUS_WAKEUP_AFTER_HLT = 5
} ghostos_litmus_kind;

typedef enum {
    GHOSTOS_LITMUS_FAILURE_NONE = 0,
    GHOSTOS_LITMUS_IPC_REORDERED = 1,
    GHOSTOS_LITMUS_REVOKED_CAPABILITY_USED = 2,
    GHOSTOS_LITMUS_PUBLISHED_DATA_NOT_VISIBLE = 3,
    GHOSTOS_LITMUS_INTERRUPT_LOST = 4,
    GHOSTOS_LITMUS_PREEMPTION_MISSED = 5,
    GHOSTOS_LITMUS_WAKEUP_LOST = 6
} ghostos_litmus_failure;

typedef enum {
    GHOSTOS_LITMUS_FAULT_NONE = 0,
    GHOSTOS_LITMUS_FAULT_IPC_REORDER = 1,
    GHOSTOS_LITMUS_FAULT_STALE_CAPABILITY_CACHE = 2,
    GHOSTOS_LITMUS_FAULT_PUBLISH_BEFORE_WRITE = 3,
    GHOSTOS_LITMUS_FAULT_LOST_INTERRUPT_WAKEUP = 4,
    GHOSTOS_LITMUS_FAULT_MISSED_PREEMPTION = 5,
    GHOSTOS_LITMUS_FAULT_LOST_HLT_WAKEUP = 6
} ghostos_litmus_fault;

typedef struct {
    uint8_t steps[GHOSTOS_LITMUS_MAX_SCHEDULE_STEPS];
    uint8_t len;
} ghostos_litmus_schedule;

typedef struct {
    ghostos_litmus_kind kind;
    uint64_t seed;
    ghostos_litmus_schedule interleaving;
    ghostos_litmus_failure failure;
    bool has_failure;
    ghostos_litmus_schedule minimal_failing_schedule;
    bool has_minimal_failing_schedule;
} ghostos_litmus_case_report;

typedef struct {
    uint64_t seed;
    ghostos_litmus_case_report cases[GHOSTOS_LITMUS_COUNT];
} ghostos_litmus_suite_report;

ghostos_litmus_schedule ghostos_litmus_schedule_empty(void);
bool ghostos_litmus_schedule_push(ghostos_litmus_schedule *schedule, uint8_t actor);
ghostos_litmus_case_report ghostos_litmus_run_case(ghostos_litmus_kind kind, uint64_t seed);
ghostos_litmus_suite_report ghostos_litmus_run_suite(uint64_t seed);
ghostos_litmus_case_report ghostos_litmus_replay_with_fault(
    ghostos_litmus_kind kind, const ghostos_litmus_schedule *schedule,
    ghostos_litmus_fault fault, uint64_t seed);

/* Rust-compatible malformed public schedule handling: false means an
 * out-of-bounds access would occur during execution or minimization. */
bool ghostos_litmus_replay_checked(ghostos_litmus_kind kind,
    const ghostos_litmus_schedule *schedule, ghostos_litmus_fault fault,
    uint64_t seed, ghostos_litmus_case_report *report);

#endif
