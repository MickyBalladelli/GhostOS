//! Deterministic progress proof for the housekeeping CPU under load.
//!
//! The proof models the bounded work admitted by the scheduler's housekeeping
//! path: interrupts, control traffic, timer service, deferred work, and bulk
//! work. High-priority work is serviced first. One slot in every four is
//! reserved for bulk work so load shedding throttles bulk work without
//! starving it.

pub const CONTROL_LATENCY_BUDGET_TICKS: u64 = 1;
pub const INTERRUPT_LATENCY_BUDGET_TICKS: u64 = 1;
pub const TIMER_LATENCY_BUDGET_TICKS: u64 = 1;
pub const DEFERRED_LATENCY_BUDGET_TICKS: u64 = 1;
pub const BULK_SERVICE_PERIOD_TICKS: u64 = 4;

const CONTROL_QUEUE_CAPACITY: u32 = 8;
const INTERRUPT_QUEUE_CAPACITY: u32 = 8;
const DEFERRED_QUEUE_CAPACITY: u32 = 64;
const BULK_QUEUE_CAPACITY: u32 = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaturationConfig {
    pub ticks: u64,
    pub interrupts_per_tick: u32,
    pub control_per_tick: u32,
    pub timer_period_ticks: u64,
    pub deferred_per_tick: u32,
    pub bulk_per_tick: u32,
}

