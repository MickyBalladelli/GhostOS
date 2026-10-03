#![no_std]

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
    records: [EventRecord; CAPACITY],
    length: usize,
}

impl<const CAPACITY: usize> DurabilityTrace<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            events: [None; CAPACITY],
            records: [EventRecord::EMPTY; CAPACITY],
            length: 0,
        }
    }

    pub fn record(&mut self, event: DurabilityEvent) -> Result<(), ContractError> {
        let index = self.length;
        // C writes one record within the supplied capacity and updates length.
        let code = unsafe {
            ghostos_durability_record(self.records.as_mut_ptr(), CAPACITY,
                &mut self.length, EventRecord::from_event(event))
        };
        if code != 0 {
            return Err(ContractError::TraceFull)
        }
        self.events[index] = Some(event);
        Ok(())
    }

    pub fn verify(&self) -> Result<(), ContractError> {
        let mut transaction = 0;
        // C reads initialized event records and retains no pointers.
        let code = unsafe {
            ghostos_durability_verify(self.records.as_ptr(), self.length, &mut transaction)
        };
        match code {
            0 => Ok(()),
            2 => Err(ContractError::MissingStep { transaction }),
            3 => Err(ContractError::InvalidOrder { transaction }),
            _ => Err(ContractError::RecoveredVolatile { transaction }),
        }
    }

    pub fn events(&self) -> &[Option<DurabilityEvent>] {
        &self.events[..self.length]
    }

}

impl<const CAPACITY: usize> Default for DurabilityTrace<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventRecord {
    transaction: u64,
    kind: u32,
    reserved: u32,
}

const _: () = {
    assert!(core::mem::size_of::<EventRecord>() == 16);
    assert!(core::mem::offset_of!(EventRecord, transaction) == 0);
    assert!(core::mem::offset_of!(EventRecord, kind) == 8);
    assert!(core::mem::offset_of!(EventRecord, reserved) == 12);
};

impl EventRecord {
    const EMPTY: Self = Self { transaction: 0, kind: 0, reserved: 0 };

    fn from_event(event: DurabilityEvent) -> Self {
        let (kind, transaction) = match event {
            DurabilityEvent::ApplicationWrite { transaction } => (0, transaction),
            DurabilityEvent::SynFsWrite { transaction } => (1, transaction),
            DurabilityEvent::Rename { transaction } => (2, transaction),
            DurabilityEvent::Commit { transaction } => (3, transaction),
            DurabilityEvent::StorageDaemonWrite { transaction } => (4, transaction),
            DurabilityEvent::CacheFlush { transaction } => (5, transaction),
            DurabilityEvent::BlockDataWrite { transaction } => (6, transaction),
            DurabilityEvent::BlockCommitRecord { transaction } => (7, transaction),
            DurabilityEvent::BlockFlush { transaction } => (8, transaction),
            DurabilityEvent::SyncAcknowledged { transaction } => (9, transaction),
            DurabilityEvent::PowerLoss => (10, 0),
            DurabilityEvent::Recovered { transaction } => (11, transaction),
        };
        Self { transaction, kind, reserved: 0 }
    }
}

unsafe extern "C" {
    fn ghostos_durability_record(events: *mut EventRecord, capacity: usize,
        length: *mut usize, event: EventRecord) -> u32;
    fn ghostos_durability_verify(events: *const EventRecord, length: usize,
        failed_transaction: *mut u64) -> u32;
}
