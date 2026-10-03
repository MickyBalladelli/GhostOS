#include "ghostos/ras.h"
#include <assert.h>

/* Ports of crates/ras/tests/ras_records_and_memory_quarantine.rs. */
static void records_and_quarantine(void) {
    ghostos_ras_telemetry telemetry = {.next_sequence = 1};
    ghostos_ras_event_slot events[2] = {0};
    ghostos_ras_event event = {0, 1, 1, 0, 0, 3, 4096, 7};
    assert(ghostos_ras_record(&telemetry, events, 2, &event) == 1);
    event = (ghostos_ras_event){0, 2, 1, 0, 3, 3, 8192, 8};
    assert(ghostos_ras_record(&telemetry, events, 2, &event) == 2);
    event = (ghostos_ras_event){0, 3, 1, 1, 3, 4, 12288, 9};
    assert(ghostos_ras_record(&telemetry, events, 2, &event) == 3);
    assert(telemetry.dropped == 1);
    assert(telemetry.counters.corrected_ecc == 1);
    assert(telemetry.counters.uncorrected_ecc == 1);
    assert(telemetry.counters.cxl_poisoned_flits == 1);
    assert(ghostos_ras_event_get(&telemetry, events, 2, 0, &event));
    assert(event.severity == 3);
    ghostos_ras_poison poison[1] = {0};
    ghostos_ras_poison range = {1, 1, 16384, 4096, 1, true};
    assert(ghostos_ras_quarantine(poison, 1, range, true) == 0);
    assert(ghostos_ras_admit(poison, 1, 1, 16384, 4096, true) == 1);
    range.sequence = 2;
    assert(ghostos_ras_quarantine(poison, 1, range, true) == 2);
    ghostos_ras_poison ecc[4] = {0};
    range = (ghostos_ras_poison){1, 5, 20480, 4096, 1, true};
    assert(ghostos_ras_quarantine(ecc, 4, range, true) == 0);
    assert(ghostos_ras_admit(ecc, 4, 1, 20480, 4096, true) == 1);
}
static void budget_prediction(void) {
    ghostos_ras_budget_policy policy = {70000, 90000, 100, 150, 1000000};
    ghostos_ras_budget_reading reading = {1, 60000, 80, 40000, 80};
    ghostos_ras_workload workloads[2] = {0};
    assert(ghostos_ras_workload_register(workloads, 2, 1, 1) == 0);
    assert(ghostos_ras_workload_register(workloads, 2, 2, 3) == 0);
    ghostos_ras_budget_plan plan = ghostos_ras_budget_decide(policy, reading);
    assert(plan.mode == 2 && plan.throttle_percent == 75);
    size_t index = ghostos_ras_workload_next(workloads, 2, 0);
    assert(index == 0 && workloads[index].id == 1);
    ghostos_ras_workload_evicted(workloads, index);
    assert(ghostos_ras_workload_next(workloads, 2, 0) == 2);
}
static void aer_isolation(void) {
    assert(ghostos_ras_aer_action(1, 0, 0) == 1);
    assert(ghostos_ras_aer_action(0, 1, 0) == 2);
}
/* Regression: the selected slot must leave Dirty, not mutate a copy. */
static void persistent_flush_progress(void) {
    ghostos_ras_dirty_page pages[2] = {0};
    assert(ghostos_ras_mark_dirty(pages, 2, 1, 1, 4096) == 0);
    assert(ghostos_ras_mark_dirty(pages, 2, 1, 1, 8192) == 0);
    size_t index = ghostos_ras_flush_begin(pages, 2);
    assert(index == 0 && pages[0].state == 1);
    ghostos_ras_flush_finish(pages, index, true);
    assert(pages[0].state == 2);
    index = ghostos_ras_flush_begin(pages, 2);
    assert(index == 1);
    ghostos_ras_flush_finish(pages, index, false);
    assert(pages[1].state == 3 && ghostos_ras_flush_begin(pages, 2) == 2);
    ghostos_ras_clear_clean(pages, 2);
    assert(!pages[0].occupied && pages[1].occupied);
    assert(ghostos_ras_recover_pages(pages, 2, UINT64_MAX) == 1);
    assert(pages[1].state == 0 && ghostos_ras_flush_begin(pages, 2) == 1);
}
int main(void) {
    records_and_quarantine();
    budget_prediction();
    aer_isolation();
    persistent_flush_progress();
    return 0;
}
