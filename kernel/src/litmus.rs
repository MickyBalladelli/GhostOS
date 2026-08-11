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
        if self.len as usize == MAX_SCHEDULE_STEPS {
            return false
        }
        self.steps[self.len as usize] = actor;
        self.len += 1;
        true
    }

    fn without(self, removed: usize) -> Self {
        let mut result = Self::EMPTY;
        for index in 0..self.len() {
            if index != removed {
                let _ = result.push(self.steps[index]);
            }
        }
        result
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

#[derive(Clone, Copy)]
struct Prng(u64);

impl Prng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 7;
        value ^= value >> 9;
        value ^= value << 8;
        self.0 = value;
        value
    }
}

#[derive(Clone, Copy)]
struct IpcModel {
    producer_pc: u8,
    consumer_pc: u8,
    queue: [u8; 2],
    count: u8,
    received: [u8; 2],
}

#[derive(Clone, Copy)]
struct CapabilityModel {
    owner_pc: u8,
    borrower_pc: u8,
    root_exists: bool,
    child_exists: bool,
    revoked: bool,
    child_authorized: bool,
}

#[derive(Clone, Copy)]
struct MemoryModel {
    writer_pc: u8,
    reader_pc: u8,
    data: u64,
    published: bool,
    observed_published: bool,
}

#[derive(Clone, Copy)]
struct InterruptModel {
    cpu_pc: u8,
    device_pc: u8,
    armed: bool,
    sleeping: bool,
    pending: bool,
    raised: u8,
    handled: u8,
}

#[derive(Clone, Copy)]
struct SchedulerModel {
    timer_pc: u8,
    worker_pc: u8,
    current: u8,
    high_priority_ready: bool,
    preemptions: u8,
}

#[derive(Clone, Copy)]
struct HltModel {
    cpu_pc: u8,
    interrupt_pc: u8,
    armed: bool,
    sleeping: bool,
    event: bool,
}

#[derive(Clone, Copy)]
enum Machine {
    Ipc(IpcModel),
    Capability(CapabilityModel),
    Memory(MemoryModel),
    Interrupt(InterruptModel),
    Scheduler(SchedulerModel),
    Hlt(HltModel),
}

impl Machine {
    fn new(kind: LitmusKind) -> Self {
        match kind {
            LitmusKind::IpcOrdering => Self::Ipc(IpcModel {
                producer_pc: 0,
                consumer_pc: 0,
                queue: [0; 2],
                count: 0,
                received: [0; 2],
            }),
            LitmusKind::CapabilityRevocation => Self::Capability(CapabilityModel {
                owner_pc: 0,
                borrower_pc: 0,
                root_exists: false,
                child_exists: false,
                revoked: false,
                child_authorized: false,
            }),
            LitmusKind::MemoryVisibility => Self::Memory(MemoryModel {
                writer_pc: 0,
                reader_pc: 0,
                data: 0,
                published: false,
                observed_published: false,
            }),
            LitmusKind::InterruptRace => Self::Interrupt(InterruptModel {
                cpu_pc: 0,
                device_pc: 0,
                armed: false,
                sleeping: false,
                pending: false,
                raised: 0,
                handled: 0,
            }),
            LitmusKind::SchedulerPreemption => Self::Scheduler(SchedulerModel {
                timer_pc: 0,
                worker_pc: 0,
                current: 1,
                high_priority_ready: true,
                preemptions: 0,
            }),
            LitmusKind::WakeupAfterHlt => Self::Hlt(HltModel {
                cpu_pc: 0,
                interrupt_pc: 0,
                armed: false,
                sleeping: false,
                event: false,
            }),
        }
    }

