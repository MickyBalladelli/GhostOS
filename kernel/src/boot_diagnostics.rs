//! Crash-safe evidence for the last kernel boot attempt.

use ghostos_status::Status;

use crate::persistence::PersistentStore;

pub const BOOT_DIAGNOSTIC_MAGIC: [u8; 8] = *b"SYNBTD01";
pub const BOOT_DIAGNOSTIC_VERSION: u16 = 1;
pub const BOOT_DIAGNOSTIC_BYTES: usize = 56;
pub const MAX_BOOT_DIAGNOSTIC_BYTES: usize = BOOT_DIAGNOSTIC_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BootStage {
    KernelEntry = 1,
    BootInfoValidated = 2,
    MemoryReady = 3,
    ArchitectureReady = 4,
    HardwareReady = 5,
    StorageReady = 6,
    ServicesReady = 7,
    UserHandoff = 8,
}

impl BootStage {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::KernelEntry),
            2 => Some(Self::BootInfoValidated),
            3 => Some(Self::MemoryReady),
            4 => Some(Self::ArchitectureReady),
            5 => Some(Self::HardwareReady),
            6 => Some(Self::StorageReady),
            7 => Some(Self::ServicesReady),
            8 => Some(Self::UserHandoff),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::KernelEntry => "kernel-entry",
            Self::BootInfoValidated => "boot-info",
            Self::MemoryReady => "memory",
            Self::ArchitectureReady => "architecture",
            Self::HardwareReady => "hardware",
            Self::StorageReady => "storage",
            Self::ServicesReady => "services",
            Self::UserHandoff => "user-handoff",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BootState {
    InProgress = 1,
    Failed = 2,
    Succeeded = 3,
}

