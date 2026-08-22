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
        let Some(previous) = previous else {
            return (Self::initial(1), None)
        };
        let mut last_failure = previous.last_failure;
        let mut reported = None;
        let mut failure_count = previous.failure_count;

        match previous.current.state {
            BootState::Failed => {
                last_failure = Some(previous.current);
                reported = last_failure;
            }
            BootState::InProgress => {
                let interrupted = BootAttempt {
                    id: previous.current.id,
                    stage: previous.current.stage,
                    state: BootState::Failed,
                    status: Status::BUSY.raw(),
                    interrupted: true,
                };
                last_failure = Some(interrupted);
                reported = last_failure;
                failure_count = failure_count.saturating_add(1);
            }
            BootState::Succeeded => {}
        }

        let mut next = Self {
            current: BootAttempt {
                id: previous.current.id.saturating_add(1).max(1),
                stage: BootStage::KernelEntry,
                state: BootState::InProgress,
                status: Status::PENDING.raw(),
                interrupted: false,
            },
            last_failure,
            failure_count,
        };
        if next.current.id == previous.current.id {
            next.current.id = 1
        }
        (next, reported)
    }

    pub fn checkpoint(&mut self, stage: BootStage) {
        if self.current.state == BootState::InProgress {
            self.current.stage = stage
        }
    }

    pub fn fail(&mut self, status: Status) {
        if self.current.state != BootState::InProgress {
            return
        }
        self.current.state = BootState::Failed;
        self.current.status = status.raw();
        self.current.interrupted = false;
        self.last_failure = Some(self.current);
        self.failure_count = self.failure_count.saturating_add(1);
    }

    pub fn complete(&mut self) {
        if self.current.state == BootState::InProgress {
            self.current.state = BootState::Succeeded;
            self.current.status = Status::NORMAL.raw();
        }
    }

    pub fn encode(&self, destination: &mut [u8]) -> Option<usize> {
        if destination.len() < BOOT_DIAGNOSTIC_BYTES {
            return None
        }
        destination[..BOOT_DIAGNOSTIC_BYTES].fill(0);
        destination[..8].copy_from_slice(&BOOT_DIAGNOSTIC_MAGIC);
        destination[8..10].copy_from_slice(&BOOT_DIAGNOSTIC_VERSION.to_le_bytes());
        destination[10..12].copy_from_slice(&(BOOT_DIAGNOSTIC_BYTES as u16).to_le_bytes());
        destination[12..20].copy_from_slice(&self.current.id.to_le_bytes());
        destination[20] = self.current.state as u8;
        destination[21] = self.current.stage as u8;
        destination[22] = u8::from(self.current.interrupted);
        destination[24..28].copy_from_slice(&self.current.status.to_le_bytes());
        if let Some(failure) = self.last_failure {
            destination[28..36].copy_from_slice(&failure.id.to_le_bytes());
            destination[36] = failure.stage as u8;
            destination[37] = u8::from(failure.interrupted);
            destination[38..42].copy_from_slice(&failure.status.to_le_bytes());
        }
        destination[42..50].copy_from_slice(&self.failure_count.to_le_bytes());
        let checksum = checksum(&destination[..50]);
        destination[50..54].copy_from_slice(&checksum.to_le_bytes());
        Some(BOOT_DIAGNOSTIC_BYTES)
    }

    pub fn decode(source: &[u8]) -> Option<Self> {
        if source.len() < BOOT_DIAGNOSTIC_BYTES
            || source[..8] != BOOT_DIAGNOSTIC_MAGIC
            || u16::from_le_bytes(source[8..10].try_into().ok()?) != BOOT_DIAGNOSTIC_VERSION
            || u16::from_le_bytes(source[10..12].try_into().ok()?)
                != BOOT_DIAGNOSTIC_BYTES as u16
            || u32::from_le_bytes(source[50..54].try_into().ok()?) != checksum(&source[..50])
            || source[23] != 0
            || source[54..BOOT_DIAGNOSTIC_BYTES].iter().any(|byte| *byte != 0)
        {
            return None
        }
        let current_id = u64::from_le_bytes(source[12..20].try_into().ok()?);
        let current = BootAttempt {
            id: current_id,
            stage: BootStage::from_raw(source[21])?,
            state: BootState::from_raw(source[20])?,
            status: u32::from_le_bytes(source[24..28].try_into().ok()?),
            interrupted: source[22] != 0,
        };
        if current.id == 0 || Status::from_raw(current.status).is_none() {
            return None
        }
        let failure_id = u64::from_le_bytes(source[28..36].try_into().ok()?);
        let last_failure = if failure_id == 0 {
            None
        } else {
            let failure = BootAttempt {
                id: failure_id,
                stage: BootStage::from_raw(source[36])?,
                state: BootState::Failed,
                status: u32::from_le_bytes(source[38..42].try_into().ok()?),
                interrupted: source[37] != 0,
            };
            Status::from_raw(failure.status)?;
            Some(failure)
        };
        Some(Self {
            current,
            last_failure,
            failure_count: u64::from_le_bytes(source[42..50].try_into().ok()?),
        })
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

fn checksum(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5_u32;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
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
