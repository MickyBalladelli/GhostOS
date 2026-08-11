#![no_std]
#![forbid(unsafe_code)]

//! Fixed-capacity admission for control-plane work.
//!
//! Recovery work owns a reserved part of the active budget. Optional work may
//! wait while the budget is busy, but it is dropped when the bounded wait
//! queue is full. Every decision increments a per-class counter and carries a
//! reason suitable for redacted operator diagnostics.

pub const ADMISSION_FORMAT_VERSION: u16 = 1;
pub const WORK_CLASS_COUNT: usize = 6;
pub const DEFAULT_ACTIVE_CAPACITY: usize = 16;
pub const DEFAULT_QUEUE_CAPACITY: u16 = 32;
pub const DEFAULT_RECOVERY_RESERVE: u16 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum WorkClass {
    ControlPlaneFanout = 0,
    MembershipChange = 1,
    Snapshot = 2,
    Backup = 3,
    PackageDistribution = 4,
    RemoteDiagnostics = 5,
}

impl WorkClass {
    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::ControlPlaneFanout => "control-plane-fanout",
            Self::MembershipChange => "membership-change",
            Self::Snapshot => "snapshot",
            Self::Backup => "backup",
            Self::PackageDistribution => "package-distribution",
            Self::RemoteDiagnostics => "remote-diagnostics",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u8)]
pub enum AdmissionPriority {
    Optional = 0,
    Normal = 1,
    Critical = 2,
    Recovery = 3,
}