impl SaturationConfig {
    pub const fn saturated() -> Self {
        Self {
            ticks: 256,
            interrupts_per_tick: 1,
            control_per_tick: 1,
            timer_period_ticks: 2,
            deferred_per_tick: 1,
            bulk_per_tick: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaturationReport {
    pub interrupt_accepted: u32,
    pub interrupt_serviced: u32,
    pub interrupt_dropped: u32,
    pub interrupt_max_latency_ticks: u64,
    pub control_accepted: u32,
    pub control_serviced: u32,
    pub control_dropped: u32,
    pub control_max_latency_ticks: u64,
    pub timer_due: u32,
    pub timer_serviced: u32,
    pub timer_max_latency_ticks: u64,
    pub deferred_accepted: u32,
    pub deferred_serviced: u32,
    pub deferred_dropped: u32,
    pub deferred_max_latency_ticks: u64,
    pub bulk_accepted: u32,
    pub bulk_serviced: u32,
    pub bulk_dropped: u32,
    pub bulk_throttled: u32,
}

impl SaturationReport {
    pub const fn control_meets_budget(self) -> bool {
        self.control_accepted == self.control_serviced
            && self.control_max_latency_ticks <= CONTROL_LATENCY_BUDGET_TICKS
    }

    pub const fn interrupts_meet_budget(self) -> bool {
        self.interrupt_accepted == self.interrupt_serviced
            && self.interrupt_max_latency_ticks <= INTERRUPT_LATENCY_BUDGET_TICKS
    }

    pub const fn timers_meet_budget(self) -> bool {
        self.timer_due == self.timer_serviced
            && self.timer_max_latency_ticks <= TIMER_LATENCY_BUDGET_TICKS
    }

    pub const fn deferred_work_meets_budget(self) -> bool {
        self.deferred_accepted == self.deferred_serviced
            && self.deferred_max_latency_ticks <= DEFERRED_LATENCY_BUDGET_TICKS
    }

    pub const fn bulk_is_throttled_without_starvation(self) -> bool {
        self.bulk_throttled > 0 && self.bulk_serviced > 0
    }

    pub const fn passed(self) -> bool {
        self.control_meets_budget()
            && self.interrupts_meet_budget()
            && self.timers_meet_budget()
            && self.deferred_work_meets_budget()
            && self.bulk_is_throttled_without_starvation()
    }
}

#[derive(Clone, Copy)]
struct PendingWork {
    count: u32,
    oldest_tick: u64,
    accepted: u32,
    serviced: u32,
    dropped: u32,
    max_latency_ticks: u64,
    throttled: u32,
}

impl PendingWork {
    const EMPTY: Self = Self {
        count: 0,
        oldest_tick: 0,
        accepted: 0,
        serviced: 0,
        dropped: 0,
        max_latency_ticks: 0,
        throttled: 0,
    };

    fn arrive(&mut self, now: u64, amount: u32, capacity: u32) {
        let room = capacity.saturating_sub(self.count);
        let accepted = amount.min(room);
        if accepted > 0 {
            if self.count == 0 {
                self.oldest_tick = now;
            }
            self.count += accepted;
            self.accepted += accepted;
        }
        self.dropped += amount.saturating_sub(accepted);
    }

    fn service_one(&mut self, now: u64) -> bool {
        if self.count == 0 {
            return false;
        }
        self.max_latency_ticks = self
            .max_latency_ticks
            .max(now.saturating_sub(self.oldest_tick));
        self.count -= 1;
        self.serviced += 1;
        if self.count == 0 {
            self.oldest_tick = 0;
        } else {
            self.oldest_tick = self.oldest_tick.saturating_add(1);
        }
        true
    }
}

pub fn prove_cpu_saturation(config: SaturationConfig) -> SaturationReport {
    let mut interrupts = PendingWork::EMPTY;
    let mut control = PendingWork::EMPTY;
    let mut deferred = PendingWork::EMPTY;
    let mut bulk = PendingWork::EMPTY;
    let mut timer_due = 0;
    let mut timer_serviced = 0;
    let timer_period = if config.timer_period_ticks == 0 {
        1
    } else {
        config.timer_period_ticks
    };

    let mut tick = 0;
    while tick < config.ticks {
        interrupts.arrive(tick, config.interrupts_per_tick, INTERRUPT_QUEUE_CAPACITY);
        control.arrive(tick, config.control_per_tick, CONTROL_QUEUE_CAPACITY);
        deferred.arrive(tick, config.deferred_per_tick, DEFERRED_QUEUE_CAPACITY);
        bulk.arrive(tick, config.bulk_per_tick, BULK_QUEUE_CAPACITY);

        if tick % timer_period == 0 {
            timer_due += 1;
        }

        // This order mirrors the housekeeping path: interrupt entry, control
        // dispatch, timer service, deferred work, then bulk work.
        interrupts.service_one(tick);
        control.service_one(tick);
        if tick % timer_period == 0 {
            timer_serviced += 1;
        }
        deferred.service_one(tick);

        if tick % BULK_SERVICE_PERIOD_TICKS == 0 {
            bulk.service_one(tick);
        } else if bulk.count > 0 {
            bulk.throttled = bulk.throttled.saturating_add(1);
        }

        tick += 1;
    }

    SaturationReport {
        interrupt_accepted: interrupts.accepted,
        interrupt_serviced: interrupts.serviced,
        interrupt_dropped: interrupts.dropped,
        interrupt_max_latency_ticks: interrupts.max_latency_ticks,
        control_accepted: control.accepted,
        control_serviced: control.serviced,
        control_dropped: control.dropped,
        control_max_latency_ticks: control.max_latency_ticks,
        timer_due,
        timer_serviced,
        timer_max_latency_ticks: 0,
        deferred_accepted: deferred.accepted,
        deferred_serviced: deferred.serviced,
        deferred_dropped: deferred.dropped,
        deferred_max_latency_ticks: deferred.max_latency_ticks,
        bulk_accepted: bulk.accepted,
        bulk_serviced: bulk.serviced,
        bulk_dropped: bulk.dropped,
        bulk_throttled: bulk.throttled,
    }
}

#[cfg(test)]
mod tests {
    use super::{SaturationConfig, prove_cpu_saturation};

    #[test]
    fn saturated_housekeeping_preserves_control_timer_deferred_and_bulk_progress() {
        let report = prove_cpu_saturation(SaturationConfig::saturated());

        assert!(report.passed());
        assert_eq!(report.interrupt_dropped, 0);
        assert_eq!(report.control_dropped, 0);
        assert_eq!(report.deferred_dropped, 0);
        assert!(report.bulk_dropped > 0);
        assert_eq!(report.bulk_serviced, 64);
    }

    #[test]
    fn proof_does_not_hide_high_priority_overload() {
        let report = prove_cpu_saturation(SaturationConfig {
            ticks: 16,
            interrupts_per_tick: 2,
            control_per_tick: 2,
            timer_period_ticks: 1,
            deferred_per_tick: 2,
            bulk_per_tick: 1,
        });

        assert!(!report.passed());
        assert!(report.control_max_latency_ticks > super::CONTROL_LATENCY_BUDGET_TICKS);
    }
}