impl BootState {
    const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::InProgress),
            2 => Some(Self::Failed),
            3 => Some(Self::Succeeded),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootAttempt {
    pub id: u64,
    pub stage: BootStage,
    pub state: BootState,
    pub status: u32,
    pub interrupted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootDiagnostics {
    pub current: BootAttempt,
    pub last_failure: Option<BootAttempt>,
    pub failure_count: u64,
}

impl BootDiagnostics {
    pub const fn initial(attempt_id: u64) -> Self {
        Self {
            current: BootAttempt {
                id: if attempt_id == 0 { 1 } else { attempt_id },
                stage: BootStage::KernelEntry,
                state: BootState::InProgress,
                status: Status::PENDING.raw(),
                interrupted: false,
            },
            last_failure: None,
            failure_count: 0,
        }
    }

    pub fn begin(previous: Option<Self>) -> (Self, Option<BootAttempt>) {
        let native_previous = previous.map(|diagnostics| diagnostics.to_native());
        let mut next = NativeDiagnostics::EMPTY;
        let mut reported = NativeAttempt::EMPTY;
        let mut has_reported = false;
        unsafe {
            ghostos_boot_diagnostics_begin(native_previous.as_ref().map_or(core::ptr::null(), |value| value),
                native_previous.is_some(), &mut next, &mut reported, &mut has_reported)
        };
        (next.to_public(), has_reported.then(|| reported.to_public()))
    }

    pub fn checkpoint(&mut self, stage: BootStage) {
        let mut native = self.to_native();
        unsafe { ghostos_boot_diagnostics_checkpoint(&mut native, stage as u32) };
        *self = native.to_public();
    }

    pub fn fail(&mut self, status: Status) {
        let mut native = self.to_native();
        unsafe { ghostos_boot_diagnostics_fail(&mut native, status.raw()) };
        *self = native.to_public();
    }

    pub fn complete(&mut self) {
        let mut native = self.to_native();
        unsafe { ghostos_boot_diagnostics_complete(&mut native) };
        *self = native.to_public();
    }

    pub fn encode(&self, destination: &mut [u8]) -> Option<usize> {
        let mut length = 0;
        unsafe {
            ghostos_boot_diagnostics_encode_raw(&self.to_native(), destination.as_mut_ptr(),
                destination.len(), &mut length)
        }.then_some(length)
    }

    pub fn decode(source: &[u8]) -> Option<Self> {
        let mut native = NativeDiagnostics::EMPTY;
        unsafe { ghostos_boot_diagnostics_decode(source.as_ptr(), source.len(), &mut native) }
            .then(|| native.to_public())
    }

    fn to_native(self) -> NativeDiagnostics {
        NativeDiagnostics {
            current: NativeAttempt::from(self.current),
            last_failure: self.last_failure.map_or(NativeAttempt::EMPTY, NativeAttempt::from),
            has_last_failure: self.last_failure.is_some(), failure_count: self.failure_count,
        }
    }
}

pub fn begin() -> Option<BootAttempt> {
    let previous = load();
    let (diagnostics, reported) = BootDiagnostics::begin(previous);
    persist(&diagnostics);
    reported
}

pub fn checkpoint(stage: BootStage) {
    update(|diagnostics| diagnostics.checkpoint(stage))
}

pub fn fail(status: Status) {
    update(|diagnostics| diagnostics.fail(status))
}

pub fn complete() {
    update(BootDiagnostics::complete)
}

pub fn last_failure() -> Option<BootAttempt> {
    load().and_then(|diagnostics| diagnostics.last_failure)
}

fn update(change: impl FnOnce(&mut BootDiagnostics)) {
    let Some(mut diagnostics) = load() else { return };
    change(&mut diagnostics);
    persist(&diagnostics)
}

fn load() -> Option<BootDiagnostics> {
    let mut bytes = [0; MAX_BOOT_DIAGNOSTIC_BYTES];
    let length = PersistentStore::new().load_boot_diagnostic(&mut bytes)?;
    BootDiagnostics::decode(&bytes[..length])
}

fn persist(diagnostics: &BootDiagnostics) {
    let mut bytes = [0; MAX_BOOT_DIAGNOSTIC_BYTES];
    if let Some(length) = diagnostics.encode(&mut bytes) {
        PersistentStore::new().save_boot_diagnostic(&bytes[..length])
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeAttempt {
    id: u64,
    stage: u32,
    state: u32,
    status: u32,
    interrupted: bool,
}

impl NativeAttempt {
    const EMPTY: Self = Self { id: 0, stage: 1, state: 2, status: 0, interrupted: false };

    fn to_public(self) -> BootAttempt {
        BootAttempt {
            id: self.id,
            stage: BootStage::from_raw(self.stage as u8).expect("native boot stage"),
            state: BootState::from_raw(self.state as u8).expect("native boot state"),
            status: self.status, interrupted: self.interrupted,
        }
    }
}

impl From<BootAttempt> for NativeAttempt {
    fn from(attempt: BootAttempt) -> Self {
        Self { id: attempt.id, stage: attempt.stage as u32, state: attempt.state as u32,
            status: attempt.status, interrupted: attempt.interrupted }
    }
}

#[repr(C)]
struct NativeDiagnostics {
    current: NativeAttempt,
    last_failure: NativeAttempt,
    has_last_failure: bool,
    failure_count: u64,
}

impl NativeDiagnostics {
    const EMPTY: Self = Self { current: NativeAttempt::EMPTY, last_failure: NativeAttempt::EMPTY,
        has_last_failure: false, failure_count: 0 };

    fn to_public(self) -> BootDiagnostics {
        BootDiagnostics { current: self.current.to_public(),
            last_failure: self.has_last_failure.then(|| self.last_failure.to_public()),
            failure_count: self.failure_count }
    }
}

const _: () = {
    assert!(core::mem::size_of::<NativeAttempt>() == 24);
    assert!(core::mem::size_of::<NativeDiagnostics>() == 64);
    assert!(core::mem::offset_of!(NativeDiagnostics, failure_count) == 56);
};

unsafe extern "C" {
    fn ghostos_boot_diagnostics_begin(previous: *const NativeDiagnostics, has_previous: bool,
        next: *mut NativeDiagnostics, reported: *mut NativeAttempt, has_reported: *mut bool);
    fn ghostos_boot_diagnostics_checkpoint(diagnostics: *mut NativeDiagnostics, stage: u32);
    fn ghostos_boot_diagnostics_fail(diagnostics: *mut NativeDiagnostics, status: u32);
    fn ghostos_boot_diagnostics_complete(diagnostics: *mut NativeDiagnostics);
    fn ghostos_boot_diagnostics_encode_raw(diagnostics: *const NativeDiagnostics, destination: *mut u8,
        capacity: usize, length: *mut usize) -> bool;
    fn ghostos_boot_diagnostics_decode(source: *const u8, length: usize, diagnostics: *mut NativeDiagnostics) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_attempt_becomes_a_reportable_failure() {
        let (first, _) = BootDiagnostics::begin(None);
        let (second, report) = BootDiagnostics::begin(Some(first));
        assert_eq!(report.unwrap().stage, BootStage::KernelEntry);
        assert!(report.unwrap_or(second.current).interrupted);
        assert_eq!(second.failure_count, 1);
    }

    #[test]
    fn failure_round_trips_with_checksum() {
        let (mut diagnostics, _) = BootDiagnostics::begin(None);
        diagnostics.checkpoint(BootStage::ServicesReady);
        diagnostics.fail(Status::CORRUPT);
        let mut bytes = [0; BOOT_DIAGNOSTIC_BYTES];
        let length = diagnostics.encode(&mut bytes).unwrap();
        assert_eq!(BootDiagnostics::decode(&bytes[..length]), Some(diagnostics));
        bytes[20] ^= 1;
        assert_eq!(BootDiagnostics::decode(&bytes[..length]), None);
    }
}
