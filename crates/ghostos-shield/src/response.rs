//! Bounded incident response primitives for the Ring 0 protection boundary.
//!
//! The tables in this module are intentionally fixed-capacity. Revocation and
//! honeypot checks therefore have predictable work and can be driven directly
//! by a microkernel trap or NIC receive path without allocating memory.

use ghostos_fabric::PAGE_SIZE;
use ghostos_init::ProcessId;
use ghostos_observability::{EventField, Level, audit_event, field};
use ghostos_status::{IntoStatus, Status};
use ghostos_ghostfs::{CheckpointInfo, Error as SynFsError, SynFs};

use crate::{Error, probes::{Operation, ProbeEvent}};

pub const DEFAULT_REVOCATION_CAPACITY: usize = 128;
pub const DEFAULT_SUBJECT_REVOCATION_CAPACITY: usize = 64;
pub const DEFAULT_HONEYPOT_CAPACITY: usize = 32;
pub const DEFAULT_INCIDENT_CAPACITY: usize = 32;

/// Namespace name reserved for decoy pages exposed by the kernel.
pub const SYS_HONEYPOT: &str = "SYS$HONEYPOT";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityLease {
    pub subject: u64,
    pub capability: u64,
    pub epoch: u64,
}

impl CapabilityLease {
    pub const fn new(subject: u64, capability: u64, epoch: u64) -> Result<Self, Error> {
        if subject == 0 || capability == 0 {
            return Err(Error::InvalidInput)
        }
        Ok(Self {
            subject,
            capability,
            epoch,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevocationReceipt {
    pub subject: u64,
    pub capability: Option<u64>,
    pub epoch: u64,
}

#[derive(Clone, Copy)]
struct RevokedCapability {
    subject: u64,
    capability: u64,
    epoch: u64,
}

#[derive(Clone, Copy)]
struct RevokedSubject {
    subject: u64,
    epoch: u64,
}

/// Generation-checked capability revocation.
///
/// Issuing a lease is a read-only operation. Revoking a handle or subject
/// advances the epoch synchronously; every lease issued before that epoch is
/// rejected by `check`. This makes the revocation point a single bounded
/// table update instead of a scan of process or thread state.
pub struct CapabilityRevocation<
    const CAPABILITIES: usize = DEFAULT_REVOCATION_CAPACITY,
    const SUBJECTS: usize = DEFAULT_SUBJECT_REVOCATION_CAPACITY,
> {
    epoch: u64,
    capabilities: [Option<RevokedCapability>; CAPABILITIES],
    subjects: [Option<RevokedSubject>; SUBJECTS],
}

impl<const CAPABILITIES: usize, const SUBJECTS: usize>
    CapabilityRevocation<CAPABILITIES, SUBJECTS>
{
    pub const fn new() -> Self {
        Self {
            epoch: 1,
            capabilities: [None; CAPABILITIES],
            subjects: [None; SUBJECTS],
        }
    }

    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn issue(&self, subject: u64, capability: u64) -> Result<CapabilityLease, Error> {
        if self.access_revoked(subject, capability) {
            return Err(Error::Unauthorized)
        }
        CapabilityLease::new(subject, capability, self.epoch)
    }

    pub fn revoke(
        &mut self,
        subject: u64,
        capability: u64,
    ) -> Result<RevocationReceipt, Error> {
        if subject == 0 || capability == 0 {
            return Err(Error::InvalidInput)
        }
        let existing = self
            .capabilities
            .iter()
            .position(|entry| {
                entry.is_some_and(|entry| {
                    entry.subject == subject && entry.capability == capability
                })
            });
        let slot = existing.or_else(|| self.capabilities.iter().position(|entry| entry.is_none()));
        let Some(slot) = slot else {
            return Err(Error::Capacity)
        };
        let epoch = self.advance_epoch()?;
        if existing.is_some() {
            let entry = self.capabilities[slot].as_mut().expect("validated revocation slot");
            entry.epoch = epoch;
        } else {
            self.capabilities[slot] = Some(RevokedCapability {
                subject,
                capability,
                epoch,
            });
        }
        audit_event!(
            Level::Critical,
            EventField::unsigned(field::CALLER, subject),
            EventField::unsigned(field::CAPABILITY, capability),
            EventField::unsigned(field::OPERATION, epoch),
            EventField::status(Status::ACCESS_DENIED),
        );
        Ok(RevocationReceipt {
            subject,
            capability: Some(capability),
            epoch,
        })
    }

    pub fn revoke_subject(&mut self, subject: u64) -> Result<RevocationReceipt, Error> {
        if subject == 0 {
            return Err(Error::InvalidInput)
        }
        let existing = self
            .subjects
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.subject == subject));
        let slot = existing.or_else(|| self.subjects.iter().position(|entry| entry.is_none()));
        let Some(slot) = slot else {
            return Err(Error::Capacity)
        };
        let epoch = self.advance_epoch()?;
        if existing.is_some() {
            let entry = self.subjects[slot].as_mut().expect("validated subject slot");
            entry.epoch = epoch;
        } else {
            self.subjects[slot] = Some(RevokedSubject { subject, epoch });
        }
        audit_event!(
            Level::Critical,
            EventField::unsigned(field::CALLER, subject),
            EventField::unsigned(field::OPERATION, epoch),
            EventField::status(Status::ACCESS_DENIED),
        );
        Ok(RevocationReceipt {
            subject,
            capability: None,
            epoch,
        })
    }

    pub fn check(&self, lease: CapabilityLease) -> Result<(), Error> {
        if lease.subject == 0 || lease.capability == 0 {
            return Err(Error::InvalidInput)
        }
        let revoked = self
            .subjects
            .iter()
            .flatten()
            .any(|entry| entry.subject == lease.subject && entry.epoch > lease.epoch)
            || self.capabilities.iter().flatten().any(|entry| {
                entry.subject == lease.subject
                    && entry.capability == lease.capability
                    && entry.epoch > lease.epoch
            });
        if revoked {
            audit_event!(
                Level::Warn,
                EventField::unsigned(field::CALLER, lease.subject),
                EventField::unsigned(field::CAPABILITY, lease.capability),
                EventField::status(Status::ACCESS_DENIED),
            );
            Err(Error::Unauthorized)
        } else {
            Ok(())
        }
    }

    pub fn is_revoked(&self, lease: CapabilityLease) -> bool {
        self.check(lease).is_err()
    }

    pub fn access_revoked(&self, subject: u64, capability: u64) -> bool {
        self.subjects
            .iter()
            .flatten()
            .any(|entry| entry.subject == subject)
            || self.capabilities.iter().flatten().any(|entry| {
                entry.subject == subject && entry.capability == capability
            })
    }

    fn advance_epoch(&mut self) -> Result<u64, Error> {
        self.epoch = self.epoch.checked_add(1).ok_or(Error::InvalidConfiguration)?;
        Ok(self.epoch)
    }
}

impl<const CAPABILITIES: usize, const SUBJECTS: usize> Default
    for CapabilityRevocation<CAPABILITIES, SUBJECTS>
{
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HoneypotRegion {
    pub id: u64,
    pub start: u64,
    pub length: u64,
}

impl HoneypotRegion {
    pub fn new(id: u64, start: u64, length: u64) -> Result<Self, Error> {
        if id == 0
            || start == 0
            || start % PAGE_SIZE != 0
            || length == 0
            || length % PAGE_SIZE != 0
            || start.checked_add(length).is_none()
        {
            return Err(Error::InvalidConfiguration)
        }
        Ok(Self { id, start, length })
    }

    fn contains(self, address: u64, length: u64) -> bool {
        let length = length.max(1);
        let Some(end) = address.checked_add(length) else {
            return false
        };
        let Some(region_end) = self.start.checked_add(self.length) else {
            return false
        };
        address >= self.start && end <= region_end
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HoneypotHit {
    pub region: HoneypotRegion,
    pub event: ProbeEvent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HoneypotDecision {
    Clean,
    Quarantine(HoneypotHit),
}

/// Global decoy-page registry. A hit is returned to the caller so the caller
/// can fence the subject in the same trap path; the registry also keeps a
/// monotonic hit count for telemetry.
pub struct HoneypotMemory<const CAPACITY: usize = DEFAULT_HONEYPOT_CAPACITY> {
    regions: [Option<HoneypotRegion>; CAPACITY],
    hits: u64,
}

impl<const CAPACITY: usize> HoneypotMemory<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            regions: [None; CAPACITY],
            hits: 0,
        }
    }

    pub const fn hits(&self) -> u64 {
        self.hits
    }

    pub fn add(&mut self, region: HoneypotRegion) -> Result<(), Error> {
        if self.regions.iter().flatten().any(|entry| {
            entry.id == region.id
                || entry.contains(region.start, 1)
                || region.contains(entry.start, 1)
        }) {
            return Err(Error::Duplicate)
        }
        let slot = self
            .regions
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(region);
        Ok(())
    }

    pub fn inspect(&mut self, event: ProbeEvent) -> HoneypotDecision {
        if !matches!(
            event.operation,
            Operation::MemoryRead | Operation::MemoryWrite | Operation::MemoryExecute
        ) {
            return HoneypotDecision::Clean
        }
        let Some(region) = self
            .regions
            .iter()
            .flatten()
            .copied()
            .find(|region| region.contains(event.address, event.length))
        else {
            return HoneypotDecision::Clean
        };
        self.hits = self.hits.saturating_add(1);
        audit_event!(
            Level::Critical,
            EventField::unsigned(field::CALLER, event.subject),
            EventField::unsigned(field::CAPABILITY, event.capability),
            EventField::unsigned(field::ADDRESS, event.address),
            EventField::status(Status::ACCESS_DENIED),
        );
        HoneypotDecision::Quarantine(HoneypotHit { region, event })
    }
}

impl<const CAPACITY: usize> Default for HoneypotMemory<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CountermeasureDecision {
    Allowed,
    Revoked,
    Quarantined { region: HoneypotRegion, epoch: u64 },
}

/// Couples honeypot detection to immediate subject-wide capability fencing.
pub struct ActiveCountermeasures<
    const REVOCATIONS: usize = DEFAULT_REVOCATION_CAPACITY,
    const SUBJECTS: usize = DEFAULT_SUBJECT_REVOCATION_CAPACITY,
    const HONEYPOTS: usize = DEFAULT_HONEYPOT_CAPACITY,
> {
    pub revocations: CapabilityRevocation<REVOCATIONS, SUBJECTS>,
    pub honeypots: HoneypotMemory<HONEYPOTS>,
}

impl<const REVOCATIONS: usize, const SUBJECTS: usize, const HONEYPOTS: usize>
    ActiveCountermeasures<REVOCATIONS, SUBJECTS, HONEYPOTS>
{
    pub const fn new() -> Self {
        Self {
            revocations: CapabilityRevocation::new(),
            honeypots: HoneypotMemory::new(),
        }
    }

    pub fn inspect(&mut self, event: ProbeEvent) -> Result<CountermeasureDecision, Error> {
        if event.subject == 0 || event.capability == 0 {
            return Err(Error::InvalidInput)
        }
        if self
            .revocations
            .access_revoked(event.subject, event.capability)
        {
            return Ok(CountermeasureDecision::Revoked)
        }
        match self.honeypots.inspect(event) {
            HoneypotDecision::Clean => Ok(CountermeasureDecision::Allowed),
            HoneypotDecision::Quarantine(hit) => {
                let receipt = self.revocations.revoke_subject(event.subject)?;
                Ok(CountermeasureDecision::Quarantined {
                    region: hit.region,
                    epoch: receipt.epoch,
                })
            }
        }
    }
}

impl<const REVOCATIONS: usize, const SUBJECTS: usize, const HONEYPOTS: usize> Default
    for ActiveCountermeasures<REVOCATIONS, SUBJECTS, HONEYPOTS>
{
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IncidentRequest {
    pub process: ProcessId,
    pub image: u128,
    pub detected_at_us: u64,
}

impl IncidentRequest {
    pub const fn new(process: ProcessId, image: u128, detected_at_us: u64) -> Self {
        Self {
            process,
            image,
            detected_at_us,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanWorkerRequest {
    pub incident: u64,
    pub compromised: ProcessId,
    pub image: u128,
    pub forensic_checkpoint: CheckpointInfo,
}

pub trait IncidentRuntime {
    type Error;

    fn freeze_process_tree(&mut self, process: ProcessId) -> Result<(), Self::Error>;

    fn spawn_clean_worker(
        &mut self,
        request: CleanWorkerRequest,
    ) -> Result<ProcessId, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IncidentState {
    Frozen,
    Replaced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IncidentRecord {
    pub id: u64,
    pub process: ProcessId,
    pub detected_at_us: u64,
    pub checkpoint: CheckpointInfo,
    pub replacement: Option<ProcessId>,
    pub state: IncidentState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IncidentReceipt {
    pub record: IncidentRecord,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseError {
    InvalidRequest,
    Capacity,
    Filesystem(SynFsError),
    FreezeFailed,
    RespawnFailed,
}

impl IntoStatus for ResponseError {
    fn status(self) -> Status {
        match self {
            Self::InvalidRequest => Status::INVALID_ARGUMENT,
            Self::Capacity => Status::NO_SPACE,
            Self::Filesystem(error) => error.status(),
            Self::FreezeFailed | Self::RespawnFailed => Status::BUSY,
        }
    }
}

/// Freezes a compromised process tree, pins the current GhostFS root for
/// forensics, and asks the supervisor to start a clean replacement.
pub struct IncidentResponder<const CAPACITY: usize = DEFAULT_INCIDENT_CAPACITY> {
    incidents: [Option<IncidentRecord>; CAPACITY],
    next_id: u64,
}

impl<const CAPACITY: usize> IncidentResponder<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            incidents: [None; CAPACITY],
            next_id: 1,
        }
    }

    pub fn respond<const BLOCKS: usize, R: IncidentRuntime>(
        &mut self,
        filesystem: &mut SynFs<BLOCKS>,
        runtime: &mut R,
        request: IncidentRequest,
    ) -> Result<IncidentReceipt, ResponseError> {
        if request.process.raw() == 0 || request.image == 0 || request.detected_at_us == 0 {
            return Err(ResponseError::InvalidRequest)
        }
        let slot = self
            .incidents
            .iter()
            .position(|entry| entry.is_none())
            .ok_or(ResponseError::Capacity)?;
        if CAPACITY == 0 {
            return Err(ResponseError::Capacity)
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(ResponseError::Capacity)?;
        let checkpoint = filesystem
            .create_checkpoint()
            .map_err(ResponseError::Filesystem)?;
        if runtime.freeze_process_tree(request.process).is_err() {
            let _ = filesystem.release_checkpoint(checkpoint.id);
            return Err(ResponseError::FreezeFailed)
        }
        let mut record = IncidentRecord {
            id,
            process: request.process,
            detected_at_us: request.detected_at_us,
            checkpoint,
            replacement: None,
            state: IncidentState::Frozen,
        };
        *self.incidents.get_mut(slot).expect("validated incident slot") = Some(record);

        let replacement = match runtime.spawn_clean_worker(CleanWorkerRequest {
            incident: id,
            compromised: request.process,
            image: request.image,
            forensic_checkpoint: checkpoint,
        }) {
            Ok(process) if process.raw() != 0 && process != request.process => process,
            _ => return Err(ResponseError::RespawnFailed),
        };
        record.replacement = Some(replacement);
        record.state = IncidentState::Replaced;
        *self.incidents.get_mut(slot).expect("validated incident slot") = Some(record);
        Ok(IncidentReceipt { record })
    }

    pub fn incident(&self, id: u64) -> Option<IncidentRecord> {
        self.incidents
            .iter()
            .flatten()
            .find(|entry| entry.id == id)
            .copied()
    }

    pub fn records(&self) -> impl Iterator<Item = IncidentRecord> + '_ {
        self.incidents.iter().flatten().copied()
    }
}

impl<const CAPACITY: usize> Default for IncidentResponder<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
