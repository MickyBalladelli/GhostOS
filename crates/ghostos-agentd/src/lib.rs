#![no_std]
#![forbid(unsafe_code)]

use ghostos_auth::{
    CapabilityKey, CapabilityLease, CryptographicCapability, LeaseContext, LeaseError, TokenError,
    TransportRights,
};
use ghostos_fabric::NodeId;
use ghostos_kernel::Rights;
use ghostos_observability::{JOURNAL_RECORD_SIZE, TraceEvent, encode_record};
use ghostos_status::{IntoStatus, Severity, Status, facility};
use ghostos_ghostfs::SynFs;

pub const DEFAULT_CONTEXT_CAPACITY: usize = 256;
pub const DEFAULT_QUEUE_CAPACITY: usize = 64;
pub const MAX_SOURCE_NAME_BYTES: usize = 192;
pub const MAX_CONTEXT_BYTES: usize = 4096;
pub const CONTEXT_RESOURCE: u64 = 0x5345_4d41_4e54_4943;

const READ_RIGHTS: Rights = Rights::READ;
const MAP_RIGHTS: Rights = Rights::READ.union(Rights::MAP);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentError {
    AccessDenied,
    BufferTooSmall { required: usize },
    Capacity,
    EmbeddingFailed,
    InvalidInput,
    InvalidToken,
    NotFound,
    QueueFull,
    StaleHandle,
}

impl From<TokenError> for AgentError {
    fn from(error: TokenError) -> Self {
        match error {
            TokenError::AccessDenied | TokenError::InvalidSignature => Self::AccessDenied,
            TokenError::CaveatCapacity | TokenError::Invalid | TokenError::RightsEscalation => {
                Self::InvalidToken
            }
        }
    }
}

impl From<LeaseError> for AgentError {
    fn from(error: LeaseError) -> Self {
        match error {
            LeaseError::Invalid | LeaseError::InvalidSignature => Self::InvalidToken,
            LeaseError::Expired
            | LeaseError::NotYetValid
            | LeaseError::Revoked
            | LeaseError::Replay
            | LeaseError::ReplayCapacity
            | LeaseError::SubjectMismatch
            | LeaseError::AudienceMismatch
            | LeaseError::ObjectMismatch
            | LeaseError::TenantMismatch
            | LeaseError::GenerationMismatch
            | LeaseError::PurposeMismatch
            | LeaseError::RightsDenied => Self::AccessDenied,
        }
    }
}

