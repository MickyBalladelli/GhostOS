#![forbid(unsafe_code)]

//! Signed release rollout coordination.
//!
//! This module owns policy and ordering. Kernel patching, service hot-swap,
//! package activation, client routing, schema migration, and cluster fencing
//! remain behind [`RolloutRuntime`]. A runtime must report the integrity facts
//! needed to prove that a mixed-version rollout or rollback was safe.

use core::fmt;

pub const ARTIFACT_COUNT: usize = 6;
pub const DEFAULT_ROLLOUT_UNITS: u32 = 1;
pub const DEFAULT_HEALTH_DEADLINE_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u8)]
pub enum ArtifactKind {
    Kernel = 0,
    Service = 1,
    Package = 2,
    Client = 3,
    Schema = 4,
    ClusterProtocol = 5,
}

impl ArtifactKind {
    pub const ALL: [Self; ARTIFACT_COUNT] = [
        Self::Kernel,
        Self::Service,
        Self::Package,
        Self::Client,
        Self::Schema,
        Self::ClusterProtocol,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CompatibilityMode {
    ReadWrite = 1,
    ReadConvert = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibilityContract {
    pub minimum_peer_version: u64,
    pub maximum_peer_version: u64,
    pub mode: CompatibilityMode,
}

impl CompatibilityContract {
    pub const fn allows(self, peer_version: u64) -> bool {
        peer_version >= self.minimum_peer_version && peer_version <= self.maximum_peer_version
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactSpec {
    pub kind: ArtifactKind,
    pub version: u64,
    pub digest: [u8; 32],
    pub compatibility: CompatibilityContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReleaseBundle {
    pub release_id: [u8; 32],
    pub artifacts: [ArtifactSpec; ARTIFACT_COUNT],
    pub migration_id: u64,
    pub rollback_target: [u8; 32],
    pub capability_epoch: u64,
    pub audit_sequence: u64,
    pub signature: [u8; 64],
}

impl ReleaseBundle {
    pub fn validate(self) -> Result<(), BundleError> {
        if self.release_id == [0; 32]
            || self.rollback_target == [0; 32]
            || self.migration_id == 0
            || self.capability_epoch == 0
            || self.audit_sequence == 0
            || self.signature == [0; 64]
        {
            return Err(BundleError::MissingReleaseMetadata)
        }

        for index in 0..ARTIFACT_COUNT {
            let artifact = self.artifacts[index];
            if artifact.kind != ArtifactKind::ALL[index] {
                return Err(BundleError::ArtifactOrder)
            }
            if artifact.version == 0 || artifact.digest == [0; 32] {
                return Err(BundleError::InvalidArtifact)
            }
            if artifact.compatibility.minimum_peer_version
                > artifact.compatibility.maximum_peer_version
            {
                return Err(BundleError::InvalidCompatibility)
            }
        }
        Ok(())
    }

    pub fn check_mixed_version(self, running: Self) -> Result<CompatibilityReport, CompatibilityError> {
        self.validate().map_err(CompatibilityError::Bundle)?;
        running.validate().map_err(CompatibilityError::RunningBundle)?;
        for index in 0..ARTIFACT_COUNT {
            let target = self.artifacts[index];
            let current = running.artifacts[index];
            if !target.compatibility.allows(current.version) {
                return Err(CompatibilityError::PeerOutsideRange {
                    kind: target.kind,
                    peer_version: current.version,
                    minimum: target.compatibility.minimum_peer_version,
                    maximum: target.compatibility.maximum_peer_version,
                })
            }
        }
        Ok(CompatibilityReport {
            checked_artifacts: ARTIFACT_COUNT,
        })
    }
}

pub trait ReleaseVerifier {
    type Error;

    fn verify(&mut self, bundle: &ReleaseBundle) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibilityReport {
    pub checked_artifacts: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundleError {
    MissingReleaseMetadata,
    ArtifactOrder,
    InvalidArtifact,
    InvalidCompatibility,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilityError {
    Bundle(BundleError),
    RunningBundle(BundleError),
    PeerOutsideRange {
        kind: ArtifactKind,
        peer_version: u64,
        minimum: u64,
        maximum: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RolloutStrategy {
    Rolling = 1,
    Canary = 2,
    BlueGreen = 3,
    Emergency = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RolloutPlan {
    pub strategy: RolloutStrategy,
    pub total_units: u32,
    pub canary_units: u32,
    pub health_deadline_ms: u64,
}

impl RolloutPlan {
    pub const fn rolling(total_units: u32) -> Self {
        Self {
            strategy: RolloutStrategy::Rolling,
            total_units,
            canary_units: 0,
            health_deadline_ms: DEFAULT_HEALTH_DEADLINE_MS,
        }
    }

    pub const fn canary(total_units: u32, canary_units: u32) -> Self {
        Self {
            strategy: RolloutStrategy::Canary,
            total_units,
            canary_units,
            health_deadline_ms: DEFAULT_HEALTH_DEADLINE_MS,
        }
    }

    pub const fn blue_green() -> Self {
        Self {
            strategy: RolloutStrategy::BlueGreen,
            total_units: DEFAULT_ROLLOUT_UNITS,
            canary_units: 0,
            health_deadline_ms: DEFAULT_HEALTH_DEADLINE_MS,
        }
    }

    pub const fn emergency() -> Self {
        Self {
            strategy: RolloutStrategy::Emergency,
            total_units: DEFAULT_ROLLOUT_UNITS,
            canary_units: 0,
            health_deadline_ms: DEFAULT_HEALTH_DEADLINE_MS,
        }
    }

    pub const fn with_health_deadline(mut self, health_deadline_ms: u64) -> Self {
        self.health_deadline_ms = health_deadline_ms;
        self
    }

    pub fn validate<const MAX_UNITS: usize>(self) -> Result<(), PlanError> {
        if self.total_units == 0
            || self.total_units as usize > MAX_UNITS
            || self.health_deadline_ms == 0
        {
            return Err(PlanError::InvalidBounds)
        }
        if self.strategy == RolloutStrategy::Canary
            && (self.canary_units == 0 || self.canary_units >= self.total_units)
        {
            return Err(PlanError::InvalidCanary)
        }
        if self.strategy != RolloutStrategy::Canary && self.canary_units != 0 {
            return Err(PlanError::UnexpectedCanary)
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanError {
    InvalidBounds,
    InvalidCanary,
    UnexpectedCanary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RolloutTarget {
    Unit(u32),
    Cohort(u32),
    Blue,
    Green,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RolloutPhase {
    Planned,
    Staged,
    Activated,
    Switched,
    Drained,
    Completed,
    RolledBack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RolloutAuditEvent {
    pub release_id: [u8; 32],
    pub strategy: RolloutStrategy,
    pub phase: RolloutPhase,
    pub target: RolloutTarget,
    pub rollout_generation: u64,
    pub capability_epoch: u64,
    pub audit_sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HealthReport {
    pub observed_ms: u64,
    pub ready: bool,
    pub mixed_version_compatible: bool,
    pub data_continuity: bool,
    pub capability_continuity: bool,
    pub stale_capabilities_fenced: bool,
    pub audit_continuity: bool,
    pub duplicate_side_effects: bool,
}

impl HealthReport {
    pub const fn passes(self, deadline_ms: u64) -> bool {
        self.observed_ms <= deadline_ms
            && self.ready
            && self.mixed_version_compatible
            && self.data_continuity
            && self.capability_continuity
            && self.stale_capabilities_fenced
            && self.audit_continuity
            && !self.duplicate_side_effects
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RollbackReport {
    pub data_continuity: bool,
    pub capability_continuity: bool,
    pub stale_capabilities_fenced: bool,
    pub audit_continuity: bool,
    pub duplicate_side_effects: bool,
}

impl RollbackReport {
    pub const fn safe(self) -> bool {
        self.data_continuity
            && self.capability_continuity
            && self.stale_capabilities_fenced
            && self.audit_continuity
            && !self.duplicate_side_effects
    }
}

pub trait RolloutRuntime {
    type Error;

    fn stage(
        &mut self,
        bundle: &ReleaseBundle,
        target: RolloutTarget,
    ) -> Result<(), Self::Error>;

    fn activate(
        &mut self,
        bundle: &ReleaseBundle,
        target: RolloutTarget,
    ) -> Result<(), Self::Error>;

    fn switch_traffic(&mut self, target: RolloutTarget) -> Result<(), Self::Error>;

    fn drain(&mut self, target: RolloutTarget) -> Result<(), Self::Error>;

    fn health(&mut self, target: RolloutTarget) -> Result<HealthReport, Self::Error>;

    fn rollback(
        &mut self,
        running: &ReleaseBundle,
        rollback_target: [u8; 32],
        target: RolloutTarget,
        capability_epoch: u64,
    ) -> Result<RollbackReport, Self::Error>;

    fn record_audit(&mut self, event: RolloutAuditEvent) -> Result<(), Self::Error>;
}

#[derive(Debug)]
pub enum RolloutError<E, V> {
    InvalidPlan,
    Busy,
    Bundle(BundleError),
    Compatibility(CompatibilityError),
    Verification(V),
    Runtime(E),
    HealthFailed(HealthReport),
    RollbackIntegrity(RollbackReport),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RolloutState {
    Idle,
    Staging,
    MixedVersion,
    Switching,
    Completed,
    RollingBack,
    RolledBack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RolloutReceipt {
    pub strategy: RolloutStrategy,
    pub release_id: [u8; 32],
    pub previous_release_id: [u8; 32],
    pub rollout_generation: u64,
    pub capability_epoch: u64,
    pub audit_sequence: u64,
    pub rolled_back: bool,
}

pub struct RolloutCoordinator<const MAX_UNITS: usize = 64> {
    state: RolloutState,
    next_generation: u64,
}

impl<const MAX_UNITS: usize> RolloutCoordinator<MAX_UNITS> {
    pub const fn new() -> Self {
        Self {
            state: RolloutState::Idle,
            next_generation: 0,
        }
    }

    pub const fn state(&self) -> RolloutState {
        self.state
    }

    pub const fn next_generation(&self) -> u64 {
        self.next_generation
    }

    pub fn reset(&mut self) {
        self.state = RolloutState::Idle;
    }

    pub fn execute<R: RolloutRuntime, V: ReleaseVerifier>(
        &mut self,
        runtime: &mut R,
        verifier: &mut V,
        running: ReleaseBundle,
        bundle: ReleaseBundle,
        plan: RolloutPlan,
    ) -> Result<RolloutReceipt, RolloutError<R::Error, V::Error>> {
        if self.state != RolloutState::Idle {
            return Err(RolloutError::Busy)
        }
        plan.validate::<MAX_UNITS>().map_err(|_| RolloutError::InvalidPlan)?;
        bundle.validate().map_err(RolloutError::Bundle)?;
        verifier.verify(&bundle).map_err(RolloutError::Verification)?;
        bundle
            .check_mixed_version(running)
            .map_err(RolloutError::Compatibility)?;

        self.next_generation = self.next_generation.saturating_add(1).max(1);
        self.state = RolloutState::Staging;
        self.audit(runtime, &bundle, &plan, RolloutPhase::Planned, RolloutTarget::All)?;
        self.state = RolloutState::MixedVersion;

        let result = self.run_strategy::<R, V::Error>(runtime, &bundle, &plan);
        if let Err(error) = result {
            self.rollback_after_failure::<R, V::Error>(runtime, &running, &bundle, &plan)?;
            return Err(error)
        }

        if let Err(error) = self.audit::<R, V::Error>(
            runtime,
            &bundle,
            &plan,
            RolloutPhase::Completed,
            RolloutTarget::All,
        ) {
            self.rollback_after_failure::<R, V::Error>(runtime, &running, &bundle, &plan)?;
            return Err(error)
        }
        self.state = RolloutState::Completed;
        Ok(RolloutReceipt {
            strategy: plan.strategy,
            release_id: bundle.release_id,
            previous_release_id: running.release_id,
            rollout_generation: self.next_generation,
            capability_epoch: bundle.capability_epoch,
            audit_sequence: bundle.audit_sequence,
            rolled_back: false,
        })
    }

    fn run_strategy<R: RolloutRuntime, V>(
        &mut self,
        runtime: &mut R,
        bundle: &ReleaseBundle,
        plan: &RolloutPlan,
    ) -> Result<(), RolloutError<R::Error, V>> {
        match plan.strategy {
            RolloutStrategy::Rolling => {
                for unit in 0..plan.total_units {
                    self.stage_activate_health::<R, V>(runtime, bundle, plan, RolloutTarget::Unit(unit))?;
                    runtime
                        .drain(RolloutTarget::Unit(unit))
                        .map_err(RolloutError::Runtime)?;
                    self.audit_local::<R, V>(
                        runtime,
                        bundle,
                        plan,
                        RolloutPhase::Drained,
                        RolloutTarget::Unit(unit),
                    )?;
                }
            }
            RolloutStrategy::Canary => {
                let canary = RolloutTarget::Cohort(plan.canary_units);
                self.stage_activate_health::<R, V>(runtime, bundle, plan, canary)?;
                for unit in plan.canary_units..plan.total_units {
                    self.stage_activate_health::<R, V>(runtime, bundle, plan, RolloutTarget::Unit(unit))?;
                }
            }
            RolloutStrategy::BlueGreen => {
                self.stage_activate_health::<R, V>(runtime, bundle, plan, RolloutTarget::Green)?;
                self.state = RolloutState::Switching;
                runtime
                    .switch_traffic(RolloutTarget::Green)
                    .map_err(RolloutError::Runtime)?;
                self.audit_local::<R, V>(
                    runtime,
                    bundle,
                    plan,
                    RolloutPhase::Switched,
                    RolloutTarget::Green,
                )?;
                runtime
                    .drain(RolloutTarget::Blue)
                    .map_err(RolloutError::Runtime)?;
                self.audit_local::<R, V>(
                    runtime,
                    bundle,
                    plan,
                    RolloutPhase::Drained,
                    RolloutTarget::Blue,
                )?;
            }
            RolloutStrategy::Emergency => {
                self.stage_activate_health::<R, V>(runtime, bundle, plan, RolloutTarget::All)?;
            }
        }
        Ok(())
    }

    fn stage_activate_health<R: RolloutRuntime, V>(
        &mut self,
        runtime: &mut R,
        bundle: &ReleaseBundle,
        plan: &RolloutPlan,
        target: RolloutTarget,
    ) -> Result<(), RolloutError<R::Error, V>> {
        runtime
            .stage(bundle, target)
            .map_err(RolloutError::Runtime)?;
        self.audit_local::<R, V>(runtime, bundle, plan, RolloutPhase::Staged, target)?;
        runtime
            .activate(bundle, target)
            .map_err(RolloutError::Runtime)?;
        self.audit_local::<R, V>(runtime, bundle, plan, RolloutPhase::Activated, target)?;
        let health = runtime.health(target).map_err(RolloutError::Runtime)?;
        if !health.passes(plan.health_deadline_ms) {
            return Err(RolloutError::HealthFailed(health))
        }
        Ok(())
    }

    fn rollback_after_failure<R: RolloutRuntime, V>(
        &mut self,
        runtime: &mut R,
        running: &ReleaseBundle,
        bundle: &ReleaseBundle,
        plan: &RolloutPlan,
    ) -> Result<(), RolloutError<R::Error, V>> {
        self.state = RolloutState::RollingBack;
        let report = runtime
            .rollback(
                running,
                bundle.rollback_target,
                RolloutTarget::All,
                bundle.capability_epoch,
            )
            .map_err(RolloutError::Runtime)?;
        if !report.safe() {
            self.state = RolloutState::RolledBack;
            return Err(RolloutError::RollbackIntegrity(report))
        }
        self.audit_local::<R, V>(
            runtime,
            bundle,
            plan,
            RolloutPhase::RolledBack,
            RolloutTarget::All,
        )?;
        self.state = RolloutState::RolledBack;
        Ok(())
    }

    fn audit<R: RolloutRuntime, V>(
        &self,
        runtime: &mut R,
        bundle: &ReleaseBundle,
        plan: &RolloutPlan,
        phase: RolloutPhase,
        target: RolloutTarget,
    ) -> Result<(), RolloutError<R::Error, V>> {
        self.audit_local::<R, V>(runtime, bundle, plan, phase, target)
    }

    fn audit_local<R: RolloutRuntime, V>(
        &self,
        runtime: &mut R,
        bundle: &ReleaseBundle,
        plan: &RolloutPlan,
        phase: RolloutPhase,
        target: RolloutTarget,
    ) -> Result<(), RolloutError<R::Error, V>> {
        runtime
            .record_audit(RolloutAuditEvent {
                release_id: bundle.release_id,
                strategy: plan.strategy,
                phase,
                target,
                rollout_generation: self.next_generation,
                capability_epoch: bundle.capability_epoch,
                audit_sequence: bundle.audit_sequence,
            })
            .map_err(RolloutError::Runtime)
    }
}

impl<const MAX_UNITS: usize> Default for RolloutCoordinator<MAX_UNITS> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for BundleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MissingReleaseMetadata => "release metadata is incomplete",
            Self::ArtifactOrder => "release artifacts are not in canonical order",
            Self::InvalidArtifact => "release artifact is invalid",
            Self::InvalidCompatibility => "release compatibility range is invalid",
        };
        formatter.write_str(message)
    }
}
