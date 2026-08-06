use synos_fabric::NodeId;
use synos_status::Status;
use synos_synfs::{CheckpointId, Error as SynFsError, SynFs};

use crate::{SignatureError, SignedConfiguration, SystemSpec, TpmConfigurationEnforcer};

pub const DEFAULT_RECONFIGURE_HISTORY: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivationReceipt {
    pub revision: u64,
    pub previous_revision: Option<u64>,
    pub generation: u64,
    pub previous_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconfigureError<E> {
    AlreadyActive,
    HistoryFull,
    Signature(SignatureError),
    StaleRevision,
    Storage(SynFsError),
    HealthCheck(Status),
    Runtime(E),
    NoRollbackTarget,
}

/// Runtime bridge used by init, netd, and policy daemons. `stage` must only
/// prepare private state. Activation health-checks the staged state before
/// `commit`, which is the short atomic publication point.
pub trait ConfigurationRuntime {
    type Error;

    fn stage(&mut self, configuration: &SystemSpec) -> Result<(), Self::Error>;
    fn clear_stage(&mut self);
    fn commit(&mut self) -> Result<(), Self::Error>;
    fn rollback(&mut self);
    fn health_check(&mut self, configuration: &SystemSpec) -> Result<(), Status>;
}

#[derive(Clone, Copy)]
struct HistoryEntry {
    checkpoint: CheckpointId,
    previous: Option<SystemSpec>,
    receipt: ActivationReceipt,
}

pub struct ReconfigureManager<const HISTORY: usize = DEFAULT_RECONFIGURE_HISTORY> {
    active: Option<SystemSpec>,
    entries: [Option<HistoryEntry>; HISTORY],
}

impl<const HISTORY: usize> ReconfigureManager<HISTORY> {
    pub const fn new() -> Self {
        Self {
            active: None,
            entries: [None; HISTORY],
        }
    }

    pub const fn active(&self) -> Option<&SystemSpec> {
        self.active.as_ref()
    }

    pub const fn history_len(&self) -> usize {
        let mut length = 0;
        while length < HISTORY {
            if self.entries[length].is_none() {
                return length;
            }
            length += 1;
        }
        length
    }

    pub fn activate<const BLOCKS: usize, const KEYS: usize, R: ConfigurationRuntime>(
        &mut self,
        filesystem: &mut SynFs<BLOCKS>,
        enforcer: &TpmConfigurationEnforcer<KEYS>,
        update: &SignedConfiguration,
        node: NodeId,
        runtime: &mut R,
    ) -> Result<ActivationReceipt, ReconfigureError<R::Error>> {
        enforcer
            .verify(update, node)
            .map_err(ReconfigureError::Signature)?;
        if self.history_len() == HISTORY || HISTORY == 0 {
            return Err(ReconfigureError::HistoryFull);
        }
        let previous = self.active;
        if update.expected_previous_revision()
            != previous.map(|configuration| configuration.revision())
            && update.expected_previous_revision().is_some()
        {
            return Err(ReconfigureError::StaleRevision);
        }
        if previous.is_some_and(|configuration| configuration.revision() == update.revision()) {
            return Err(ReconfigureError::AlreadyActive);
        }

        let checkpoint = filesystem
            .create_checkpoint()
            .map_err(ReconfigureError::Storage)?;
        let rollback = |filesystem: &mut SynFs<BLOCKS>, runtime: &mut R| {
            runtime.rollback();
            runtime.clear_stage();
            let result = filesystem.rollback_to_checkpoint(checkpoint.id);
            let _ = filesystem.release_checkpoint(checkpoint.id);
            result
        };

        if let Err(error) = runtime.stage(update.configuration()) {
            let _ = rollback(filesystem, runtime);
            return Err(ReconfigureError::Runtime(error));
        }
        if let Err(status) = runtime.health_check(update.configuration()) {
            let _ = rollback(filesystem, runtime);
            return Err(ReconfigureError::HealthCheck(status));
        }
        if let Err(error) = runtime.commit() {
            let _ = rollback(filesystem, runtime);
            return Err(ReconfigureError::Runtime(error));
        }

        let receipt = ActivationReceipt {
            revision: update.revision(),
            previous_revision: previous.map(|configuration| configuration.revision()),
            generation: filesystem.generation(),
            previous_generation: checkpoint.generation,
        };
        self.entries[self.history_len()] = Some(HistoryEntry {
            checkpoint: checkpoint.id,
            previous,
            receipt,
        });
        self.active = Some(*update.configuration());
        Ok(receipt)
    }

    pub fn rollback_last<const BLOCKS: usize, R: ConfigurationRuntime>(
        &mut self,
        filesystem: &mut SynFs<BLOCKS>,
        runtime: &mut R,
    ) -> Result<ActivationReceipt, ReconfigureError<R::Error>> {
        let index = self
            .history_len()
            .checked_sub(1)
            .ok_or(ReconfigureError::NoRollbackTarget)?;
        let entry = self.entries[index].ok_or(ReconfigureError::NoRollbackTarget)?;
        filesystem
            .rollback_to_checkpoint(entry.checkpoint)
            .map_err(ReconfigureError::Storage)?;
        if let Some(previous) = entry.previous {
            runtime
                .stage(&previous)
                .map_err(ReconfigureError::Runtime)?;
        } else {
            runtime.clear_stage();
        }
        runtime.commit().map_err(ReconfigureError::Runtime)?;
        filesystem
            .release_checkpoint(entry.checkpoint)
            .map_err(ReconfigureError::Storage)?;
        self.entries[index] = None;
        self.active = entry.previous;
        Ok(ActivationReceipt {
            revision: entry
                .previous
                .map_or(0, |configuration| configuration.revision()),
            previous_revision: Some(entry.receipt.revision),
            generation: filesystem.generation(),
            previous_generation: entry.receipt.generation,
        })
    }
}

impl<const HISTORY: usize> Default for ReconfigureManager<HISTORY> {
    fn default() -> Self {
        Self::new()
    }
}
