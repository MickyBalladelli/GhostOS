#![no_std]

//! Fixed-capacity admission for control-plane work.
//!
//! Recovery work owns a reserved part of the active budget. Optional work may
//! wait while the budget is busy, but it is dropped when the bounded wait
//! queue is full. Every decision increments a per-class counter and carries a
//! reason suitable for redacted operator diagnostics.

mod native;

pub const ADMISSION_FORMAT_VERSION: u16 = 1;
pub const WORK_CLASS_COUNT: usize = 6;
pub const DEFAULT_ACTIVE_CAPACITY: usize = 16;
pub const DEFAULT_QUEUE_CAPACITY: u16 = 32;
pub const DEFAULT_RECOVERY_RESERVE: u16 = 4;
pub const MAX_TENANT_POLICIES: usize = 32;

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
    TenantLimit = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TenantAdmissionPolicy {
    pub tenant: u64,
    pub parent: Option<u64>,
    pub active_limit: u16,
    pub recovery_reserve: u16,
}

impl TenantAdmissionPolicy {
    pub const fn new(
        tenant: u64,
        parent: Option<u64>,
        active_limit: u16,
        recovery_reserve: u16,
    ) -> Self {
        Self {
            tenant,
            parent,
            active_limit,
            recovery_reserve,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
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
#[repr(C)]
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
#[repr(C)]
pub struct AdmissionLease {
    slot: u16,
    sequence: u64,
    class: WorkClass,
    priority: AdmissionPriority,
    tenant: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    InvalidPolicy,
    InvalidLease,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
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
    state: native::State,
    leases: [native::Slot; CAPACITY],
}

impl<const CAPACITY: usize> AdmissionController<CAPACITY> {
    pub fn new(policy: AdmissionPolicy) -> Result<Self, AdmissionError> {
        let (state, leases) = native::initialize::<CAPACITY>(policy)?;
        Ok(Self { state, leases })
    }

    pub fn policy(&self) -> AdmissionPolicy {
        self.state.policy
    }

    pub fn report(&self) -> AdmissionReport {
        native::report(&self.state)
    }

    /// Configure one tenant's active and recovery budgets. Parent usage is
    /// charged together with child usage, forming a bounded hierarchy.
    pub fn configure_tenant(&mut self, policy: TenantAdmissionPolicy) -> Result<(), AdmissionError> {
        native::configure_tenant(&mut self.state, CAPACITY, policy)
    }

    /// Admit one operation or record the exact bounded shedding decision.
    pub fn admit(&mut self, class: WorkClass, priority: AdmissionPriority) -> AdmissionOutcome {
        self.admit_for(0, class, priority)
    }

    /// Admit work for a tenant, charging each configured ancestor.
    pub fn admit_for(&mut self, tenant: u64, class: WorkClass, priority: AdmissionPriority) -> AdmissionOutcome {
        native::admit(&mut self.state, &mut self.leases, tenant, class, priority)
    }

    /// Record a retry after a caller's transport or dependency failed.
    pub fn record_retry(&mut self, class: WorkClass, priority: AdmissionPriority) -> AdmissionOutcome {
        native::record_retry(&mut self.state, class, priority)
    }

    pub fn finish(&mut self, lease: AdmissionLease) -> Result<(), AdmissionError> {
        native::finish(&mut self.state, &mut self.leases, lease)
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

    #[test]
    fn tenant_children_share_parent_recovery_reserve() {
        let policy = AdmissionPolicy::new(4, 2, 0, [4; WORK_CLASS_COUNT]);
        let mut controller = AdmissionController::<4>::new(policy).unwrap();
        controller
            .configure_tenant(TenantAdmissionPolicy::new(100, None, 2, 1))
            .unwrap();
        controller
            .configure_tenant(TenantAdmissionPolicy::new(1, Some(100), 2, 0))
            .unwrap();
        controller
            .configure_tenant(TenantAdmissionPolicy::new(2, Some(100), 2, 0))
            .unwrap();

        assert!(controller
            .admit_for(1, WorkClass::Backup, AdmissionPriority::Normal)
            .admitted());
        assert_eq!(
            controller
                .admit_for(2, WorkClass::Backup, AdmissionPriority::Normal)
                .reason,
            AdmissionReason::TenantLimit
        );
        assert!(controller
            .admit_for(2, WorkClass::Snapshot, AdmissionPriority::Recovery)
            .admitted());
    }
}
