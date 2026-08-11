#![no_std]
#![forbid(unsafe_code)]

mod hotswap;
mod patch;
mod rollout;

pub use hotswap::{
    HotSwapCoordinator, HotSwapError, HotSwapReceipt, HotSwapRequest, HotSwapRuntime,
    ReplacementSpawnRequest, MAX_HOT_SWAP_DESCRIPTORS,
};
pub use patch::{
    DEFAULT_KERNEL_PATCH_CAPACITY, KernelPatchCoordinator, KernelPatchError,
    KernelPatchRecord, KernelPatchReceipt, KernelPatchRequest, KernelPatchRuntime,
    MicrokernelPatchCoordinator, MAX_KERNEL_PATCH_BATCH,
};
pub use rollout::{
    ArtifactKind, ArtifactSpec, CompatibilityContract, CompatibilityError, CompatibilityMode,
    CompatibilityReport, BundleError, HealthReport, ReleaseBundle, ReleaseVerifier,
    PlanError, RollbackReport, RolloutAuditEvent, RolloutCoordinator, RolloutError, RolloutPhase,
    RolloutPlan, RolloutReceipt, RolloutRuntime, RolloutState, RolloutStrategy, RolloutTarget,
    ARTIFACT_COUNT, DEFAULT_HEALTH_DEADLINE_MS, DEFAULT_ROLLOUT_UNITS,
};
pub use synos_ipc::InheritableDescriptor;

use synos_pkg::{PackageDaemon, PackageError, SystemConfiguration};
use synos_durability::{CrashBoundary, CrashDomain, InterruptionInjector, NoInterruption};
use synos_status::{IntoStatus, Status};
use synos_synfs::{CheckpointInfo, Error as SynFsError, SynFs};
use synos_system_model::RootManifest;
use synos_policy::{
    ObjectId, PolicyChange, PolicySnapshot, SimulationError, SimulationReport,
    UpdateRolloutChange,
};

pub const DEFAULT_UPDATE_HISTORY: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateError {
    AlreadyActive,
    HistoryFull,
    InvalidPlan,
    NoRollbackTarget,
    Package(PackageError),
    Rollback(SynFsError),
    Storage(SynFsError),
    HealthCheck(Status),
    Interrupted,
}

impl IntoStatus for UpdateError {
    fn status(self) -> Status {
        match self {
            Self::AlreadyActive | Self::InvalidPlan => Status::INVALID_ARGUMENT,
            Self::HistoryFull => Status::NO_SPACE,
            Self::NoRollbackTarget => Status::NOT_FOUND,
            Self::Package(error) => error.status(),
            Self::Rollback(error) | Self::Storage(error) => error.status(),
            Self::HealthCheck(status) => status,
            Self::Interrupted => Status::BUSY,
        }
    }
}