impl IntoStatus for AgentError {
    fn status(self) -> Status {
        match self {
            Self::AccessDenied | Self::InvalidToken => Status::ACCESS_DENIED,
            Self::BufferTooSmall { .. } | Self::Capacity | Self::QueueFull => Status::NO_SPACE,
            Self::NotFound | Self::StaleHandle => Status::NOT_FOUND,
            Self::EmbeddingFailed => {
                Status::new(Severity::Error, facility::LLM, 6, 0).expect("valid agent status")
            }
            Self::InvalidInput => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SourceKind {
    SynFsFile = 1,
    SystemLog = 2,
    KvState = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceRef {
    pub kind: SourceKind,
    pub id: u64,
}

impl SourceRef {
    pub const fn new(kind: SourceKind, id: u64) -> Result<Self, AgentError> {
        if id == 0 {
            Err(AgentError::InvalidInput)
        } else {
            Ok(Self { kind, id })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IngestOptions {
    pub owner: NodeId,
    pub lease_expires_at_us: u64,
    pub now_us: u64,
    pub ttl_us: Option<u64>,
    /// Context is removed after this much time without a retrieval. Zero
    /// disables inactivity collection while the process lease remains valid.
    pub decay_after_us: u64,
}

impl IngestOptions {
    pub const fn new(
        owner: NodeId,
        now_us: u64,
        lease_expires_at_us: u64,
    ) -> Result<Self, AgentError> {
        if lease_expires_at_us <= now_us {
            return Err(AgentError::InvalidInput);
        }
        Ok(Self {
            owner,
            lease_expires_at_us,
            now_us,
            ttl_us: None,
            decay_after_us: 0,
        })
    }

    pub const fn with_ttl(mut self, ttl_us: u64) -> Self {
        self.ttl_us = Some(ttl_us);
        self
    }

    pub const fn with_decay(mut self, decay_after_us: u64) -> Self {
        self.decay_after_us = decay_after_us;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Submission {
    pub source: SourceRef,
    pub queued: usize,
    pub replaced: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct PollReport {
    pub processed: usize,
    pub indexed: usize,
    pub replaced: usize,
    pub collected: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct GcReport {
    pub expired: usize,
    pub inactive: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ContextHandle(u64);

impl ContextHandle {
    const fn from_parts(slot: usize, generation: u32) -> Self {
        Self(((generation as u64) << 32) | slot as u64)
    }

    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw >> 32 == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    const fn slot(self) -> usize {
        self.0 as u32 as usize
    }

    const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContextHit {
    pub handle: ContextHandle,
    pub source: SourceRef,
    pub score: f32,
    pub age_us: u64,
}

/// Borrowed context returned from a mapped retrieval handle. The bytes and
/// embedding remain in the daemon-owned page; resolving a handle performs no
/// payload copy.
pub struct ContextView<'a> {
    pub source: SourceRef,
    pub owner: NodeId,
    pub name: &'a [u8],
    pub bytes: &'a [u8],
    pub embedding: &'a [f32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcceleratorKind {
    Cpu,
    Gpu,
    Npu,
}

/// Embedding work is deliberately separated from the bus. A Ring 3 NPU/GPU
/// service implements this trait and `poll` feeds it bounded, immutable input
/// pages. The CPU implementation is useful during boot and in small systems.
pub trait EmbeddingAccelerator<const DIMENSION: usize> {
    fn kind(&self) -> AcceleratorKind;

    fn embed(&mut self, input: &[u8], output: &mut [f32; DIMENSION]) -> Result<(), AgentError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuEmbedding;

impl<const DIMENSION: usize> EmbeddingAccelerator<DIMENSION> for CpuEmbedding {
    fn kind(&self) -> AcceleratorKind {
        AcceleratorKind::Cpu
    }

    fn embed(&mut self, input: &[u8], output: &mut [f32; DIMENSION]) -> Result<(), AgentError> {
        if DIMENSION == 0 {
            return Err(AgentError::InvalidInput);
        }
        output.fill(0.0);
        for (offset, byte) in input.iter().copied().enumerate() {
            let first = (offset.wrapping_mul(31).wrapping_add(byte as usize)) % DIMENSION;
            let second = (first + (byte as usize % DIMENSION)) % DIMENSION;
            output[first] += 1.0 + byte as f32 / 255.0;
            output[second] -= 0.25;
        }
        let squared_norm = output.iter().map(|value| value * value).sum::<f32>();
        let mut norm = squared_norm;
        if norm > 0.0 {
            for _ in 0..8 {
                norm = 0.5 * (norm + squared_norm / norm);
            }
        }
        if norm > 0.0 {
            for value in output.iter_mut() {
                *value /= norm;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Name {
    bytes: [u8; MAX_SOURCE_NAME_BYTES],
    len: u16,
}

impl Name {
    const EMPTY: Self = Self {
        bytes: [0; MAX_SOURCE_NAME_BYTES],
        len: 0,
    };

    fn from_bytes(bytes: &[u8]) -> Result<Self, AgentError> {
        if bytes.is_empty() || bytes.len() > MAX_SOURCE_NAME_BYTES || bytes.contains(&0) {
            return Err(AgentError::InvalidInput);
        }
        let mut name = Self::EMPTY;
        name.bytes[..bytes.len()].copy_from_slice(bytes);
        name.len = bytes.len() as u16;
        Ok(name)
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Clone, Copy)]
struct Pending<const CONTENT_BYTES: usize> {
    source: SourceRef,
    name: Name,
    owner: NodeId,
    lease_expires_at_us: u64,
    expires_at_us: u64,
    decay_after_us: u64,
    created_at_us: u64,
    bytes: [u8; CONTENT_BYTES],
    len: usize,
}

#[derive(Clone, Copy)]
struct Entry<const DIMENSION: usize, const CONTENT_BYTES: usize> {
    occupied: bool,
    generation: u32,
    source: SourceRef,
    name: Name,
    owner: NodeId,
    lease_expires_at_us: u64,
    expires_at_us: u64,
    decay_after_us: u64,
    created_at_us: u64,
    last_access_us: u64,
    bytes: [u8; CONTENT_BYTES],
    len: usize,
    embedding: [f32; DIMENSION],
}

impl<const DIMENSION: usize, const CONTENT_BYTES: usize> Entry<DIMENSION, CONTENT_BYTES> {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        source: SourceRef {
            kind: SourceKind::SystemLog,
            id: 1,
        },
        name: Name::EMPTY,
        owner: NodeId::LOCAL,
        lease_expires_at_us: 0,
        expires_at_us: 0,
        decay_after_us: 0,
        created_at_us: 0,
        last_access_us: 0,
        bytes: [0; CONTENT_BYTES],
        len: 0,
        embedding: [0.0; DIMENSION],
    };
}

/// Fixed-capacity semantic memory and capability-authenticated context bus.
///
/// Producers enqueue immutable source pages. `poll` runs embedding work on a
/// CPU, GPU, or NPU backend and publishes a new generation atomically from a
/// caller's perspective. Retrieval returns handles first, then borrowed views,
/// so agents can map context without a second payload allocation or copy.
pub struct ContextBus<
    const DIMENSION: usize = 32,
    const CONTEXTS: usize = DEFAULT_CONTEXT_CAPACITY,
    const QUEUE: usize = DEFAULT_QUEUE_CAPACITY,
    const CONTENT_BYTES: usize = MAX_CONTEXT_BYTES,
> {
    entries: [Entry<DIMENSION, CONTENT_BYTES>; CONTEXTS],
    queue: [Option<Pending<CONTENT_BYTES>>; QUEUE],
    key: CapabilityKey,
    issuer: NodeId,
    resource: u64,
    revocation_epoch: u64,
    next_nonce: u64,
}

impl<const DIMENSION: usize, const CONTEXTS: usize, const QUEUE: usize, const CONTENT_BYTES: usize>
    ContextBus<DIMENSION, CONTEXTS, QUEUE, CONTENT_BYTES>
{
    pub fn new(issuer: NodeId, key: CapabilityKey, resource: u64) -> Result<Self, AgentError> {
        if resource == 0 || DIMENSION == 0 || CONTEXTS == 0 || QUEUE == 0 || CONTENT_BYTES == 0 {
            return Err(AgentError::InvalidInput);
        }
        Ok(Self {
            entries: [Entry::EMPTY; CONTEXTS],
            queue: [None; QUEUE],
            key,
            issuer,
            resource,
            revocation_epoch: 1,
            next_nonce: 1,
        })
    }

    pub const fn resource(&self) -> u64 {
        self.resource
    }

    pub const fn revocation_epoch(&self) -> u64 {
        self.revocation_epoch
    }

    pub fn issue_capability(
        &mut self,
        subject: NodeId,
        rights: Rights,
        transports: TransportRights,
        now_us: u64,
        lifetime_us: u64,
    ) -> Result<CryptographicCapability, AgentError> {
        if rights.is_empty() || transports.bits() == 0 || lifetime_us == 0 {
            return Err(AgentError::InvalidInput);
        }
        let expires_at_us = now_us
            .checked_add(lifetime_us)
            .ok_or(AgentError::InvalidInput)?;
        self.next_nonce = self.next_nonce.wrapping_add(1).max(1);
        CryptographicCapability::issue(
            self.key,
            self.issuer,
            subject,
            self.resource,
            rights,
            transports,
            now_us,
            expires_at_us,
            self.revocation_epoch,
            self.next_nonce,
        )
        .map_err(AgentError::from)
    }

    pub fn issue_lease(
        &mut self,
        subject: NodeId,
        tenant: u64,
        generation: u64,
        purpose: u64,
        rights: Rights,
        now_us: u64,
        lifetime_us: u64,
    ) -> Result<CapabilityLease, AgentError> {
        let expires_at_us = now_us
            .checked_add(lifetime_us)
            .ok_or(AgentError::InvalidInput)?;
        self.next_nonce = self.next_nonce.wrapping_add(1).max(1);
        CapabilityLease::issue(
            self.key,
            self.issuer,
            subject,
            self.issuer,
            self.resource,
            tenant,
            generation,
            purpose,
            rights,
            now_us,
            expires_at_us,
            self.revocation_epoch,
            self.next_nonce,
        )
        .map_err(AgentError::from)
    }

    pub fn revoke_all(&mut self) {
        self.revocation_epoch = self.revocation_epoch.wrapping_add(1).max(1);
    }

    pub fn open_channel(
        &self,
        token: CryptographicCapability,
        subject: NodeId,
        transport: TransportRights,
        now_us: u64,
    ) -> Result<ContextChannel, AgentError> {
        self.authorize(&token, subject, READ_RIGHTS, transport, now_us)?;
        Ok(ContextChannel {
            token,
            subject,
            transport,
            lease: None,
        })
    }

    pub fn open_channel_with_lease(
        &self,
        token: CryptographicCapability,
        lease: CapabilityLease,
        subject: NodeId,
        transport: TransportRights,
        tenant: u64,
        generation: u64,
        purpose: u64,
        now_us: u64,
    ) -> Result<ContextChannel, AgentError> {
        self.authorize(&token, subject, READ_RIGHTS, transport, now_us)?;
        lease
            .authorize(
                self.key,
                LeaseContext {
                    subject,
                    audience: self.issuer,
                    object: self.resource,
                    tenant,
                    generation,
                    purpose,
                    required: READ_RIGHTS,
                    now_us,
                },
                self.revocation_epoch,
            )
            .map_err(AgentError::from)?;
        Ok(ContextChannel {
            token,
            subject,
            transport,
            lease: Some((lease, tenant, generation, purpose)),
        })
    }

    pub fn submit(
        &mut self,
        source: SourceRef,
        name: &[u8],
        bytes: &[u8],
        options: IngestOptions,
    ) -> Result<Submission, AgentError> {
        let name = Name::from_bytes(name)?;
        if bytes.len() > CONTENT_BYTES {
            return Err(AgentError::InvalidInput);
        }
        let mut expires_at_us = match options.ttl_us {
            Some(ttl) => options
                .now_us
                .checked_add(ttl)
                .ok_or(AgentError::InvalidInput)?,
            None => 0,
        };
        if expires_at_us != 0 && expires_at_us <= options.now_us {
            return Err(AgentError::InvalidInput);
        }
        if expires_at_us == 0 {
            expires_at_us = options.lease_expires_at_us;
        } else {
            expires_at_us = expires_at_us.min(options.lease_expires_at_us);
        }
        let mut pending = Pending {
            source,
            name,
            owner: options.owner,
            lease_expires_at_us: options.lease_expires_at_us,
            expires_at_us,
            decay_after_us: options.decay_after_us,
            created_at_us: options.now_us,
            bytes: [0; CONTENT_BYTES],
            len: bytes.len(),
        };
        pending.bytes[..bytes.len()].copy_from_slice(bytes);

        let existing = self
            .queue
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.source == source));
        let slot = existing
            .or_else(|| self.queue.iter().position(Option::is_none))
            .ok_or(AgentError::QueueFull)?;
        let replaced = existing.is_some();
        self.queue[slot] = Some(pending);
        let queued = self.queue.iter().flatten().count();
        Ok(Submission {
            source,
            queued,
            replaced,
        })
    }

    pub fn submit_ghostfs_file<const BLOCKS: usize>(
        &mut self,
        filesystem: &SynFs<BLOCKS>,
        path: &str,
        options: IngestOptions,
    ) -> Result<Submission, AgentError> {
        let file = filesystem.lookup(path).map_err(|_| AgentError::NotFound)?;
        if file.size as usize > CONTENT_BYTES {
            return Err(AgentError::BufferTooSmall {
                required: file.size as usize,
            });
        }
        let mut bytes = [0; CONTENT_BYTES];
        let read = filesystem
            .read(path, &mut bytes)
            .map_err(|_| AgentError::NotFound)?;
        let copied = read.bytes_read;
        let source = SourceRef::new(SourceKind::SynFsFile, hash(file.file.as_bytes(), 0))?;
        self.submit(source, file.file.as_bytes(), &bytes[..copied], options)
    }

    pub fn submit_log(
        &mut self,
        event: TraceEvent,
        options: IngestOptions,
    ) -> Result<Submission, AgentError> {
        let mut bytes = [0; JOURNAL_RECORD_SIZE];
        encode_record(event, &mut bytes).map_err(|_| AgentError::EmbeddingFailed)?;
        let source = SourceRef::new(SourceKind::SystemLog, hash(&bytes, event.timestamp))?;
        self.submit(source, b"SYS$LOG:SYSTEM.JOURNAL", &bytes, options)
    }

    pub fn submit_kv(
        &mut self,
        key: &[u8],
        value: &[u8],
        options: IngestOptions,
    ) -> Result<Submission, AgentError> {
        if key.is_empty() || key.len().saturating_add(value.len()).saturating_add(1) > CONTENT_BYTES
        {
            return Err(AgentError::BufferTooSmall {
                required: key.len().saturating_add(value.len()).saturating_add(1),
            });
        }
        let mut bytes = [0; CONTENT_BYTES];
        bytes[..key.len()].copy_from_slice(key);
        bytes[key.len()] = b'=';
        bytes[key.len() + 1..key.len() + 1 + value.len()].copy_from_slice(value);
        let length = key.len() + 1 + value.len();
        let source = SourceRef::new(SourceKind::KvState, hash(key, 0))?;
        self.submit(source, key, &bytes[..length], options)
    }

    pub fn poll<A: EmbeddingAccelerator<DIMENSION>>(
        &mut self,
        accelerator: &mut A,
        budget: usize,
        now_us: u64,
    ) -> Result<PollReport, AgentError> {
        let mut report = PollReport {
            collected: self.collect_expired(now_us, budget),
            ..PollReport::default()
        };
        while report.processed < budget {
            let Some(slot) = self.queue.iter().position(Option::is_some) else {
                break;
            };
            let pending = self.queue[slot].ok_or(AgentError::QueueFull)?;
            let entry_slot = self
                .entries
                .iter()
                .position(|entry| entry.occupied && entry.source == pending.source)
                .or_else(|| self.entries.iter().position(|entry| !entry.occupied))
                .ok_or(AgentError::Capacity)?;
            let replaced = self.entries[entry_slot].occupied;
            let generation = self.entries[entry_slot].generation.wrapping_add(1).max(1);
            let mut embedding = [0.0; DIMENSION];
            accelerator.embed(&pending.bytes[..pending.len], &mut embedding)?;
            self.queue[slot] = None;
            self.entries[entry_slot] = Entry {
                occupied: true,
                generation,
                source: pending.source,
                name: pending.name,
                owner: pending.owner,
                lease_expires_at_us: pending.lease_expires_at_us,
                expires_at_us: pending.expires_at_us,
                decay_after_us: pending.decay_after_us,
                created_at_us: pending.created_at_us,
                last_access_us: now_us,
                bytes: pending.bytes,
                len: pending.len,
                embedding,
            };
            report.processed += 1;
            report.indexed += 1;
            report.replaced += replaced as usize;
        }
        Ok(report)
    }

    pub fn collect_expired(&mut self, now_us: u64, budget: usize) -> usize {
        let mut removed = 0;
        for entry in &mut self.entries {
            if removed == budget {
                break;
            }
            let expired = entry.occupied
                && (entry.lease_expires_at_us <= now_us
                    || entry.expires_at_us != 0 && entry.expires_at_us <= now_us);
            let inactive = entry.occupied
                && entry.decay_after_us != 0
                && now_us.saturating_sub(entry.last_access_us) >= entry.decay_after_us;
            if expired || inactive {
                entry.occupied = false;
                removed += 1;
            }
        }
        for pending in self.queue.iter_mut() {
            if removed == budget {
                break;
            }
            if pending.is_some_and(|pending| {
                pending.lease_expires_at_us <= now_us
                    || pending.expires_at_us != 0 && pending.expires_at_us <= now_us
            }) {
                *pending = None;
                removed += 1;
            }
        }
        removed
    }

    pub fn renew_process(&mut self, owner: NodeId, lease_expires_at_us: u64, now_us: u64) -> usize {
        if lease_expires_at_us <= now_us {
            return 0;
        }
        let mut renewed = 0;
        for entry in &mut self.entries {
            if entry.occupied && entry.owner == owner && entry.lease_expires_at_us > now_us {
                entry.lease_expires_at_us = lease_expires_at_us;
                renewed += 1;
            }
        }
        for pending in self.queue.iter_mut().flatten() {
            if pending.owner == owner && pending.lease_expires_at_us > now_us {
                pending.lease_expires_at_us = lease_expires_at_us;
            }
        }
        renewed
    }

    pub fn revoke_process(&mut self, owner: NodeId) -> usize {
        let mut removed = 0;
        for entry in &mut self.entries {
            if entry.occupied && entry.owner == owner {
                entry.occupied = false;
                removed += 1;
            }
        }
        for pending in self.queue.iter_mut() {
            if pending.is_some_and(|pending| pending.owner == owner) {
                *pending = None;
            }
        }
        removed
    }

    pub fn query<const RESULTS: usize>(
        &mut self,
        token: &CryptographicCapability,
        subject: NodeId,
        transport: TransportRights,
        query: &[f32; DIMENSION],
        limit: usize,
        now_us: u64,
        output: &mut [Option<ContextHit>; RESULTS],
    ) -> Result<usize, AgentError> {
        self.authorize(token, subject, READ_RIGHTS, transport, now_us)?;
        output.fill(None);
        let limit = limit.min(RESULTS);
        let mut written = 0;
        for slot in 0..CONTEXTS {
            let entry = &self.entries[slot];
            if !entry.occupied
                || entry.lease_expires_at_us <= now_us
                || entry.expires_at_us != 0 && entry.expires_at_us <= now_us
            {
                continue;
            }
            let freshness = freshness(entry, now_us);
            let score = dot(query, &entry.embedding) * freshness;
            let hit = ContextHit {
                handle: ContextHandle::from_parts(slot, entry.generation),
                source: entry.source,
                score,
                age_us: now_us.saturating_sub(entry.created_at_us),
            };
            let insert_at = output[..written]
                .iter()
                .position(|current| current.is_some_and(|current| current.score < score))
                .unwrap_or(written);
            if insert_at >= limit {
                continue;
            }
            if written < limit {
                written += 1
            }
            for index in (insert_at..written.saturating_sub(1)).rev() {
                output[index + 1] = output[index];
            }
            output[insert_at] = Some(hit);
            self.entries[slot].last_access_us = now_us;
        }
        Ok(written)
    }

    pub fn resolve<'a>(
        &'a self,
        token: &CryptographicCapability,
        subject: NodeId,
        transport: TransportRights,
        handle: ContextHandle,
        now_us: u64,
    ) -> Result<ContextView<'a>, AgentError> {
        self.authorize(token, subject, MAP_RIGHTS, transport, now_us)?;
        let entry = self
            .entries
            .get(handle.slot())
            .ok_or(AgentError::StaleHandle)?;
        if !entry.occupied
            || entry.generation != handle.generation()
            || entry.lease_expires_at_us <= now_us
            || entry.expires_at_us != 0 && entry.expires_at_us <= now_us
        {
            return Err(AgentError::StaleHandle);
        }
        Ok(ContextView {
            source: entry.source,
            owner: entry.owner,
            name: entry.name.as_bytes(),
            bytes: &entry.bytes[..entry.len],
            embedding: &entry.embedding,
        })
    }

    pub fn query_channel<const RESULTS: usize>(
        &mut self,
        channel: &ContextChannel,
        query: &[f32; DIMENSION],
        limit: usize,
        now_us: u64,
        output: &mut [Option<ContextHit>; RESULTS],
    ) -> Result<usize, AgentError> {
        self.authorize_channel(channel, READ_RIGHTS, now_us)?;
        self.query(
            &channel.token,
            channel.subject,
            channel.transport,
            query,
            limit,
            now_us,
            output,
        )
    }

    pub fn resolve_channel<'a>(
        &'a self,
        channel: &ContextChannel,
        handle: ContextHandle,
        now_us: u64,
    ) -> Result<ContextView<'a>, AgentError> {
        self.authorize_channel(channel, MAP_RIGHTS, now_us)?;
        self.resolve(
            &channel.token,
            channel.subject,
            channel.transport,
            handle,
            now_us,
        )
    }

    fn authorize_channel(
        &self,
        channel: &ContextChannel,
        required: Rights,
        now_us: u64,
    ) -> Result<(), AgentError> {
        let Some((lease, tenant, generation, purpose)) = channel.lease else {
            return Ok(())
        };
        lease
            .authorize(
                self.key,
                LeaseContext {
                    subject: channel.subject,
                    audience: self.issuer,
                    object: self.resource,
                    tenant,
                    generation,
                    purpose,
                    required,
                    now_us,
                },
                self.revocation_epoch,
            )
            .map_err(AgentError::from)
    }

    fn authorize(
        &self,
        token: &CryptographicCapability,
        subject: NodeId,
        required: Rights,
        transport: TransportRights,
        now_us: u64,
    ) -> Result<(), AgentError> {
        if token.resource != self.resource {
            return Err(AgentError::AccessDenied);
        }
        token
            .verify(
                self.key,
                subject,
                required,
                transport,
                now_us,
                self.revocation_epoch,
            )
            .map_err(AgentError::from)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextChannel {
    token: CryptographicCapability,
    subject: NodeId,
    transport: TransportRights,
    lease: Option<(CapabilityLease, u64, u64, u64)>,
}

impl ContextChannel {
    pub const fn subject(&self) -> NodeId {
        self.subject
    }

    pub const fn transport(&self) -> TransportRights {
        self.transport
    }
}

fn hash(bytes: &[u8], salt: u64) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ salt;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash.max(1)
}

fn dot<const DIMENSION: usize>(left: &[f32; DIMENSION], right: &[f32; DIMENSION]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn freshness<const DIMENSION: usize, const CONTENT_BYTES: usize>(
    entry: &Entry<DIMENSION, CONTENT_BYTES>,
    now_us: u64,
) -> f32 {
    if entry.decay_after_us == 0 {
        return 1.0;
    }
    let age = now_us.saturating_sub(entry.last_access_us);
    entry.decay_after_us.saturating_sub(age) as f32 / entry.decay_after_us as f32
}
