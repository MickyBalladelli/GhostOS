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
