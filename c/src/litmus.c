#include "ghostos/litmus.h"

typedef struct { uint64_t state; } prng;
typedef struct { uint8_t producer_pc, consumer_pc, queue[2], count, received[2]; } ipc_model;
typedef struct { uint8_t owner_pc, borrower_pc; bool root_exists, child_exists, revoked, child_authorized; } capability_model;
typedef struct { uint8_t writer_pc, reader_pc; uint64_t data; bool published, observed_published; } memory_model;
typedef struct { uint8_t cpu_pc, device_pc, raised, handled; bool armed, sleeping, pending; } interrupt_model;
typedef struct { uint8_t timer_pc, worker_pc, current, preemptions; bool high_priority_ready; } scheduler_model;
typedef struct { uint8_t cpu_pc, interrupt_pc; bool armed, sleeping, event; } hlt_model;

typedef struct {
    ghostos_litmus_kind kind;
    union { ipc_model ipc; capability_model capability; memory_model memory;
        interrupt_model interrupt; scheduler_model scheduler; hlt_model hlt; } state;
} machine;

static bool valid_kind(ghostos_litmus_kind kind) {
    return kind >= GHOSTOS_LITMUS_IPC_ORDERING && kind <= GHOSTOS_LITMUS_WAKEUP_AFTER_HLT;
}

ghostos_litmus_schedule ghostos_litmus_schedule_empty(void) {
    ghostos_litmus_schedule schedule = {{0}, 0};
    return schedule;
}

bool ghostos_litmus_schedule_push(ghostos_litmus_schedule *schedule, uint8_t actor) {
    if (!schedule || schedule->len >= GHOSTOS_LITMUS_MAX_SCHEDULE_STEPS) return false;
    schedule->steps[schedule->len++] = actor;
    return true;
}

static ghostos_litmus_schedule without_step(const ghostos_litmus_schedule *schedule, size_t removed) {
    ghostos_litmus_schedule result = ghostos_litmus_schedule_empty();
    for (size_t i = 0; i < schedule->len && i < GHOSTOS_LITMUS_MAX_SCHEDULE_STEPS; ++i)
        if (i != removed) (void)ghostos_litmus_schedule_push(&result, schedule->steps[i]);
    return result;
}

static uint64_t next_random(prng *random) {
    uint64_t value = random->state;
    value ^= value << 7;
    value ^= value >> 9;
    value ^= value << 8;
    random->state = value;
    return value;
}

static machine new_machine(ghostos_litmus_kind kind) {
    machine result = {0};
    result.kind = kind;
    switch (kind) {
        case GHOSTOS_LITMUS_IPC_ORDERING: break;
        case GHOSTOS_LITMUS_CAPABILITY_REVOCATION: break;
        case GHOSTOS_LITMUS_MEMORY_VISIBILITY: break;
        case GHOSTOS_LITMUS_INTERRUPT_RACE: break;
        case GHOSTOS_LITMUS_SCHEDULER_PREEMPTION:
            result.state.scheduler.current = 1;
            result.state.scheduler.high_priority_ready = true;
            break;
        case GHOSTOS_LITMUS_WAKEUP_AFTER_HLT: break;
    }
    return result;
}

static bool enabled(const machine *m, uint8_t actor) {
    switch (m->kind) {
        case GHOSTOS_LITMUS_IPC_ORDERING:
            return actor == 0 ? m->state.ipc.producer_pc < 2 && m->state.ipc.count < 2 :
                actor == 1 && m->state.ipc.consumer_pc < 2 && m->state.ipc.count > 0;
        case GHOSTOS_LITMUS_CAPABILITY_REVOCATION:
            return actor == 0 ? m->state.capability.owner_pc < 3 :
                actor == 1 && m->state.capability.borrower_pc < 1 && m->state.capability.child_exists;
        case GHOSTOS_LITMUS_MEMORY_VISIBILITY:
            return actor == 0 ? m->state.memory.writer_pc < 2 : actor == 1 && m->state.memory.reader_pc < 2;
        case GHOSTOS_LITMUS_INTERRUPT_RACE:
            return actor == 0 ? m->state.interrupt.device_pc < 1 : actor == 1 && m->state.interrupt.cpu_pc < 2;
        case GHOSTOS_LITMUS_SCHEDULER_PREEMPTION:
            return actor == 0 ? m->state.scheduler.timer_pc < 1 : actor == 1 && m->state.scheduler.worker_pc < 1;
        case GHOSTOS_LITMUS_WAKEUP_AFTER_HLT:
            return actor == 0 ? m->state.hlt.cpu_pc < 2 : actor == 1 && m->state.hlt.interrupt_pc < 1;
    }
    return false;
}

