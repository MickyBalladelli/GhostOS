#include "ghostos/reconfigure.h"
int ghostos_reconfigure_prepare(size_t history_length, size_t history_capacity, bool has_active,
    uint64_t active_revision, bool has_expected, uint64_t expected_revision, uint64_t update_revision) {
    if (history_length == history_capacity || !history_capacity) return 1;
    if (has_expected && (!has_active || active_revision != expected_revision)) return 2;
    if (has_active && active_revision == update_revision) return 3;
    return 0;
}
int ghostos_reconfigure_commit(ghostos_reconfigure_entry *history, size_t history_capacity, size_t *history_length,
    uint64_t *active_revision, bool *has_active, uint64_t update_revision, uint64_t generation, uint64_t previous_generation) {
    ghostos_reconfigure_entry *entry;
    (void)previous_generation;
    if (*history_length >= history_capacity) return 1;
    entry = &history[*history_length];
    entry->revision = update_revision;
    entry->previous_revision = *has_active ? *active_revision : 0;
    entry->generation = generation;
    entry->has_previous = *has_active;
    entry->occupied = true;
    *active_revision = update_revision;
    *has_active = true;
    *history_length += 1;
    return 0;
}
int ghostos_reconfigure_rollback(const ghostos_reconfigure_entry *history, size_t history_length, size_t *index,
    uint64_t *revision, uint64_t *previous_revision, uint64_t *previous_generation) {
    const ghostos_reconfigure_entry *entry;
    if (!history_length) return 1;
    *index = history_length - 1;
    entry = &history[*index];
    if (!entry->occupied) return 1;
    *revision = entry->has_previous ? entry->previous_revision : 0;
    *previous_revision = entry->revision;
    *previous_generation = entry->generation;
    return 0;
}
