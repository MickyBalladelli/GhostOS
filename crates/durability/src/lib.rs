#![no_std]
#![forbid(unsafe_code)]

/// Persistence owner used to label a deterministic interruption point.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum CrashDomain {
    SynFs,
    Storage,
    PackageActivation,
    Configuration,
    CompilerJob,
    UpdateRecovery,
}

impl CrashDomain {
    pub const ALL: [Self; 6] = [
        Self::SynFs,
        Self::Storage,
        Self::PackageActivation,
        Self::Configuration,
        Self::CompilerJob,
        Self::UpdateRecovery,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Durable boundary after which a caller may safely model a power loss.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum CrashBoundary {
    Flush,
    JournalRecord,
    ManifestSlot,
    Rename,
    CapabilityChange,
    ServiceRestart,
}

impl CrashBoundary {
    pub const ALL: [Self; 6] = [
        Self::Flush,
        Self::JournalRecord,
        Self::ManifestSlot,
        Self::Rename,
        Self::CapabilityChange,
        Self::ServiceRestart,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Hook called immediately after a durable boundary becomes observable.
pub trait InterruptionInjector {
    fn checkpoint(&mut self, domain: CrashDomain, boundary: CrashBoundary) -> bool;
}

/// Hook used by production callers when no interruption is being injected.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoInterruption;

impl InterruptionInjector for NoInterruption {
    fn checkpoint(&mut self, _domain: CrashDomain, _boundary: CrashBoundary) -> bool {
        false
    }
}

/// The layers that participate in the durable-write path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurabilityLayer {
    Application,
    SynFs,
    StorageDaemon,
    Cache,
    BlockDevice,
    Recovery,
}

/// The strongest state a layer may report to its caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurabilityGuarantee {
    Volatile,
    Published,
    Durable,
    Recoverable,
}

/// Machine-readable summary of the contract in `docs/durability.md`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LayerContract {
    pub layer: DurabilityLayer,
    pub write: DurabilityGuarantee,
    pub flush: DurabilityGuarantee,
    pub sync: DurabilityGuarantee,
    pub rename: DurabilityGuarantee,
    pub commit: DurabilityGuarantee,
    pub recovery: DurabilityGuarantee,
}

pub const DURABILITY_CONTRACT_VERSION: u16 = 1;

pub const DURABILITY_CONTRACT: [LayerContract; 6] = [
    LayerContract {
        layer: DurabilityLayer::Application,
        write: DurabilityGuarantee::Volatile,
        flush: DurabilityGuarantee::Volatile,
        sync: DurabilityGuarantee::Durable,
        rename: DurabilityGuarantee::Volatile,
        commit: DurabilityGuarantee::Volatile,
        recovery: DurabilityGuarantee::Recoverable,
    },
    LayerContract {
        layer: DurabilityLayer::SynFs,
        write: DurabilityGuarantee::Volatile,
        flush: DurabilityGuarantee::Published,
        sync: DurabilityGuarantee::Durable,
        rename: DurabilityGuarantee::Published,
        commit: DurabilityGuarantee::Published,
        recovery: DurabilityGuarantee::Recoverable,
    },
    LayerContract {
        layer: DurabilityLayer::StorageDaemon,
        write: DurabilityGuarantee::Volatile,
        flush: DurabilityGuarantee::Published,
        sync: DurabilityGuarantee::Durable,
        rename: DurabilityGuarantee::Volatile,
        commit: DurabilityGuarantee::Published,
        recovery: DurabilityGuarantee::Recoverable,
    },
    LayerContract {
        layer: DurabilityLayer::Cache,
        write: DurabilityGuarantee::Volatile,
        flush: DurabilityGuarantee::Durable,
        sync: DurabilityGuarantee::Durable,
        rename: DurabilityGuarantee::Volatile,
        commit: DurabilityGuarantee::Published,
        recovery: DurabilityGuarantee::Recoverable,
    },
    LayerContract {
        layer: DurabilityLayer::BlockDevice,
        write: DurabilityGuarantee::Volatile,
        flush: DurabilityGuarantee::Durable,
        sync: DurabilityGuarantee::Durable,
        rename: DurabilityGuarantee::Volatile,
        commit: DurabilityGuarantee::Published,
        recovery: DurabilityGuarantee::Recoverable,
    },
    LayerContract {
        layer: DurabilityLayer::Recovery,
        write: DurabilityGuarantee::Recoverable,
        flush: DurabilityGuarantee::Recoverable,
        sync: DurabilityGuarantee::Recoverable,
        rename: DurabilityGuarantee::Recoverable,
        commit: DurabilityGuarantee::Recoverable,
        recovery: DurabilityGuarantee::Recoverable,
    },
];

/// One observable step in a durable transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurabilityEvent {
    ApplicationWrite { transaction: u64 },
    SynFsWrite { transaction: u64 },
    Rename { transaction: u64 },
    Commit { transaction: u64 },
    StorageDaemonWrite { transaction: u64 },
    CacheFlush { transaction: u64 },
    BlockDataWrite { transaction: u64 },
    BlockCommitRecord { transaction: u64 },
    BlockFlush { transaction: u64 },
    SyncAcknowledged { transaction: u64 },
    PowerLoss,
    Recovered { transaction: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractError {
    TraceFull,
    MissingStep { transaction: u64 },
    InvalidOrder { transaction: u64 },
    RecoveredVolatile { transaction: u64 },
}

/// Fixed-capacity ordering proof used by layer and power-loss tests.
pub struct DurabilityTrace<const CAPACITY: usize> {
    events: [Option<DurabilityEvent>; CAPACITY],
    length: usize,
}

impl<const CAPACITY: usize> DurabilityTrace<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            events: [None; CAPACITY],
            length: 0,
        }
    }

    pub fn record(&mut self, event: DurabilityEvent) -> Result<(), ContractError> {
        let slot = self.events.get_mut(self.length).ok_or(ContractError::TraceFull)?;
        *slot = Some(event);
        self.length += 1;
        Ok(())
    }

    pub fn verify(&self) -> Result<(), ContractError> {
        for index in 0..self.length {
            let Some(DurabilityEvent::SyncAcknowledged { transaction }) = self.events[index]
            else {
                continue
            };
            let application = self.find_before(index, transaction, |event| {
                matches!(event, DurabilityEvent::ApplicationWrite { .. })
            });
            let ghostfs_write = self.find_before(index, transaction, |event| {
                matches!(event, DurabilityEvent::SynFsWrite { .. })
            });
            let commit = self.find_before(index, transaction, |event| {
                matches!(event, DurabilityEvent::Commit { .. })
            });
            let data = self.find_before(index, transaction, |event| {
                matches!(event, DurabilityEvent::BlockDataWrite { .. })
            });
            let commit_record = self.find_before(index, transaction, |event| {
                matches!(event, DurabilityEvent::BlockCommitRecord { .. })
            });
            let block_flush = self.find_before(index, transaction, |event| {
                matches!(event, DurabilityEvent::BlockFlush { .. })
            });
            let Some(application) = application else {
                return Err(ContractError::MissingStep { transaction })
            };
            let Some(ghostfs_write) = ghostfs_write else {
                return Err(ContractError::MissingStep { transaction })
            };
            let Some(commit) = commit else {
                return Err(ContractError::MissingStep { transaction })
            };
            let Some(data) = data else {
                return Err(ContractError::MissingStep { transaction })
            };
            let Some(commit_record) = commit_record else {
                return Err(ContractError::MissingStep { transaction })
            };
            let Some(block_flush) = block_flush else {
                return Err(ContractError::MissingStep { transaction })
            };
            if !(application < ghostfs_write
                && ghostfs_write < commit
                && commit < data
                && data < commit_record
                && commit_record < block_flush
                && block_flush < index)
            {
                return Err(ContractError::InvalidOrder { transaction })
            }
            let storage = self.find_before(index, transaction, |event| {
                matches!(event, DurabilityEvent::StorageDaemonWrite { .. })
            });
            let cache = self.find_before(index, transaction, |event| {
                matches!(event, DurabilityEvent::CacheFlush { .. })
            });
            let rename = self.find_before(index, transaction, |event| {
                matches!(event, DurabilityEvent::Rename { .. })
            });
            if rename.is_some_and(|rename| rename > commit)
                || storage.is_some_and(|storage| storage < commit || storage > data)
                || cache.is_some_and(|cache| cache < storage.unwrap_or(commit))
                || cache.is_some_and(|cache| cache > data)
                || cache.is_some_and(|_| storage.is_none())
                || self.has_data_after_commit_record(transaction, commit_record, index)
            {
                return Err(ContractError::InvalidOrder { transaction })
            }
        }

        let mut power_loss = None;
        for index in 0..self.length {
            match self.events[index] {
                Some(DurabilityEvent::PowerLoss) => power_loss = Some(index),
                Some(DurabilityEvent::Recovered { transaction }) => {
                    let Some(power_loss) = power_loss else {
                        return Err(ContractError::RecoveredVolatile { transaction })
                    };
                    let durable = (0..power_loss).any(|before| {
                        self.events[before]
                            == Some(DurabilityEvent::SyncAcknowledged { transaction })
                    });
                    if !durable {
                        return Err(ContractError::RecoveredVolatile { transaction })
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn events(&self) -> &[Option<DurabilityEvent>] {
        &self.events[..self.length]
    }

    fn find_before(
        &self,
        end: usize,
        transaction: u64,
        predicate: impl Fn(DurabilityEvent) -> bool,
    ) -> Option<usize> {
        (0..end).rev().find(|index| {
            self.events[*index].is_some_and(|event| match event {
                DurabilityEvent::PowerLoss | DurabilityEvent::Recovered { .. } => false,
                _ => event_transaction(event) == Some(transaction) && predicate(event),
            })
        })
    }

    fn has_data_after_commit_record(
        &self,
        transaction: u64,
        commit_record: usize,
        end: usize,
    ) -> bool {
        self.events[commit_record + 1..end].iter().flatten().any(|event| {
            matches!(event, DurabilityEvent::BlockDataWrite { transaction: value } if *value == transaction)
        })
    }
}

impl<const CAPACITY: usize> Default for DurabilityTrace<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn event_transaction(event: DurabilityEvent) -> Option<u64> {
    match event {
        DurabilityEvent::ApplicationWrite { transaction }
        | DurabilityEvent::SynFsWrite { transaction }
        | DurabilityEvent::Rename { transaction }
        | DurabilityEvent::Commit { transaction }
        | DurabilityEvent::StorageDaemonWrite { transaction }
        | DurabilityEvent::CacheFlush { transaction }
        | DurabilityEvent::BlockDataWrite { transaction }
        | DurabilityEvent::BlockCommitRecord { transaction }
        | DurabilityEvent::BlockFlush { transaction }
        | DurabilityEvent::SyncAcknowledged { transaction } => Some(transaction),
        DurabilityEvent::PowerLoss | DurabilityEvent::Recovered { .. } => None,
    }
}