static ghostos_litmus_failure failure(const machine *m) {
    if (m->kind == GHOSTOS_LITMUS_INTERRUPT_RACE && m->state.interrupt.raised != m->state.interrupt.handled && m->state.interrupt.cpu_pc == 2)
        return GHOSTOS_LITMUS_INTERRUPT_LOST;
    if (m->kind == GHOSTOS_LITMUS_WAKEUP_AFTER_HLT && m->state.hlt.event && m->state.hlt.sleeping && m->state.hlt.cpu_pc == 2)
        return GHOSTOS_LITMUS_WAKEUP_LOST;
    return GHOSTOS_LITMUS_FAILURE_NONE;
}

static ghostos_litmus_failure step(machine *m, uint8_t actor, ghostos_litmus_fault fault) {
    switch (m->kind) {
        case GHOSTOS_LITMUS_IPC_ORDERING: {
            ipc_model *s = &m->state.ipc;
            if (actor == 0) {
                if (fault == GHOSTOS_LITMUS_FAULT_IPC_REORDER && s->producer_pc == 1) { s->queue[0] = 2; s->queue[1] = 1; }
                else s->queue[s->count] = (uint8_t)(s->producer_pc + 1);
                ++s->count; ++s->producer_pc;
            } else {
                uint8_t message = s->queue[0]; s->queue[0] = s->queue[1]; s->queue[1] = 0;
                --s->count; s->received[s->consumer_pc++] = message;
                if (message != s->consumer_pc) return GHOSTOS_LITMUS_IPC_REORDERED;
            }
            break;
        }
        case GHOSTOS_LITMUS_CAPABILITY_REVOCATION: {
            capability_model *s = &m->state.capability;
            if (actor == 0) {
                if (s->owner_pc == 0) s->root_exists = true;
                else if (s->owner_pc == 1) s->child_exists = s->root_exists;
                else if (s->owner_pc == 2) s->revoked = true;
                ++s->owner_pc;
            } else {
                s->child_authorized = fault == GHOSTOS_LITMUS_FAULT_STALE_CAPABILITY_CACHE || !s->revoked;
                ++s->borrower_pc;
                if (s->revoked && s->child_authorized) return GHOSTOS_LITMUS_REVOKED_CAPABILITY_USED;
            }
            break;
        }
        case GHOSTOS_LITMUS_MEMORY_VISIBILITY: {
            memory_model *s = &m->state.memory;
            if (actor == 0) {
                if (s->writer_pc == 0) { if (fault == GHOSTOS_LITMUS_FAULT_PUBLISH_BEFORE_WRITE) s->published = true; else s->data = 42; }
                else { s->published = true; if (fault == GHOSTOS_LITMUS_FAULT_PUBLISH_BEFORE_WRITE) s->data = 42; }
                ++s->writer_pc;
            } else if (s->reader_pc++ == 0) s->observed_published = s->published;
            else if (s->observed_published && s->data != 42) return GHOSTOS_LITMUS_PUBLISHED_DATA_NOT_VISIBLE;
            break;
        }
        case GHOSTOS_LITMUS_INTERRUPT_RACE: {
            interrupt_model *s = &m->state.interrupt;
            if (actor == 0) {
                s->device_pc = 1; ++s->raised; s->pending = true;
                if (s->armed && s->sleeping && fault != GHOSTOS_LITMUS_FAULT_LOST_INTERRUPT_WAKEUP) s->sleeping = false;
            } else if (s->cpu_pc == 0) {
                s->armed = true;
                if (s->pending) { s->pending = false; ++s->handled; } else s->sleeping = true;
                s->cpu_pc = 1;
            } else {
                if (!s->sleeping && s->pending) { s->pending = false; ++s->handled; }
                s->cpu_pc = 2;
            }
            break;
        }
        case GHOSTOS_LITMUS_SCHEDULER_PREEMPTION: {
            scheduler_model *s = &m->state.scheduler;
            if (actor == 0) {
                s->timer_pc = 1;
                if (s->high_priority_ready && fault != GHOSTOS_LITMUS_FAULT_MISSED_PREEMPTION) { s->current = 0; ++s->preemptions; }
                if (s->high_priority_ready && s->current != 0) return GHOSTOS_LITMUS_PREEMPTION_MISSED;
            } else s->worker_pc = 1;
            break;
        }
        case GHOSTOS_LITMUS_WAKEUP_AFTER_HLT: {
            hlt_model *s = &m->state.hlt;
            if (actor == 0) {
                if (s->cpu_pc == 0) { s->armed = true; s->cpu_pc = 1; }
                else { if (!s->event) s->sleeping = true; s->cpu_pc = 2; }
            } else {
                s->interrupt_pc = 1; s->event = true;
                if (s->armed && s->sleeping && fault != GHOSTOS_LITMUS_FAULT_LOST_HLT_WAKEUP) s->sleeping = false;
            }
            break;
        }
    }
    return failure(m);
}

