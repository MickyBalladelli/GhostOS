use ghostos_init::ProcessId;
use ghostos_status::{IntoStatus, Status};
use ghostos_ghostfs::{CheckpointInfo, Error as SynFsError, ReadOnlySnapshot, RmsMapHandle, SynFs};

pub const DEFAULT_RECOVERY_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoverySpawnRequest {
    pub service: u64,
    pub image: u128,
    pub generation: u32,
    pub snapshot: CheckpointInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryReceipt {
    pub service: u64,
    pub process: ProcessId,
    pub generation: u32,
    pub snapshot: CheckpointInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryStatus {
    pub service: u64,
    pub image: u128,
    pub process: Option<ProcessId>,
    pub generation: u32,
    pub snapshot: Option<CheckpointInfo>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryError {
    Capacity,
    InvalidService,
    AlreadyRegistered,
    NotFound,
    StaleProcess,
    NoSnapshot,
    InvalidProcess,
    Snapshot(SynFsError),
    FenceFailed(Status),
    RestoreFailed(Status),
    SpawnFailed(Status),
}

impl IntoStatus for RecoveryError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::InvalidService | Self::AlreadyRegistered | Self::InvalidProcess => {
                Status::INVALID_ARGUMENT
            }
            Self::NotFound | Self::StaleProcess | Self::NoSnapshot => Status::NOT_FOUND,
            Self::Snapshot(error) => error.status(),
            Self::FenceFailed(status) | Self::RestoreFailed(status) | Self::SpawnFailed(status) => {
                status
            }
        }
    }
}

/// Runtime operations required to restore one isolated daemon.
///
/// The runtime owns the privileged work. `restore_state` receives a pinned
/// GhostFS generation, allowing it to map and copy only the daemon's state
/// before the replacement process is spawned.
pub trait RecoveryRuntime {
    type Error;

    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error>;
    fn restore_state<const MAX_BLOCKS: usize>(
        &mut self,
        service: u64,
        snapshot: &ReadOnlySnapshot<'_, MAX_BLOCKS>,
    ) -> Result<(), Self::Error>;
    fn spawn_recovered(&mut self, request: RecoverySpawnRequest) -> Result<ProcessId, Self::Error>;
}

#[derive(Clone, Copy)]
struct RecoverySlot {
    occupied: bool,
    service: u64,
    image: u128,
    process: Option<ProcessId>,
    generation: u32,
    snapshot: Option<CheckpointInfo>,
}

impl RecoverySlot {
    const EMPTY: Self = Self {
        occupied: false,
        service: 0,
        image: 0,
        process: None,
        generation: 0,
        snapshot: None,
    };
}

/// Bounded registry of clean daemon generations and their pinned CoW roots.
pub struct RecoveryCoordinator<const CAPACITY: usize = DEFAULT_RECOVERY_CAPACITY> {
    slots: [RecoverySlot; CAPACITY],
}