    fn enabled(self, actor: u8) -> bool {
        match self {
            Self::Ipc(model) => match actor {
                0 => model.producer_pc < 2 && model.count < 2,
                1 => model.consumer_pc < 2 && model.count > 0,
                _ => false,
            },
            Self::Capability(model) => match actor {
                0 => model.owner_pc < 3,
                1 => model.borrower_pc < 1 && model.child_exists,
                _ => false,
            },
            Self::Memory(model) => match actor {
                0 => model.writer_pc < 2,
                1 => model.reader_pc < 2,
                _ => false,
            },
            Self::Interrupt(model) => match actor {
                0 => model.device_pc < 1,
                1 => model.cpu_pc < 2,
                _ => false,
            },
            Self::Scheduler(model) => match actor {
                0 => model.timer_pc < 1,
                1 => model.worker_pc < 1,
                _ => false,
            },
            Self::Hlt(model) => match actor {
                0 => model.cpu_pc < 2,
                1 => model.interrupt_pc < 1,
                _ => false,
            },
        }
    }

    fn step(&mut self, actor: u8, fault: Fault) -> Option<Failure> {
        match self {
            Self::Ipc(model) => {
                if actor == 0 {
                    if fault == Fault::IpcReorder && model.producer_pc == 1 {
                        model.queue[0] = 2;
                        model.queue[1] = 1;
                    } else {
                        model.queue[model.count as usize] = model.producer_pc + 1;
                    }
                    model.count += 1;
                    model.producer_pc += 1;
                } else {
                    let message = model.queue[0];
                    model.queue[0] = model.queue[1];
                    model.queue[1] = 0;
                    model.count -= 1;
                    model.received[model.consumer_pc as usize] = message;
                    model.consumer_pc += 1;
                    let expected = model.consumer_pc;
                    if message != expected {
                        return Some(Failure::IpcReordered)
                    }
                }
            }
            Self::Capability(model) => {
                if actor == 0 {
                    match model.owner_pc {
                        0 => model.root_exists = true,
                        1 => model.child_exists = model.root_exists,
                        2 => model.revoked = true,
                        _ => {}
                    }
                    model.owner_pc += 1;
                } else {
                    model.child_authorized = if fault == Fault::StaleCapabilityCache {
                        true
                    } else {
                        !model.revoked
                    };
                    model.borrower_pc += 1;
                    if model.revoked && model.child_authorized {
                        return Some(Failure::RevokedCapabilityUsed)
                    }
                }
            }
            Self::Memory(model) => {
                if actor == 0 {
                    if model.writer_pc == 0 {
                        if fault == Fault::PublishBeforeWrite {
                            model.published = true;
                        } else {
                            model.data = 42;
                        }
                    } else {
                        model.published = true;
                        if fault == Fault::PublishBeforeWrite {
                            model.data = 42;
                        }
                    }
                    model.writer_pc += 1;
                } else if model.reader_pc == 0 {
                    model.observed_published = model.published;
                    model.reader_pc += 1;
                } else {
                    model.reader_pc += 1;
                    if model.observed_published && model.data != 42 {
                        return Some(Failure::PublishedDataNotVisible)
                    }
                }
            }
            Self::Interrupt(model) => {
                if actor == 0 {
                    model.device_pc = 1;
                    model.raised += 1;
                    model.pending = true;
                    if model.armed && model.sleeping && fault != Fault::LostInterruptWakeup {
                        model.sleeping = false;
                    }
                } else if model.cpu_pc == 0 {
                    model.armed = true;
                    if model.pending {
                        model.pending = false;
                        model.handled += 1;
                    } else {
                        model.sleeping = true;
                    }
                    model.cpu_pc = 1;
                } else {
                    if !model.sleeping && model.pending {
                        model.pending = false;
                        model.handled += 1;
                    }
                    model.cpu_pc = 2;
                }
            }
            Self::Scheduler(model) => {
                if actor == 0 {
                    model.timer_pc = 1;
                    if model.high_priority_ready && fault != Fault::MissedPreemption {
                        model.current = 0;
                        model.preemptions += 1;
                    }
                    if model.high_priority_ready && model.current != 0 {
                        return Some(Failure::PreemptionMissed)
                    }
                } else {
                    model.worker_pc = 1;
                }
            }
            Self::Hlt(model) => {
                if actor == 0 {
                    if model.cpu_pc == 0 {
                        model.armed = true;
                        model.cpu_pc = 1;
                    } else {
                        if !model.event {
                            model.sleeping = true;
                        }
                        model.cpu_pc = 2;
                    }
                } else {
                    model.interrupt_pc = 1;
                    model.event = true;
                    if model.armed && model.sleeping && fault != Fault::LostHltWakeup {
                        model.sleeping = false;
                    }
                }
            }
        }
        self.failure()
    }