impl From<PackageError> for UpdateError {
    fn from(error: PackageError) -> Self {
        Self::Package(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpdatePlan {
    configuration: SystemConfiguration,
    expected_previous_revision: Option<u64>,
}

impl UpdatePlan {
    pub const fn new(configuration: SystemConfiguration) -> Self {
        Self {
            configuration,
            expected_previous_revision: None,
        }
    }

    pub const fn configuration(&self) -> &SystemConfiguration {
        &self.configuration
    }

    pub const fn revision(&self) -> u64 {
        self.configuration.revision()
    }

    pub const fn expected_previous_revision(&self) -> Option<u64> {
        self.expected_previous_revision
    }

    pub const fn expect_previous_revision(mut self, revision: u64) -> Self {
        self.expected_previous_revision = Some(revision);
        self
    }
}

/// A check runs after the new root is live and before its rollback checkpoint
/// is released. A failed check therefore returns the exact old root.
pub trait HealthCheck {
    fn check<const MAX_BLOCKS: usize>(
        &mut self,
        filesystem: &SynFs<MAX_BLOCKS>,
        configuration: &RootManifest,
    ) -> Result<(), Status>;
}

pub struct NoHealthCheck;

impl HealthCheck for NoHealthCheck {
    fn check<const MAX_BLOCKS: usize>(
        &mut self,
        _filesystem: &SynFs<MAX_BLOCKS>,
        _configuration: &RootManifest,
    ) -> Result<(), Status> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpdateRecord {
    pub revision: u64,
    pub previous_revision: Option<u64>,
    pub generation: u64,
    pub previous_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpdateReceipt {
    pub record: UpdateRecord,
    pub rolled_back: bool,
}

#[derive(Clone, Copy)]
struct HistoryEntry {
    checkpoint: CheckpointInfo,
    previous: Option<RootManifest>,
    record: UpdateRecord,
}

/// Applies complete declarative roots and keeps bounded, instant rollback
/// points. Packages are content-addressed by `synos-pkg`; this type only
/// changes the single active root pointer and its SynFS state.
pub struct UpdateManager<const HISTORY: usize = DEFAULT_UPDATE_HISTORY> {
    entries: [Option<HistoryEntry>; HISTORY],
}

impl<const HISTORY: usize> UpdateManager<HISTORY> {
    pub const fn new() -> Self {
        Self {
            entries: [None; HISTORY],
        }
    }

    /// Preview the objects and principals touched by a rollout. This method
    /// does not create a checkpoint or call package activation.
    pub fn simulate_rollout_policy<
        const PACKAGES: usize,
        const KEYS: usize,
        const PRINCIPALS: usize,
        const OBJECTS: usize,
        const BINDINGS: usize,
    >(
        &self,
        snapshot: &PolicySnapshot<PRINCIPALS, OBJECTS, BINDINGS>,
        packages: &PackageDaemon<PACKAGES, KEYS>,
        update: ObjectId,
        plan: UpdatePlan,
    ) -> Result<SimulationReport, SimulationError> {
        let from_revision = packages
            .active_configuration()
            .map_or(0, RootManifest::revision);
        snapshot.simulate(PolicyChange::UpdateRollout(UpdateRolloutChange {
            update,
            from_revision,
            to_revision: plan.revision(),
        }))
    }

    pub const fn len(&self) -> usize {
        let mut count = 0;
        while count < HISTORY {
            if self.entries[count].is_none() {
                return count;
            }
            count += 1;
        }
        count
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn history(&self) -> impl Iterator<Item = UpdateRecord> + '_ {
        self.entries
            .iter()
            .take(self.len())
            .flatten()
            .map(|entry| entry.record)
    }

    pub fn apply<const BLOCKS: usize, const PACKAGES: usize, const KEYS: usize, H: HealthCheck>(
        &mut self,
        filesystem: &mut SynFs<BLOCKS>,
        packages: &mut PackageDaemon<PACKAGES, KEYS>,
        plan: UpdatePlan,
        health_check: &mut H,
    ) -> Result<UpdateReceipt, UpdateError> {
        let mut no_interruption = NoInterruption;
        self.apply_with_interruption(
            filesystem,
            packages,
            plan,
            health_check,
            &mut no_interruption,
        )
    }

    pub fn apply_with_interruption<
        const BLOCKS: usize,
        const PACKAGES: usize,
        const KEYS: usize,
        H: HealthCheck,
        I: InterruptionInjector,
    >(
        &mut self,
        filesystem: &mut SynFs<BLOCKS>,
        packages: &mut PackageDaemon<PACKAGES, KEYS>,
        plan: UpdatePlan,
        health_check: &mut H,
        injector: &mut I,
    ) -> Result<UpdateReceipt, UpdateError> {
        if plan.revision() == 0 {
            return Err(UpdateError::InvalidPlan);
        }

        let previous = packages.active_configuration().copied();
        if plan.expected_previous_revision() != previous.map(|root| root.revision())
            && plan.expected_previous_revision().is_some()
        {
            return Err(UpdateError::AlreadyActive);
        }
        if previous.is_some_and(|root| root.revision() == plan.revision()) {
            return Err(UpdateError::AlreadyActive);
        }
        if HISTORY == 0 || self.len() == HISTORY {
            return Err(UpdateError::HistoryFull);
        }

        let checkpoint = filesystem
            .create_checkpoint()
            .map_err(UpdateError::Storage)?;
        let previous_generation = checkpoint.generation;
        let activation = packages.activate_with_interruption(
            filesystem,
            plan.configuration(),
            injector,
        );
        if let Err(error) = activation {
            let _ = filesystem.release_checkpoint(checkpoint.id);
            return Err(error.into());
        }

        let active = packages
            .active_configuration()
            .copied()
            .ok_or(UpdateError::InvalidPlan)?;
        if let Err(status) = health_check.check(filesystem, &active) {
            return self.abort_failed_update(filesystem, packages, checkpoint, previous, status);
        }

        let record = UpdateRecord {
            revision: active.revision(),
            previous_revision: previous.map(|root| root.revision()),
            generation: filesystem.generation(),
            previous_generation,
        };
        self.entries[self.len()] = Some(HistoryEntry {
            checkpoint,
            previous,
            record,
        });
        if injector.checkpoint(CrashDomain::UpdateRecovery, CrashBoundary::ManifestSlot) {
            return Err(UpdateError::Interrupted)
        }
        Ok(UpdateReceipt {
            record,
            rolled_back: false,
        })
    }

    pub fn rollback_last<const BLOCKS: usize, const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        filesystem: &mut SynFs<BLOCKS>,
        packages: &mut PackageDaemon<PACKAGES, KEYS>,
    ) -> Result<UpdateReceipt, UpdateError> {
        let index = self
            .len()
            .checked_sub(1)
            .ok_or(UpdateError::NoRollbackTarget)?;
        self.rollback_at(index, filesystem, packages)
    }

    /// Roll back to the state immediately before the update that introduced
    /// `revision`. Newer update records are discarded together.
    pub fn rollback_to<const BLOCKS: usize, const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        filesystem: &mut SynFs<BLOCKS>,
        packages: &mut PackageDaemon<PACKAGES, KEYS>,
        revision: u64,
    ) -> Result<UpdateReceipt, UpdateError> {
        let current = packages
            .active_configuration()
            .map(|root| root.revision())
            .ok_or(UpdateError::NoRollbackTarget)?;
        if current == revision {
            return Err(UpdateError::AlreadyActive);
        }
        let index = self
            .entries
            .iter()
            .take(self.len())
            .position(|entry| {
                entry.is_some_and(|entry| {
                    entry.record.previous_revision == Some(revision)
                        || (revision == 0 && entry.record.previous_revision.is_none())
                })
            })
            .ok_or(UpdateError::NoRollbackTarget)?;
        self.rollback_at(index, filesystem, packages)
    }

    fn abort_failed_update<const BLOCKS: usize, const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        filesystem: &mut SynFs<BLOCKS>,
        packages: &mut PackageDaemon<PACKAGES, KEYS>,
        checkpoint: CheckpointInfo,
        previous: Option<RootManifest>,
        status: Status,
    ) -> Result<UpdateReceipt, UpdateError> {
        filesystem
            .rollback_to_checkpoint(checkpoint.id)
            .map_err(UpdateError::Rollback)?;
        packages.restore_configuration(previous)?;
        filesystem
            .release_checkpoint(checkpoint.id)
            .map_err(UpdateError::Rollback)?;
        Err(UpdateError::HealthCheck(status))
    }

    fn rollback_at<const BLOCKS: usize, const PACKAGES: usize, const KEYS: usize>(
        &mut self,
        index: usize,
        filesystem: &mut SynFs<BLOCKS>,
        packages: &mut PackageDaemon<PACKAGES, KEYS>,
    ) -> Result<UpdateReceipt, UpdateError> {
        let entry = self.entries[index].ok_or(UpdateError::NoRollbackTarget)?;
        filesystem
            .rollback_to_checkpoint(entry.checkpoint.id)
            .map_err(UpdateError::Rollback)?;
        packages.restore_configuration(entry.previous)?;
        for slot in &mut self.entries[index..] {
            if let Some(entry) = *slot {
                filesystem
                    .release_checkpoint(entry.checkpoint.id)
                    .map_err(UpdateError::Rollback)?;
            }
            *slot = None;
        }
        Ok(UpdateReceipt {
            record: UpdateRecord {
                revision: entry.previous.map_or(0, |root| root.revision()),
                previous_revision: entry.record.previous_revision,
                generation: filesystem.generation(),
                previous_generation: entry.record.generation,
            },
            rolled_back: true,
        })
    }
}

impl<const HISTORY: usize> Default for UpdateManager<HISTORY> {
    fn default() -> Self {
        Self::new()
    }
}