impl<const CAPACITY: usize> RecoveryCoordinator<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY > 0);
        Self {
            slots: [RecoverySlot::EMPTY; CAPACITY],
        }
    }

    pub fn register(
        &mut self,
        service: u64,
        image: u128,
        process: ProcessId,
    ) -> Result<(), RecoveryError> {
        if service == 0 {
            return Err(RecoveryError::InvalidService);
        }
        if image == 0 || process.raw() == 0 {
            return Err(RecoveryError::InvalidProcess);
        }
        if self
            .slots
            .iter()
            .any(|slot| slot.occupied && slot.service == service)
        {
            return Err(RecoveryError::AlreadyRegistered);
        }
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| !slot.occupied)
            .ok_or(RecoveryError::Capacity)?;
        *slot = RecoverySlot {
            occupied: true,
            service,
            image,
            process: Some(process),
            generation: 1,
            snapshot: None,
        };
        Ok(())
    }

    pub fn set_process(&mut self, service: u64, process: ProcessId) -> Result<(), RecoveryError> {
        if process.raw() == 0 {
            return Err(RecoveryError::InvalidProcess);
        }
        let slot = self.slot_mut(service)?;
        slot.process = Some(process);
        Ok(())
    }

    /// Pin the current GhostFS root as the daemon's latest clean state.
    pub fn capture_clean_snapshot<const MAX_BLOCKS: usize>(
        &mut self,
        filesystem: &mut SynFs<MAX_BLOCKS>,
        service: u64,
    ) -> Result<CheckpointInfo, RecoveryError> {
        let slot = self.slot_mut(service)?;
        let checkpoint = filesystem
            .create_checkpoint()
            .map_err(RecoveryError::Snapshot)?;
        if let Some(previous) = slot.snapshot {
            if let Err(error) = filesystem.release_checkpoint(previous.id) {
                let _ = filesystem.release_checkpoint(checkpoint.id);
                return Err(RecoveryError::Snapshot(error));
            }
        }
        slot.snapshot = Some(checkpoint);
        Ok(checkpoint)
    }

    pub fn release_snapshot<const MAX_BLOCKS: usize>(
        &mut self,
        filesystem: &mut SynFs<MAX_BLOCKS>,
        service: u64,
    ) -> Result<(), RecoveryError> {
        let slot = self.slot_mut(service)?;
        let snapshot = slot.snapshot.ok_or(RecoveryError::NoSnapshot)?;
        filesystem
            .release_checkpoint(snapshot.id)
            .map_err(RecoveryError::Snapshot)?;
        slot.snapshot = None;
        Ok(())
    }

    pub fn recover<const MAX_BLOCKS: usize, R: RecoveryRuntime>(
        &mut self,
        service: u64,
        crashed_process: ProcessId,
        filesystem: &SynFs<MAX_BLOCKS>,
        capability: RmsMapHandle,
        runtime: &mut R,
    ) -> Result<RecoveryReceipt, RecoveryError> {
        let slot = self.slot_mut(service)?;
        if slot.process != Some(crashed_process) {
            return Err(RecoveryError::StaleProcess);
        }
        let snapshot = slot.snapshot.ok_or(RecoveryError::NoSnapshot)?;
        runtime
            .fence_process(crashed_process)
            .map_err(|_| RecoveryError::FenceFailed(Status::BUSY))?;
        slot.process = None;
        let snapshot_view = filesystem
            .checkpoint_snapshot(snapshot.id, capability)
            .map_err(RecoveryError::Snapshot)?;
        runtime
            .restore_state(service, &snapshot_view)
            .map_err(|_| RecoveryError::RestoreFailed(Status::CORRUPT))?;

        let generation = slot.generation.wrapping_add(1).max(1);
        let process = runtime
            .spawn_recovered(RecoverySpawnRequest {
                service,
                image: slot.image,
                generation,
                snapshot,
            })
            .map_err(|_| RecoveryError::SpawnFailed(Status::BUSY))?;
        if process.raw() == 0 || process == crashed_process {
            return Err(RecoveryError::InvalidProcess);
        }
        slot.process = Some(process);
        slot.generation = generation;
        Ok(RecoveryReceipt {
            service,
            process,
            generation,
            snapshot,
        })
    }

    pub fn status(&self, service: u64) -> Result<RecoveryStatus, RecoveryError> {
        let slot = self.slot(service)?;
        Ok(RecoveryStatus {
            service: slot.service,
            image: slot.image,
            process: slot.process,
            generation: slot.generation,
            snapshot: slot.snapshot,
        })
    }

    pub fn statuses(&self) -> impl Iterator<Item = RecoveryStatus> + '_ {
        self.slots
            .iter()
            .filter(|slot| slot.occupied)
            .map(|slot| RecoveryStatus {
                service: slot.service,
                image: slot.image,
                process: slot.process,
                generation: slot.generation,
                snapshot: slot.snapshot,
            })
    }

    fn slot(&self, service: u64) -> Result<&RecoverySlot, RecoveryError> {
        self.slots
            .iter()
            .find(|slot| slot.occupied && slot.service == service)
            .ok_or(RecoveryError::NotFound)
    }

    fn slot_mut(&mut self, service: u64) -> Result<&mut RecoverySlot, RecoveryError> {
        self.slots
            .iter_mut()
            .find(|slot| slot.occupied && slot.service == service)
            .ok_or(RecoveryError::NotFound)
    }
}

impl<const CAPACITY: usize> Default for RecoveryCoordinator<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
