#ifndef GHOSTOS_SATURATION_H
#define GHOSTOS_SATURATION_H

#include <stdint.h>

#define GHOSTOS_SATURATION_CONTROL_LATENCY_BUDGET_TICKS UINT64_C(1)
#define GHOSTOS_SATURATION_INTERRUPT_LATENCY_BUDGET_TICKS UINT64_C(1)
#define GHOSTOS_SATURATION_TIMER_LATENCY_BUDGET_TICKS UINT64_C(1)
#define GHOSTOS_SATURATION_DEFERRED_LATENCY_BUDGET_TICKS UINT64_C(1)
#define GHOSTOS_SATURATION_BULK_SERVICE_PERIOD_TICKS UINT64_C(4)

typedef struct {
    uint64_t ticks;
    uint32_t interrupts_per_tick;
    uint32_t control_per_tick;
    uint64_t timer_period_ticks;
    uint32_t deferred_per_tick;
    uint32_t bulk_per_tick;
} ghostos_saturation_config;

typedef struct {
    uint32_t interrupt_accepted;
    uint32_t interrupt_serviced;
    uint32_t interrupt_dropped;
    uint64_t interrupt_max_latency_ticks;
    uint32_t control_accepted;
    uint32_t control_serviced;
    uint32_t control_dropped;
    uint64_t control_max_latency_ticks;
    uint32_t timer_due;
    uint32_t timer_serviced;
    uint64_t timer_max_latency_ticks;
    uint32_t deferred_accepted;
    uint32_t deferred_serviced;
    uint32_t deferred_dropped;
    uint64_t deferred_max_latency_ticks;
    uint32_t bulk_accepted;
    uint32_t bulk_serviced;
    uint32_t bulk_dropped;
    uint32_t bulk_throttled;
} ghostos_saturation_report;

_Static_assert(sizeof(ghostos_saturation_config) == 32, "saturation config layout");
_Static_assert(sizeof(ghostos_saturation_report) == 104, "saturation report layout");

void ghostos_saturation_prove(const ghostos_saturation_config *config,
    ghostos_saturation_report *report);

#endif
