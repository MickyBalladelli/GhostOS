#ifndef GHOSTOS_RECONFIGURE_H
#define GHOSTOS_RECONFIGURE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Prepare: 0 ready, 1 history full, 2 stale revision, 3 already active.
 * Rollback: 0 ready, 1 no rollback target. */
typedef struct {
    uint64_t revision, previous_revision, generation;
    bool has_previous, occupied;
} ghostos_reconfigure_entry;
int ghostos_reconfigure_prepare(size_t history_length, size_t history_capacity, bool has_active,
    uint64_t active_revision, bool has_expected, uint64_t expected_revision, uint64_t update_revision);
int ghostos_reconfigure_commit(ghostos_reconfigure_entry *history, size_t history_capacity, size_t *history_length,
    uint64_t *active_revision, bool *has_active, uint64_t update_revision, uint64_t generation, uint64_t previous_generation);
int ghostos_reconfigure_rollback(const ghostos_reconfigure_entry *history, size_t history_length, size_t *index,
    uint64_t *revision, uint64_t *previous_revision, uint64_t *previous_generation);
#endif