    fn failure(self) -> Option<Failure> {
        match self {
            Self::Interrupt(model) if model.raised != model.handled && model.cpu_pc == 2 => {
                Some(Failure::InterruptLost)
            }
            Self::Hlt(model) if model.event && model.sleeping && model.cpu_pc == 2 => {
                Some(Failure::WakeupLost)
            }
            _ => None,
        }
    }
}

fn generated_schedule(kind: LitmusKind, seed: u64) -> Schedule {
    let mut machine = Machine::new(kind);
    let mut random = Prng::new(seed ^ ((kind as u64 + 1) * 0x9e37_79b9));
    let mut schedule = Schedule::EMPTY;
    while schedule.len() < MAX_SCHEDULE_STEPS {
        let mut enabled = [0_u8; 3];
        let mut count = 0;
        for actor in 0..3 {
            if machine.enabled(actor) {
                enabled[count] = actor;
                count += 1;
            }
        }
        if count == 0 {
            break
        }
        let actor = enabled[(random.next() as usize) % count as usize];
        let _ = schedule.push(actor);
        let _ = machine.step(actor, Fault::None);
    }
    schedule
}

fn execute(kind: LitmusKind, schedule: Schedule, fault: Fault) -> Option<Failure> {
    let mut machine = Machine::new(kind);
    for index in 0..schedule.len() {
        let actor = schedule.steps[index];
        if !machine.enabled(actor) {
            return None
        }
        if let Some(failure) = machine.step(actor, fault) {
            return Some(failure)
        }
    }
    machine.failure()
}

fn minimize(kind: LitmusKind, mut schedule: Schedule, fault: Fault) -> Schedule {
    let mut index = 0;
    while index < schedule.len() {
        let candidate = schedule.without(index);
        if execute(kind, candidate, fault).is_some() {
            schedule = candidate;
        } else {
            index += 1;
        }
    }
    schedule
}

pub fn run_case(kind: LitmusKind, seed: u64) -> CaseReport {
    let schedule = generated_schedule(kind, seed);
    let failure = execute(kind, schedule, Fault::None);
    CaseReport {
        kind,
        seed,
        interleaving: schedule,
        failure,
        minimal_failing_schedule: failure.map(|_| minimize(kind, schedule, Fault::None)),
    }
}

pub fn run_suite(seed: u64) -> SuiteReport {
    SuiteReport {
        seed,
        cases: [
            run_case(LitmusKind::IpcOrdering, seed),
            run_case(LitmusKind::CapabilityRevocation, seed),
            run_case(LitmusKind::MemoryVisibility, seed),
            run_case(LitmusKind::InterruptRace, seed),
            run_case(LitmusKind::SchedulerPreemption, seed),
            run_case(LitmusKind::WakeupAfterHlt, seed),
        ],
    }
}

pub fn replay_with_fault(
    kind: LitmusKind,
    schedule: Schedule,
    fault: Fault,
    seed: u64,
) -> CaseReport {
    let failure = execute(kind, schedule, fault);
    CaseReport {
        kind,
        seed,
        interleaving: schedule,
        failure,
        minimal_failing_schedule: failure.map(|_| minimize(kind, schedule, fault)),
    }
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
