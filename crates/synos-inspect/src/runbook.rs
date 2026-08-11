//! Bounded operator runbooks assembled from live diagnostic metadata.
//!
//! A runbook is publishable only when diagnosis, safe action, rollback, and
//! recovery proof links are all present. The generator does not execute an
//! operator action; it gives the shell or alert gateway one stable record to
//! display and audit.

use synos_heal::RecoveryStatus;
use synos_observability::{Alert, AlertLevel, AlertRegistry, HealthState, HealthTransport, OperationalHealth};
use synos_system_model::quota::{QuotaLedger, QuotaResource};
use synos_update::{ArtifactKind, CompatibilityContract, ReleaseBundle};

pub const DEFAULT_RUNBOOK_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RunbookSource {
    ServiceHealth = 1,
    Dependency = 2,
    Recovery = 3,
    Quota = 4,
    Compatibility = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceHealthMetadata {
    pub service: u64,
    pub transport: HealthTransport,
    pub state: HealthState,
    pub sampled_at_us: u64,
    pub degraded_mode: bool,
    pub queue_depth: u32,
    pub queue_capacity: u32,
    pub dropped_packets: u64,
    pub retries: u64,
}

impl ServiceHealthMetadata {
    pub const fn from_sample(sample: OperationalHealth) -> Self {
        Self {
            service: sample.node as u64,
            transport: sample.transport,
            state: sample.state,
            sampled_at_us: sample.sampled_at_us,
            degraded_mode: sample.degraded_mode,
            queue_depth: sample.queue_depth,
            queue_capacity: sample.queue_capacity,
            dropped_packets: sample.dropped_packets,
            retries: sample.retries,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DependencyMetadata {
    pub dependency: u64,
    pub required: bool,
    pub available: bool,
    pub timeout_us: u64,
    pub degraded_behavior: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryMetadata {
    pub service: u64,
    pub generation: u32,
    pub checkpoint_available: bool,
    pub recovery_deadline_us: u64,
    pub rollback_generation: u32,
}

impl RecoveryMetadata {
    pub const fn from_status(status: RecoveryStatus, recovery_deadline_us: u64) -> Self {
        Self {
            service: status.service,
            generation: status.generation,
            checkpoint_available: status.snapshot.is_some(),
            recovery_deadline_us,
            rollback_generation: status.generation,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuotaMetadata {
    pub resource: QuotaResource,
    pub consumed: u64,
    pub limit: u64,
    pub recovery_reserved: u64,
}

impl QuotaMetadata {
    pub const fn from_ledger(
        ledger: QuotaLedger,
        resource: QuotaResource,
        recovery_reserved: u64,
    ) -> Self {
        Self {
            resource,
            consumed: ledger.usage().consumed(resource),
            limit: ledger.policy().limit(resource),
            recovery_reserved,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibilityMetadata {
    pub artifact: ArtifactKind,
    pub current_version: u64,
    pub contract: CompatibilityContract,
    pub migration_id: u64,
    pub rollback_target: [u8; 32],
    pub audit_sequence: u64,
}

impl CompatibilityMetadata {
    pub fn from_bundle(bundle: ReleaseBundle, artifact: ArtifactKind) -> Option<Self> {
        bundle.validate().ok()?;
        let spec = bundle.artifacts.iter().find(|spec| spec.kind == artifact)?;
        Some(Self {
            artifact,
            current_version: spec.version,
            contract: spec.compatibility,
            migration_id: bundle.migration_id,
            rollback_target: bundle.rollback_target,
            audit_sequence: bundle.audit_sequence,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunbookLink {
    pub id: u32,
    pub label: &'static str,
    pub reference: &'static str,
}

impl RunbookLink {
    pub const fn new(id: u32, label: &'static str, reference: &'static str) -> Self {
        Self { id, label, reference }
    }

    const fn is_complete(self) -> bool {
        self.id != 0 && !self.label.is_empty() && !self.reference.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunbookMetadata {
    pub alert_code: u16,
    pub severity: AlertLevel,
    pub source: RunbookSource,
    pub health: ServiceHealthMetadata,
    pub dependency: DependencyMetadata,
    pub recovery: RecoveryMetadata,
    pub quota: QuotaMetadata,
    pub compatibility: CompatibilityMetadata,
    pub diagnosis: RunbookLink,
    pub safe_action: RunbookLink,
    pub rollback: RunbookLink,
    pub recovery_proof: RunbookLink,
}

impl RunbookMetadata {
    fn validate(self) -> Result<(), RunbookError> {
        if self.alert_code == 0 {
            return Err(RunbookError::InvalidAlertCode)
        }
        if self.health.service == 0 || self.health.sampled_at_us == 0 {
            return Err(RunbookError::InvalidHealth)
        }
        if self.dependency.dependency == 0 || self.dependency.timeout_us == 0 {
            return Err(RunbookError::InvalidDependency)
        }
        if self.dependency.degraded_behavior.is_empty() {
            return Err(RunbookError::MissingDegradedBehavior)
        }
        if self.recovery.service == 0 || self.recovery.recovery_deadline_us == 0 {
            return Err(RunbookError::InvalidRecovery)
        }
        if self.quota.limit == 0 || self.quota.consumed > self.quota.limit {
            return Err(RunbookError::InvalidQuota)
        }
        if self.compatibility.current_version == 0
            || self.compatibility.migration_id == 0
            || self.compatibility.rollback_target == [0; 32]
            || self.compatibility.audit_sequence == 0
            || self.compatibility.contract.minimum_peer_version
                > self.compatibility.contract.maximum_peer_version
            || !self
                .compatibility
                .contract
                .allows(self.compatibility.current_version)
        {
            return Err(RunbookError::InvalidCompatibility)
        }
        if !self.diagnosis.is_complete() {
            return Err(RunbookError::MissingDiagnosis)
        }
        if !self.safe_action.is_complete() {
            return Err(RunbookError::MissingSafeAction)
        }
        if !self.rollback.is_complete() {
            return Err(RunbookError::MissingRollback)
        }
        if !self.recovery_proof.is_complete() {
            return Err(RunbookError::MissingRecoveryProof)
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Runbook {
    pub alert: Alert,
    pub metadata: RunbookMetadata,
}

impl Runbook {
    pub const fn diagnosis(&self) -> RunbookLink {
        self.metadata.diagnosis
    }

    pub const fn safe_action(&self) -> RunbookLink {
        self.metadata.safe_action
    }

    pub const fn rollback(&self) -> RunbookLink {
        self.metadata.rollback
    }

    pub const fn recovery_proof(&self) -> RunbookLink {
        self.metadata.recovery_proof
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunbookError {
    Capacity,
    DuplicateAlertCode,
    InvalidAlertCode,
    InvalidHealth,
    InvalidDependency,
    MissingDegradedBehavior,
    InvalidRecovery,
    InvalidQuota,
    InvalidCompatibility,
    MissingDiagnosis,
    MissingSafeAction,
    MissingRollback,
    MissingRecoveryProof,
    AlertNotRegistered,
    OutputCapacity,
}

pub struct RunbookGenerator<const CAPACITY: usize = DEFAULT_RUNBOOK_CAPACITY> {
    entries: [Option<RunbookMetadata>; CAPACITY],
}

impl<const CAPACITY: usize> RunbookGenerator<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY > 0);
        Self { entries: [None; CAPACITY] }
    }

    pub fn register(&mut self, metadata: RunbookMetadata) -> Result<(), RunbookError> {
        metadata.validate()?;
        if self
            .entries
            .iter()
            .flatten()
            .any(|entry| entry.alert_code == metadata.alert_code)
        {
            return Err(RunbookError::DuplicateAlertCode)
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(RunbookError::Capacity)?;
        *slot = Some(metadata);
        Ok(())
    }

    pub fn for_alert(&self, alert: Alert) -> Result<Runbook, RunbookError> {
        let metadata = self
            .entries
            .iter()
            .flatten()
            .find(|entry| entry.alert_code == alert.code)
            .copied()
            .ok_or(RunbookError::AlertNotRegistered)?;
        Ok(Runbook { alert, metadata })
    }

    pub fn generate<const ALERT_CAPACITY: usize>(
        &self,
        alerts: &AlertRegistry<ALERT_CAPACITY>,
        destination: &mut [Option<Runbook>],
    ) -> Result<usize, RunbookError> {
        let count = alerts.alerts().count();
        if destination.len() < count {
            return Err(RunbookError::OutputCapacity)
        }
        for alert in alerts.alerts() {
            self.for_alert(alert)?;
        }
        for (index, alert) in alerts.alerts().enumerate() {
            destination[index] = Some(self.for_alert(alert)?);
        }
        Ok(count)
    }

    pub fn registered(&self) -> impl Iterator<Item = RunbookMetadata> + '_ {
        self.entries.iter().flatten().copied()
    }
}

impl<const CAPACITY: usize> Default for RunbookGenerator<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
