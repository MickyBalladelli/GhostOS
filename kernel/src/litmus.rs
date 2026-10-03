//! Deterministic, allocation-free kernel litmus models.
//!
//! Each model is driven by an explicit actor schedule. The schedule is
//! generated from a seed, can be replayed exactly, and is reduced by removing
//! steps until the failure no longer reproduces. The models describe the
//! ordering contracts at the kernel boundaries; they do not create host
//! threads or depend on host timing.

pub const MAX_SCHEDULE_STEPS: usize = 32;
pub const LITMUS_COUNT: usize = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum LitmusKind {
    IpcOrdering = 0,
    CapabilityRevocation = 1,
    MemoryVisibility = 2,
    InterruptRace = 3,
    SchedulerPreemption = 4,
    WakeupAfterHlt = 5,
}

impl LitmusKind {
    pub const ALL: [Self; LITMUS_COUNT] = [
        Self::IpcOrdering,
        Self::CapabilityRevocation,
        Self::MemoryVisibility,
        Self::InterruptRace,
        Self::SchedulerPreemption,
        Self::WakeupAfterHlt,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Failure {
    IpcReordered = 0,
    RevokedCapabilityUsed = 1,
    PublishedDataNotVisible = 2,
    InterruptLost = 3,
    PreemptionMissed = 4,
    WakeupLost = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Schedule {
    pub steps: [u8; MAX_SCHEDULE_STEPS],
    pub len: u8,
}

impl Schedule {
    pub const EMPTY: Self = Self {
        steps: [0; MAX_SCHEDULE_STEPS],
        len: 0,
    };

    pub const fn len(self) -> usize {
        self.len as usize
    }

    pub fn push(&mut self, actor: u8) -> bool {
        if self.len() > MAX_SCHEDULE_STEPS {
            // Retain the public-field bounds panic before entering C.
            let _ = self.steps[self.len()];
        }
        unsafe { ghostos_litmus_schedule_push(self, actor) }
    }

}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaseReport {
    pub kind: LitmusKind,
    pub seed: u64,
    pub interleaving: Schedule,
    pub failure: Option<Failure>,
    pub minimal_failing_schedule: Option<Schedule>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SuiteReport {
    pub seed: u64,
    pub cases: [CaseReport; LITMUS_COUNT],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fault {
    None,
    IpcReorder,
    StaleCapabilityCache,
    PublishBeforeWrite,
    LostInterruptWakeup,
    MissedPreemption,
    LostHltWakeup,
}

pub fn run_case(kind: LitmusKind, seed: u64) -> CaseReport {
    unsafe { ghostos_litmus_run_case(kind as u32, seed) }.to_public(kind)
}

pub fn run_suite(seed: u64) -> SuiteReport {
    SuiteReport { seed, cases: LitmusKind::ALL.map(|kind| run_case(kind, seed)) }
}

pub fn replay_with_fault(kind: LitmusKind, schedule: Schedule, fault: Fault, seed: u64) -> CaseReport {
    let mut report = NativeReport::EMPTY;
    let fault = match fault {
        Fault::None => 0,
        Fault::IpcReorder => 1,
        Fault::StaleCapabilityCache => 2,
        Fault::PublishBeforeWrite => 3,
        Fault::LostInterruptWakeup => 4,
        Fault::MissedPreemption => 5,
        Fault::LostHltWakeup => 6,
    };
    if !unsafe { ghostos_litmus_replay_checked(kind as u32, &schedule, fault, seed, &mut report) } {
        panic!("index out of bounds: the len is {MAX_SCHEDULE_STEPS} but the index is {MAX_SCHEDULE_STEPS}")
    }
    report.to_public(kind)
}

#[repr(C)]
struct NativeReport {
    kind: u32,
    seed: u64,
    interleaving: Schedule,
    failure: u32,
    has_failure: bool,
    minimal_failing_schedule: Schedule,
    has_minimal_failing_schedule: bool,
}

impl NativeReport {
    const EMPTY: Self = Self {
        kind: 0, seed: 0, interleaving: Schedule::EMPTY, failure: 0, has_failure: false,
        minimal_failing_schedule: Schedule::EMPTY, has_minimal_failing_schedule: false,
    };

    fn to_public(self, kind: LitmusKind) -> CaseReport {
        let failure = self.has_failure.then(|| match self.failure {
            1 => Failure::IpcReordered,
            2 => Failure::RevokedCapabilityUsed,
            3 => Failure::PublishedDataNotVisible,
            4 => Failure::InterruptLost,
            5 => Failure::PreemptionMissed,
            6 => Failure::WakeupLost,
            _ => unreachable!("invalid native litmus failure"),
        });
        CaseReport {
            kind, seed: self.seed, interleaving: self.interleaving, failure,
            minimal_failing_schedule: self.has_minimal_failing_schedule.then_some(self.minimal_failing_schedule),
        }
    }
}

const _: () = {
    assert!(core::mem::size_of::<Schedule>() == 33);
    assert!(core::mem::size_of::<NativeReport>() == 96);
    assert!(core::mem::offset_of!(NativeReport, failure) == 52);
    assert!(core::mem::offset_of!(NativeReport, minimal_failing_schedule) == 57);
};

unsafe extern "C" {
    fn ghostos_litmus_schedule_push(schedule: *mut Schedule, actor: u8) -> bool;
    fn ghostos_litmus_run_case(kind: u32, seed: u64) -> NativeReport;
    fn ghostos_litmus_replay_checked(kind: u32, schedule: *const Schedule,
        fault: u32, seed: u64, report: *mut NativeReport) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule(steps: &[u8]) -> Schedule {
        let mut result = Schedule::EMPTY;
        for &step in steps {
            assert!(result.push(step));
        }
        result
    }

    #[test]
    fn same_seed_reproduces_every_interleaving() {
        let first = run_suite(0x59_13_00D);
        let second = run_suite(0x59_13_00D);

        assert_eq!(first, second);
        assert_eq!(first.seed, 0x59_13_00D);
        assert!(first.cases.iter().all(|case| case.failure.is_none()));
    }

    #[test]
    fn ipc_ordering_reports_a_minimal_reordering_schedule() {
        let report = replay_with_fault(
            LitmusKind::IpcOrdering,
            schedule(&[0, 0, 1, 1]),
            Fault::IpcReorder,
            1,
        );

        assert_eq!(report.failure, Some(Failure::IpcReordered));
        assert_eq!(report.minimal_failing_schedule, Some(schedule(&[0, 0, 1])));
    }

    #[test]
    fn capability_revocation_reports_stale_authorization() {
        let report = replay_with_fault(
            LitmusKind::CapabilityRevocation,
            schedule(&[0, 0, 0, 1]),
            Fault::StaleCapabilityCache,
            2,
        );

        assert_eq!(report.failure, Some(Failure::RevokedCapabilityUsed));
        assert_eq!(report.minimal_failing_schedule, Some(schedule(&[0, 0, 0, 1])));
    }

    #[test]
    fn memory_visibility_reports_publish_before_write() {
        let report = replay_with_fault(
            LitmusKind::MemoryVisibility,
            schedule(&[0, 1, 1]),
            Fault::PublishBeforeWrite,
            3,
        );

        assert_eq!(report.failure, Some(Failure::PublishedDataNotVisible));
        assert_eq!(report.minimal_failing_schedule, Some(schedule(&[0, 1, 1])));
    }

    #[test]
    fn interrupt_race_reports_a_lost_wakeup() {
        let report = replay_with_fault(
            LitmusKind::InterruptRace,
            schedule(&[1, 0, 1]),
            Fault::LostInterruptWakeup,
            4,
        );

        assert_eq!(report.failure, Some(Failure::InterruptLost));
        assert_eq!(report.minimal_failing_schedule, Some(schedule(&[1, 0, 1])));
    }

    #[test]
    fn scheduler_preemption_reports_a_missed_timer_switch() {
        let report = replay_with_fault(
            LitmusKind::SchedulerPreemption,
            schedule(&[0]),
            Fault::MissedPreemption,
            5,
        );

        assert_eq!(report.failure, Some(Failure::PreemptionMissed));
        assert_eq!(report.minimal_failing_schedule, Some(schedule(&[0])));
    }

    #[test]
    fn wakeup_after_hlt_reports_a_lost_interrupt_wakeup() {
        let report = replay_with_fault(
            LitmusKind::WakeupAfterHlt,
            schedule(&[0, 0, 1]),
            Fault::LostHltWakeup,
            6,
        );

        assert_eq!(report.failure, Some(Failure::WakeupLost));
        assert_eq!(report.minimal_failing_schedule, Some(schedule(&[0, 0, 1])));
    }
}