static ghostos_litmus_failure execute(ghostos_litmus_kind kind, const ghostos_litmus_schedule *schedule, ghostos_litmus_fault fault) {
    if (!valid_kind(kind) || !schedule || schedule->len > GHOSTOS_LITMUS_MAX_SCHEDULE_STEPS) return GHOSTOS_LITMUS_FAILURE_NONE;
    machine m = new_machine(kind);
    for (size_t i = 0; i < schedule->len; ++i) {
        uint8_t actor = schedule->steps[i];
        if (!enabled(&m, actor)) return GHOSTOS_LITMUS_FAILURE_NONE;
        ghostos_litmus_failure found = step(&m, actor, fault);
        if (found != GHOSTOS_LITMUS_FAILURE_NONE) return found;
    }
    return failure(&m);
}

static ghostos_litmus_schedule minimize(ghostos_litmus_kind kind, ghostos_litmus_schedule schedule, ghostos_litmus_fault fault) {
    size_t index = 0;
    while (index < schedule.len) {
        ghostos_litmus_schedule candidate = without_step(&schedule, index);
        if (execute(kind, &candidate, fault) != GHOSTOS_LITMUS_FAILURE_NONE) schedule = candidate;
        else ++index;
    }
    return schedule;
}

ghostos_litmus_case_report ghostos_litmus_replay_with_fault(ghostos_litmus_kind kind,
    const ghostos_litmus_schedule *schedule, ghostos_litmus_fault fault, uint64_t seed) {
    ghostos_litmus_case_report report = {0};
    report.kind = kind; report.seed = seed;
    if (schedule && schedule->len <= GHOSTOS_LITMUS_MAX_SCHEDULE_STEPS) report.interleaving = *schedule;
    report.failure = execute(kind, schedule, fault);
    report.has_failure = report.failure != GHOSTOS_LITMUS_FAILURE_NONE;
    if (report.has_failure) {
        report.minimal_failing_schedule = minimize(kind, *schedule, fault);
        report.has_minimal_failing_schedule = true;
    }
    return report;
}

static ghostos_litmus_schedule generated_schedule(ghostos_litmus_kind kind, uint64_t seed) {
    machine m = new_machine(kind);
    prng random = {seed ^ ((uint64_t)(kind + 1) * UINT64_C(0x9e3779b9))};
    random.state |= 1;
    ghostos_litmus_schedule schedule = ghostos_litmus_schedule_empty();
    while (schedule.len < GHOSTOS_LITMUS_MAX_SCHEDULE_STEPS) {
        uint8_t actors[3], count = 0;
        for (uint8_t actor = 0; actor < 3; ++actor) if (enabled(&m, actor)) actors[count++] = actor;
        if (!count) break;
        uint8_t actor = actors[next_random(&random) % count];
        (void)ghostos_litmus_schedule_push(&schedule, actor);
        (void)step(&m, actor, GHOSTOS_LITMUS_FAULT_NONE);
    }
    return schedule;
}

ghostos_litmus_case_report ghostos_litmus_run_case(ghostos_litmus_kind kind, uint64_t seed) {
    ghostos_litmus_schedule schedule = valid_kind(kind) ? generated_schedule(kind, seed) : ghostos_litmus_schedule_empty();
    return ghostos_litmus_replay_with_fault(kind, &schedule, GHOSTOS_LITMUS_FAULT_NONE, seed);
}

ghostos_litmus_suite_report ghostos_litmus_run_suite(uint64_t seed) {
    ghostos_litmus_suite_report suite = {0};
    suite.seed = seed;
    for (unsigned i = 0; i < GHOSTOS_LITMUS_COUNT; ++i) suite.cases[i] = ghostos_litmus_run_case((ghostos_litmus_kind)i, seed);
    return suite;
}