impl AdmissionPriority {
    pub const fn is_recovery(self) -> bool {
        matches!(self, Self::Recovery)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AdmissionAction {
    Admitted = 1,
    Delayed = 2,
    Dropped = 3,
    Retried = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AdmissionReason {
    None = 0,
    ActiveCapacity = 1,
    RecoveryReserve = 2,
    ClassLimit = 3,
    QueueFull = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionPolicy {
    pub active_capacity: u16,
    pub queue_capacity: u16,
    pub recovery_reserve: u16,
    pub class_limits: [u16; WORK_CLASS_COUNT],
}

impl AdmissionPolicy {
    pub const fn new(
        active_capacity: u16,
        queue_capacity: u16,
        recovery_reserve: u16,
        class_limits: [u16; WORK_CLASS_COUNT],
    ) -> Self {
        Self {
            active_capacity,
            queue_capacity,
            recovery_reserve,
            class_limits,
        }
    }

    pub const fn default_policy() -> Self {
        Self::new(
            DEFAULT_ACTIVE_CAPACITY as u16,
            DEFAULT_QUEUE_CAPACITY,
            DEFAULT_RECOVERY_RESERVE,
            [DEFAULT_ACTIVE_CAPACITY as u16; WORK_CLASS_COUNT],
        )
    }

    pub const fn class_limit(self, class: WorkClass) -> u16 {
        self.class_limits[class.index()]
    }
}

impl Default for AdmissionPolicy {
    fn default() -> Self {
        Self::default_policy()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct AdmissionStats {
    pub admitted: u32,
    pub delayed: u32,
    pub dropped: u32,
    pub retried: u32,
    pub completed: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionOutcome {
    pub version: u16,
    pub sequence: u64,
    pub class: WorkClass,
    pub priority: AdmissionPriority,
    pub action: AdmissionAction,
    pub reason: AdmissionReason,
    pub active: u16,
    pub queued: u16,
    lease: Option<AdmissionLease>,
}

impl AdmissionOutcome {
    pub const fn admitted(self) -> bool {
        matches!(self.action, AdmissionAction::Admitted)
    }

    pub const fn lease(self) -> Option<AdmissionLease> {
        self.lease
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionLease {
    slot: u16,
    sequence: u64,
    class: WorkClass,
    priority: AdmissionPriority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    InvalidPolicy,
    InvalidLease,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionReport {
    pub version: u16,
    pub policy: AdmissionPolicy,
    pub active: u16,
    pub recovery_active: u16,
    pub queued: u16,
    pub classes: [AdmissionStats; WORK_CLASS_COUNT],
}

impl AdmissionReport {
    pub const fn class(self, class: WorkClass) -> AdmissionStats {
        self.classes[class.index()]
    }
}

pub struct AdmissionController<const CAPACITY: usize = DEFAULT_ACTIVE_CAPACITY> {
    policy: AdmissionPolicy,
    leases: [Option<AdmissionLease>; CAPACITY],
    active_by_class: [u16; WORK_CLASS_COUNT],
    stats: [AdmissionStats; WORK_CLASS_COUNT],
    active: u16,
    recovery_active: u16,
    queued: u16,
    next_sequence: u64,
}

impl<const CAPACITY: usize> AdmissionController<CAPACITY> {
    pub fn new(policy: AdmissionPolicy) -> Result<Self, AdmissionError> {
        if CAPACITY == 0
            || policy.active_capacity == 0
            || policy.active_capacity as usize > CAPACITY
            || policy.recovery_reserve > policy.active_capacity
            || policy.class_limits.iter().any(|limit| *limit == 0)
        {
            return Err(AdmissionError::InvalidPolicy)
        }
        Ok(Self {
            policy,
            leases: [None; CAPACITY],
            active_by_class: [0; WORK_CLASS_COUNT],
            stats: [AdmissionStats::default(); WORK_CLASS_COUNT],
            active: 0,
            recovery_active: 0,
            queued: 0,
            next_sequence: 1,
        })
    }

    pub fn policy(&self) -> AdmissionPolicy {
        self.policy
    }

    pub fn report(&self) -> AdmissionReport {
        AdmissionReport {
            version: ADMISSION_FORMAT_VERSION,
            policy: self.policy,
            active: self.active,
            recovery_active: self.recovery_active,
            queued: self.queued,
            classes: self.stats,
        }
    }

    /// Admit one operation or record the exact bounded shedding decision.
    pub fn admit(
        &mut self,
        class: WorkClass,
        priority: AdmissionPriority,
    ) -> AdmissionOutcome {
        let reason = self.block_reason(class, priority);
        if reason == AdmissionReason::None {
            if self.queued != 0 {
                self.queued -= 1;
            }
            let slot = self
                .leases
                .iter()
                .position(Option::is_none)
                .expect("admission policy capacity invariant");
            let sequence = self.next_sequence();
            let lease = AdmissionLease {
                slot: slot as u16,
                sequence,
                class,
                priority,
            };
            self.leases[slot] = Some(lease);
            self.active = self.active.saturating_add(1);
            self.active_by_class[class.index()] =
                self.active_by_class[class.index()].saturating_add(1);
            if priority.is_recovery() {
                self.recovery_active = self.recovery_active.saturating_add(1);
            }
            self.stats[class.index()].admitted = self.stats[class.index()].admitted.saturating_add(1);
            return self.outcome(
                sequence,
                class,
                priority,
                AdmissionAction::Admitted,
                reason,
                Some(lease),
            )
        }

        let sequence = self.next_sequence();
        let action = if self.queued < self.policy.queue_capacity {
            self.queued = self.queued.saturating_add(1);
            self.stats[class.index()].delayed = self.stats[class.index()].delayed.saturating_add(1);
            AdmissionAction::Delayed
        } else if priority == AdmissionPriority::Optional {
            self.stats[class.index()].dropped = self.stats[class.index()].dropped.saturating_add(1);
            AdmissionAction::Dropped
        } else {
            self.stats[class.index()].retried = self.stats[class.index()].retried.saturating_add(1);
            AdmissionAction::Retried
        };
        self.outcome(sequence, class, priority, action, reason, None)
    }

    /// Record a retry after a caller's transport or dependency failed.
    pub fn record_retry(
        &mut self,
        class: WorkClass,
        priority: AdmissionPriority,
    ) -> AdmissionOutcome {
        let sequence = self.next_sequence();
        self.stats[class.index()].retried = self.stats[class.index()].retried.saturating_add(1);
        self.outcome(
            sequence,
            class,
            priority,
            AdmissionAction::Retried,
            AdmissionReason::QueueFull,
            None,
        )
    }

    pub fn finish(&mut self, lease: AdmissionLease) -> Result<(), AdmissionError> {
        let Some(slot) = self.leases.get_mut(lease.slot as usize) else {
            return Err(AdmissionError::InvalidLease)
        };
        if slot.is_none_or(|current| current != lease) {
            return Err(AdmissionError::InvalidLease)
        }
        *slot = None;
        self.active = self.active.saturating_sub(1);
        self.active_by_class[lease.class.index()] =
            self.active_by_class[lease.class.index()].saturating_sub(1);
        if lease.priority.is_recovery() {
            self.recovery_active = self.recovery_active.saturating_sub(1);
        }
        self.stats[lease.class.index()].completed =
            self.stats[lease.class.index()].completed.saturating_add(1);
        Ok(())
    }

    fn block_reason(&self, class: WorkClass, priority: AdmissionPriority) -> AdmissionReason {
        if !priority.is_recovery() && self.active_by_class[class.index()] >= self.policy.class_limit(class) {
            return AdmissionReason::ClassLimit
        }
        if self.active >= self.policy.active_capacity {
            return AdmissionReason::ActiveCapacity
        }
        if !priority.is_recovery()
            && self.active.saturating_add(self.policy.recovery_reserve)
                >= self.policy.active_capacity
        {
            return AdmissionReason::RecoveryReserve
        }
        AdmissionReason::None
    }

    fn outcome(
        &self,
        sequence: u64,
        class: WorkClass,
        priority: AdmissionPriority,
        action: AdmissionAction,
        reason: AdmissionReason,
        lease: Option<AdmissionLease>,
    ) -> AdmissionOutcome {
        AdmissionOutcome {
            version: ADMISSION_FORMAT_VERSION,
            sequence,
            class,
            priority,
            action,
            reason,
            active: self.active,
            queued: self.queued,
            lease,
        }
    }

    fn next_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1).max(1);
        sequence
    }
}

impl<const CAPACITY: usize> Default for AdmissionController<CAPACITY> {
    fn default() -> Self {
        Self::new(AdmissionPolicy::default()).expect("default admission policy is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_uses_reserved_capacity_when_optional_work_is_blocked() {
        let policy = AdmissionPolicy::new(4, 1, 1, [4; WORK_CLASS_COUNT]);
        let mut controller = AdmissionController::<4>::new(policy).unwrap();
        for _ in 0..3 {
            assert_eq!(
                controller
                    .admit(WorkClass::Backup, AdmissionPriority::Normal)
                    .action,
                AdmissionAction::Admitted
            );
        }
        assert_eq!(
            controller
                .admit(WorkClass::RemoteDiagnostics, AdmissionPriority::Optional)
                .action,
            AdmissionAction::Delayed
        );
        assert_eq!(
            controller
                .admit(WorkClass::Snapshot, AdmissionPriority::Recovery)
                .action,
            AdmissionAction::Admitted
        );
        assert_eq!(controller.report().recovery_active, 1);
    }

    #[test]
    fn full_queue_reports_drop_and_retry_separately() {
        let policy = AdmissionPolicy::new(1, 1, 0, [1; WORK_CLASS_COUNT]);
        let mut controller = AdmissionController::<1>::new(policy).unwrap();
        let lease = match controller.admit(WorkClass::ControlPlaneFanout, AdmissionPriority::Critical) {
            outcome if outcome.admitted() => outcome.lease().unwrap(),
            _ => panic!("first request must be admitted"),
        };
        assert_eq!(
            controller
                .admit(WorkClass::RemoteDiagnostics, AdmissionPriority::Optional)
                .action,
            AdmissionAction::Delayed
        );
        assert_eq!(
            controller
                .admit(WorkClass::RemoteDiagnostics, AdmissionPriority::Optional)
                .action,
            AdmissionAction::Dropped
        );
        assert_eq!(
            controller
                .admit(WorkClass::MembershipChange, AdmissionPriority::Critical)
                .action,
            AdmissionAction::Retried
        );
        assert_eq!(controller.report().class(WorkClass::RemoteDiagnostics).dropped, 1);
        assert_eq!(controller.report().class(WorkClass::MembershipChange).retried, 1);
        controller.finish(lease).unwrap();
    }
}
