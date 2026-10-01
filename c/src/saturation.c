#include "ghostos/saturation.h"

#define CONTROL_QUEUE_CAPACITY 8u
#define INTERRUPT_QUEUE_CAPACITY 8u
#define DEFERRED_QUEUE_CAPACITY 64u
#define BULK_QUEUE_CAPACITY 64u

typedef struct {
    uint32_t count;
    uint64_t oldest_tick;
    uint32_t accepted;
    uint32_t serviced;
    uint32_t dropped;
    uint64_t max_latency_ticks;
    uint32_t throttled;
} pending_work;

static void arrive(pending_work *work, uint64_t now, uint32_t amount, uint32_t capacity) {
    uint32_t room = capacity - work->count;
    uint32_t accepted = amount < room ? amount : room;
    if (accepted) {
        if (!work->count) work->oldest_tick = now;
        work->count += accepted;
        work->accepted += accepted;
    }
    work->dropped += amount - accepted;
}

static void service_one(pending_work *work, uint64_t now) {
    if (!work->count) return;
    uint64_t latency = now >= work->oldest_tick ? now - work->oldest_tick : 0;
    if (latency > work->max_latency_ticks) work->max_latency_ticks = latency;
    --work->count;
    ++work->serviced;
    if (!work->count) work->oldest_tick = 0;
}

void ghostos_saturation_prove(const ghostos_saturation_config *config,
    ghostos_saturation_report *report) {
    if (!report) return;
    *report = (ghostos_saturation_report){0};
    if (!config) return;
    pending_work interrupts = {0}, control = {0}, deferred = {0}, bulk = {0};
    uint32_t timer_due = 0, timer_serviced = 0;
    uint64_t timer_period = config->timer_period_ticks ? config->timer_period_ticks : 1;
    uint64_t tick = 0;
    while (tick < config->ticks) {
        arrive(&interrupts, tick, config->interrupts_per_tick, INTERRUPT_QUEUE_CAPACITY);
        arrive(&control, tick, config->control_per_tick, CONTROL_QUEUE_CAPACITY);
        arrive(&deferred, tick, config->deferred_per_tick, DEFERRED_QUEUE_CAPACITY);
        arrive(&bulk, tick, config->bulk_per_tick, BULK_QUEUE_CAPACITY);
        bool timer_due_now = tick % timer_period == 0;
        if (timer_due_now) ++timer_due;
        service_one(&interrupts, tick);
        service_one(&control, tick);
        if (timer_due_now) ++timer_serviced;
        service_one(&deferred, tick);
        if (tick % GHOSTOS_SATURATION_BULK_SERVICE_PERIOD_TICKS == 0) {
            service_one(&bulk, tick);
        } else if (bulk.count) {
            ++bulk.throttled;
        }
        ++tick;
    }
    *report = (ghostos_saturation_report){
        interrupts.accepted, interrupts.serviced, interrupts.dropped, interrupts.max_latency_ticks,
        control.accepted, control.serviced, control.dropped, control.max_latency_ticks,
        timer_due, timer_serviced, 0,
        deferred.accepted, deferred.serviced, deferred.dropped, deferred.max_latency_ticks,
        bulk.accepted, bulk.serviced, bulk.dropped, bulk.throttled,
    };
}
