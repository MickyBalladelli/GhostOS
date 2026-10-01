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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
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
#[repr(C)]
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

const _: [(); 32] = [(); core::mem::size_of::<SaturationConfig>()];
const _: [(); 104] = [(); core::mem::size_of::<SaturationReport>()];

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

unsafe extern "C" {
    fn ghostos_saturation_prove(config: *const SaturationConfig, report: *mut SaturationReport);
}

pub fn prove_cpu_saturation(config: SaturationConfig) -> SaturationReport {
    let mut report = SaturationReport {
        interrupt_accepted: 0,
        interrupt_serviced: 0,
        interrupt_dropped: 0,
        interrupt_max_latency_ticks: 0,
        control_accepted: 0,
        control_serviced: 0,
        control_dropped: 0,
        control_max_latency_ticks: 0,
        timer_due: 0,
        timer_serviced: 0,
        timer_max_latency_ticks: 0,
        deferred_accepted: 0,
        deferred_serviced: 0,
        deferred_dropped: 0,
        deferred_max_latency_ticks: 0,
        bulk_accepted: 0,
        bulk_serviced: 0,
        bulk_dropped: 0,
        bulk_throttled: 0,
    };
    unsafe { ghostos_saturation_prove(&config, &mut report) }
    report
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
