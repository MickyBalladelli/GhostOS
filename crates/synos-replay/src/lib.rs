#![no_std]
#![forbid(unsafe_code)]

//! Bounded primitives for deterministic execution and post-mortem replay.
//!
//! Producers record only fixed-size values into a lock-free overwrite ring.
//! A replay consumer then reads the same values in sequence, while callers
//! provide process snapshots to [`TimeTravel`] for reverse debugging.

use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use synos_init::{CrashReason, ProcessId};
use synos_status::{IntoStatus, Status};
use synos_synfs::{Error as FileError, FileType, SynFs};

mod bundle;

pub use bundle::{
    public_input_digest, ReplayBundle, ReplayBundleEvent, ReplayBundleEventKind,
    ReplayBundleReader, ReplaySensitivity, REPLAY_BUNDLE_CONFIG_DIGEST_BYTES,
    REPLAY_BUNDLE_EVENT_BYTES, REPLAY_BUNDLE_FORMAT_VERSION, REPLAY_BUNDLE_HEADER_BYTES,
    REPLAY_BUNDLE_MAX_EVENTS, REPLAY_BUNDLE_MAX_PAYLOAD_BYTES,
};

pub const REPLAY_FORMAT_VERSION: u16 = 1;
pub const EVENT_BYTES: usize = 56;
pub const MAX_REPLAY_PATH_BYTES: usize = 192;

const EVENT_MAGIC: [u8; 8] = *b"SYNREP01";
const META_BYTES: usize = 40;
const SLOT_BUSY: u64 = 1_u64 << 63;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayError {
    Inactive,
    BufferTooSmall { required: usize },
    Capacity,
    Corrupt,
    NoEvent,
    UnexpectedEvent,
    InputMismatch,
    SecretExcluded,
    UnsupportedVersion,
    InvalidPath,
    Storage(FileError),
}

impl From<FileError> for ReplayError {
    fn from(error: FileError) -> Self {
        Self::Storage(error)
    }
}

impl IntoStatus for ReplayError {
    fn status(self) -> Status {
        match self {
            Self::Inactive | Self::NoEvent => Status::PENDING,
            Self::BufferTooSmall { .. } | Self::Capacity => Status::NO_SPACE,
            Self::Corrupt | Self::UnexpectedEvent | Self::InputMismatch | Self::UnsupportedVersion => Status::CORRUPT,
            Self::SecretExcluded | Self::InvalidPath => Status::INVALID_ARGUMENT,
            Self::Storage(error) => error.status(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ReplayMode {
    Disabled = 0,
    Recording = 1,
    Replaying = 2,
}

impl ReplayMode {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Recording,
            2 => Self::Replaying,
            _ => Self::Disabled,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ReplayEventKind {
    Timing = 1,
    NetworkInterrupt = 2,
    CxlMemoryAccess = 3,
}

impl ReplayEventKind {
    fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Timing),
            2 => Some(Self::NetworkInterrupt),
            3 => Some(Self::CxlMemoryAccess),
            _ => None,
        }
    }
}

/// One nondeterministic input. Values are deliberately generic so the same
/// record format works for kernel timing, interrupt, and CXL paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayEvent {
    pub sequence: u64,
    pub timestamp_us: u64,
    pub kind: ReplayEventKind,
    pub source: u64,
    pub value0: u64,
    pub value1: u64,
    pub value2: u64,
}

impl ReplayEvent {
    pub const EMPTY: Self = Self {
        sequence: 0,
        timestamp_us: 0,
        kind: ReplayEventKind::Timing,
        source: 0,
        value0: 0,
        value1: 0,
        value2: 0,
    };

    pub const fn timing(timestamp_us: u64, cpu: u64, elapsed_us: u64, deadline_us: u64) -> Self {
        Self {
            sequence: 0,
            timestamp_us,
            kind: ReplayEventKind::Timing,
            source: cpu,
            value0: elapsed_us,
            value1: deadline_us,
            value2: 0,
        }
    }

    pub const fn network_interrupt(
        timestamp_us: u64,
        vector: u64,
        line: u64,
        packet_hash: u64,
    ) -> Self {
        Self {
            sequence: 0,
            timestamp_us,
            kind: ReplayEventKind::NetworkInterrupt,
            source: vector,
            value0: line,
            value1: packet_hash,
            value2: 0,
        }
    }

    pub const fn cxl_memory_access(
        timestamp_us: u64,
        node: u64,
        address: u64,
        latency_ns: u64,
        variation_ns: u64,
    ) -> Self {
        Self {
            sequence: 0,
            timestamp_us,
            kind: ReplayEventKind::CxlMemoryAccess,
            source: node,
            value0: address,
            value1: latency_ns,
            value2: variation_ns,
        }
    }

    pub fn encode(self, output: &mut [u8]) -> Result<(), ReplayError> {
        if output.len() < EVENT_BYTES {
            return Err(ReplayError::BufferTooSmall {
                required: EVENT_BYTES,
            });
        }
        output[..8].copy_from_slice(&self.sequence.to_le_bytes());
        output[8..16].copy_from_slice(&self.timestamp_us.to_le_bytes());
        output[16] = self.kind as u8;
        output[17..24].fill(0);
        output[24..32].copy_from_slice(&self.source.to_le_bytes());
        output[32..40].copy_from_slice(&self.value0.to_le_bytes());
        output[40..48].copy_from_slice(&self.value1.to_le_bytes());
        output[48..56].copy_from_slice(&self.value2.to_le_bytes());
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<Self, ReplayError> {
        if input.len() < EVENT_BYTES {
            return Err(ReplayError::BufferTooSmall {
                required: EVENT_BYTES,
            });
        }
        let kind = ReplayEventKind::from_raw(input[16]).ok_or(ReplayError::Corrupt)?;
        Ok(Self {
            sequence: read_u64(&input[..8]),
            timestamp_us: read_u64(&input[8..16]),
            kind,
            source: read_u64(&input[24..32]),
            value0: read_u64(&input[32..40]),
            value1: read_u64(&input[40..48]),
            value2: read_u64(&input[48..56]),
        })
    }
}

struct AtomicReplayEvent {
    timestamp_us: AtomicU64,
    kind: AtomicU8,
    source: AtomicU64,
    value0: AtomicU64,
    value1: AtomicU64,
    value2: AtomicU64,
}

impl AtomicReplayEvent {
    const fn new() -> Self {
        Self {
            timestamp_us: AtomicU64::new(0),
            kind: AtomicU8::new(0),
            source: AtomicU64::new(0),
            value0: AtomicU64::new(0),
            value1: AtomicU64::new(0),
            value2: AtomicU64::new(0),
        }
    }

    fn write(&self, event: ReplayEvent) {
        self.timestamp_us
            .store(event.timestamp_us, Ordering::Relaxed);
        self.kind.store(event.kind as u8, Ordering::Relaxed);
        self.source.store(event.source, Ordering::Relaxed);
        self.value0.store(event.value0, Ordering::Relaxed);
        self.value1.store(event.value1, Ordering::Relaxed);
        self.value2.store(event.value2, Ordering::Relaxed);
    }

    fn read(&self, sequence: u64) -> Option<ReplayEvent> {
        Some(ReplayEvent {
            sequence: sequence.saturating_add(1),
            timestamp_us: self.timestamp_us.load(Ordering::Relaxed),
            kind: ReplayEventKind::from_raw(self.kind.load(Ordering::Relaxed))?,
            source: self.source.load(Ordering::Relaxed),
            value0: self.value0.load(Ordering::Relaxed),
            value1: self.value1.load(Ordering::Relaxed),
            value2: self.value2.load(Ordering::Relaxed),
        })
    }
}

struct ReplaySlot {
    published: AtomicU64,
    event: AtomicReplayEvent,
}

impl ReplaySlot {
    const fn new() -> Self {
        Self {
            published: AtomicU64::new(u64::MAX),
            event: AtomicReplayEvent::new(),
        }
    }
}

/// Lock-free, fixed-size MPMC ring. Oldest entries are overwritten when a
/// consumer falls behind, keeping interrupt paths bounded and non-blocking.
pub struct ReplayRing<const CAPACITY: usize> {
    write_position: AtomicU64,
    read_position: AtomicU64,
    dropped: AtomicU64,
    slots: [ReplaySlot; CAPACITY],
}

impl<const CAPACITY: usize> ReplayRing<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY >= 2);
        Self {
            write_position: AtomicU64::new(0),
            read_position: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            slots: [const { ReplaySlot::new() }; CAPACITY],
        }
    }

    pub fn push(&self, event: ReplayEvent) -> u64 {
        let ticket = self.write_position.fetch_add(1, Ordering::AcqRel);
        let minimum = ticket.saturating_add(1).saturating_sub(CAPACITY as u64);
        let mut read = self.read_position.load(Ordering::Acquire);
        while read < minimum {
            match self.read_position.compare_exchange_weak(
                read,
                minimum,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.dropped.fetch_add(minimum - read, Ordering::Relaxed);
                    break;
                }
                Err(observed) => read = observed,
            }
        }

        let slot = &self.slots[ticket as usize % CAPACITY];
        slot.published.store(ticket | SLOT_BUSY, Ordering::Release);
        slot.event.write(event);
        slot.published.store(ticket, Ordering::Release);
        ticket.saturating_add(1)
    }

    pub fn try_pop(&self) -> Option<ReplayEvent> {
        loop {
            let position = self.read_position.load(Ordering::Acquire);
            if position >= self.write_position.load(Ordering::Acquire) {
                return None;
            }
            let slot = &self.slots[position as usize % CAPACITY];
            let published = slot.published.load(Ordering::Acquire);
            if published & SLOT_BUSY != 0 {
                return None;
            }
            if published != position {
                if published > position {
                    let _ = self.read_position.compare_exchange_weak(
                        position,
                        published,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    continue;
                }
                return None;
            }
            if self
                .read_position
                .compare_exchange_weak(position, position + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                continue;
            }
            let event = slot.event.read(position);
            if slot.published.load(Ordering::Acquire) == position {
                return event;
            }
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn pending(&self) -> usize {
        self.write_position
            .load(Ordering::Acquire)
            .saturating_sub(self.read_position.load(Ordering::Acquire))
            .min(CAPACITY as u64) as usize
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Acquire)
    }

    pub fn clear(&self) {
        self.read_position.store(
            self.write_position.load(Ordering::Acquire),
            Ordering::Release,
        );
    }

    pub fn snapshot_into(&self, output: &mut [ReplayEvent]) -> Result<usize, ReplayError> {
        let start = self.read_position.load(Ordering::Acquire);
        let end = self.write_position.load(Ordering::Acquire);
        let count = end.saturating_sub(start).min(CAPACITY as u64) as usize;
        if output.len() < count {
            return Err(ReplayError::BufferTooSmall { required: count });
        }
        let first = end.saturating_sub(count as u64);
        let mut written = 0;
        for position in first..end {
            let slot = &self.slots[position as usize % CAPACITY];
            if slot.published.load(Ordering::Acquire) != position {
                continue;
            }
            if let Some(event) = slot.event.read(position) {
                if slot.published.load(Ordering::Acquire) == position {
                    output[written] = event;
                    written += 1;
                }
            }
        }
        Ok(written)
    }
}

impl<const CAPACITY: usize> Default for ReplayRing<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared logger used by kernel timing and device interrupt paths.
pub struct ReplayLog<const CAPACITY: usize> {
    ring: ReplayRing<CAPACITY>,
    mode: AtomicU8,
}

impl<const CAPACITY: usize> ReplayLog<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            ring: ReplayRing::new(),
            mode: AtomicU8::new(ReplayMode::Disabled as u8),
        }
    }

    pub fn mode(&self) -> ReplayMode {
        ReplayMode::from_raw(self.mode.load(Ordering::Acquire))
    }

    pub fn begin_recording(&self) {
        self.ring.clear();
        self.mode
            .store(ReplayMode::Recording as u8, Ordering::Release);
    }

    pub fn begin_replay(&self) {
        self.mode
            .store(ReplayMode::Replaying as u8, Ordering::Release);
    }

    pub fn stop(&self) {
        self.mode
            .store(ReplayMode::Disabled as u8, Ordering::Release);
    }

    pub fn record(&self, event: ReplayEvent) -> Result<u64, ReplayError> {
        if self.mode() != ReplayMode::Recording {
            return Err(ReplayError::Inactive);
        }
        Ok(self.ring.push(event))
    }

    pub fn next(&self, expected: ReplayEventKind) -> Result<ReplayEvent, ReplayError> {
        if self.mode() != ReplayMode::Replaying {
            return Err(ReplayError::Inactive);
        }
        let event = self.ring.try_pop().ok_or(ReplayError::NoEvent)?;
        if event.kind != expected {
            return Err(ReplayError::UnexpectedEvent);
        }
        Ok(event)
    }

    pub fn try_next(&self) -> Option<ReplayEvent> {
        (self.mode() == ReplayMode::Replaying)
            .then(|| self.ring.try_pop())
            .flatten()
    }

    pub fn pending(&self) -> usize {
        self.ring.pending()
    }

    pub fn dropped(&self) -> u64 {
        self.ring.dropped()
    }

    pub fn snapshot_into(&self, output: &mut [ReplayEvent]) -> Result<usize, ReplayError> {
        self.ring.snapshot_into(output)
    }

    pub fn preserve_on_crash<const MAX_BLOCKS: usize>(
        &self,
        filesystem: &mut SynFs<MAX_BLOCKS>,
        request: FlightRecorderRequest<'_>,
    ) -> Result<FlightRecorderReceipt, ReplayError> {
        let directory = ReplayPath::new(request.directory)?;
        let mut events = [ReplayEvent::EMPTY; CAPACITY];
        let event_count = self.snapshot_into(&mut events)?;
        let mut metadata = [0; META_BYTES];
        metadata[..8].copy_from_slice(&EVENT_MAGIC);
        metadata[8..10].copy_from_slice(&REPLAY_FORMAT_VERSION.to_le_bytes());
        metadata[10] = crash_reason(request.reason);
        metadata[11] = self.mode() as u8;
        metadata[12..20].copy_from_slice(&request.process.raw().to_le_bytes());
        metadata[20..28].copy_from_slice(&request.timestamp_us.to_le_bytes());
        metadata[28..32].copy_from_slice(&(event_count as u32).to_le_bytes());
        metadata[32..40].copy_from_slice(&self.dropped().to_le_bytes());

        let mut transaction = filesystem.transaction();
        match transaction.lookup(directory.as_str()) {
            Ok(file) if file.file_type == FileType::Directory => {}
            Ok(_) => return Err(ReplayError::InvalidPath),
            Err(FileError::NotFound) => {
                transaction.create_directory(directory.as_str(), true)?;
            }
            Err(error) => return Err(error.into()),
        }
        transaction.write(directory.child("META")?.as_str(), &metadata)?;

        let mut bytes = 0_u64;
        let mut encoded = [0; EVENT_BYTES];
        for (index, event) in events.iter().take(event_count).copied().enumerate() {
            event.encode(&mut encoded)?;
            transaction.write(directory.event(index)?.as_str(), &encoded)?;
            bytes = bytes.saturating_add(EVENT_BYTES as u64);
        }
        let commit = transaction.commit()?;
        Ok(FlightRecorderReceipt {
            process: request.process,
            directory,
            generation: commit.generation,
            events: event_count as u32,
            bytes: bytes.saturating_add(META_BYTES as u64),
            dropped: self.dropped(),
        })
    }
}

impl<const CAPACITY: usize> Default for ReplayLog<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlightRecorderRequest<'a> {
    pub process: ProcessId,
    pub reason: CrashReason,
    pub timestamp_us: u64,
    pub directory: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlightRecorderReceipt {
    pub process: ProcessId,
    pub directory: ReplayPath,
    pub generation: u64,
    pub events: u32,
    pub bytes: u64,
    pub dropped: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayPath {
    bytes: [u8; MAX_REPLAY_PATH_BYTES],
    len: u16,
}

impl ReplayPath {
    pub fn new(path: &str) -> Result<Self, ReplayError> {
        let bytes = path.as_bytes();
        if bytes.is_empty()
            || bytes.len() > MAX_REPLAY_PATH_BYTES
            || bytes.contains(&0)
            || bytes.ends_with(b"/")
            || bytes.windows(2).any(|pair| pair == b"//")
            || bytes
                .split(|byte| *byte == b'/')
                .any(|component| component == b"." || component == b"..")
        {
            return Err(ReplayError::InvalidPath);
        }
        let mut result = Self {
            bytes: [0; MAX_REPLAY_PATH_BYTES],
            len: bytes.len() as u16,
        };
        result.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(result)
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).expect("replay path invariant")
    }

    fn child(&self, suffix: &str) -> Result<Self, ReplayError> {
        let required = self.len as usize + 1 + suffix.len();
        if required > MAX_REPLAY_PATH_BYTES {
            return Err(ReplayError::BufferTooSmall { required });
        }
        let mut result = *self;
        result.bytes[self.len as usize] = b'/';
        result.bytes[self.len as usize + 1..required].copy_from_slice(suffix.as_bytes());
        result.len = required as u16;
        Ok(result)
    }

    fn event(&self, index: usize) -> Result<Self, ReplayError> {
        if index > 99_999_999 {
            return Err(ReplayError::Capacity);
        }
        let mut suffix = [0; 14];
        suffix[..6].copy_from_slice(b"EVENT-");
        let mut value = index as u64;
        for position in 0..8 {
            suffix[13 - position] = b'0' + (value % 10) as u8;
            value /= 10;
        }
        let suffix = core::str::from_utf8(&suffix).expect("static replay path suffix");
        self.child(suffix)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointId(u64);

impl CheckpointId {
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointInfo {
    pub id: CheckpointId,
    pub event_sequence: u64,
}

struct StoredCheckpoint<S> {
    info: CheckpointInfo,
    state: S,
}

/// Bounded state history for reverse debugging. The runtime owns the state
/// type; restoring a checkpoint returns a clone for the runtime to install.
pub struct TimeTravel<S, const MAX_CHECKPOINTS: usize> {
    checkpoints: [Option<StoredCheckpoint<S>>; MAX_CHECKPOINTS],
    next_id: u64,
}

impl<S: Clone, const MAX_CHECKPOINTS: usize> TimeTravel<S, MAX_CHECKPOINTS> {
    pub fn new() -> Self {
        assert!(MAX_CHECKPOINTS > 0);
        Self {
            checkpoints: core::array::from_fn(|_| None),
            next_id: 1,
        }
    }

    pub fn checkpoint(
        &mut self,
        event_sequence: u64,
        state: S,
    ) -> Result<CheckpointInfo, ReplayError> {
        let slot = self
            .checkpoints
            .iter()
            .position(Option::is_none)
            .ok_or(ReplayError::Capacity)?;
        let id = CheckpointId(self.next_id);
        self.next_id = self.next_id.checked_add(1).ok_or(ReplayError::Capacity)?;
        let info = CheckpointInfo { id, event_sequence };
        self.checkpoints[slot] = Some(StoredCheckpoint { info, state });
        Ok(info)
    }

    pub fn restore(&self, id: CheckpointId) -> Result<(CheckpointInfo, S), ReplayError> {
        self.checkpoints
            .iter()
            .flatten()
            .find(|checkpoint| checkpoint.info.id == id)
            .map(|checkpoint| (checkpoint.info, checkpoint.state.clone()))
            .ok_or(ReplayError::NoEvent)
    }

    pub fn release(&mut self, id: CheckpointId) -> Result<(), ReplayError> {
        let checkpoint = self
            .checkpoints
            .iter_mut()
            .find(|checkpoint| checkpoint.as_ref().is_some_and(|value| value.info.id == id))
            .ok_or(ReplayError::NoEvent)?;
        *checkpoint = None;
        Ok(())
    }

    pub fn nearest_before(&self, event_sequence: u64) -> Option<(CheckpointInfo, S)> {
        self.checkpoints
            .iter()
            .flatten()
            .filter(|checkpoint| checkpoint.info.event_sequence <= event_sequence)
            .max_by_key(|checkpoint| checkpoint.info.event_sequence)
            .map(|checkpoint| (checkpoint.info, checkpoint.state.clone()))
    }

    pub fn list(&self, output: &mut [CheckpointInfo]) -> usize {
        let mut written = 0;
        for checkpoint in self.checkpoints.iter().flatten() {
            if written == output.len() {
                break;
            }
            output[written] = checkpoint.info;
            written += 1;
        }
        written
    }
}

impl<S: Clone, const MAX_CHECKPOINTS: usize> Default for TimeTravel<S, MAX_CHECKPOINTS> {
    fn default() -> Self {
        Self::new()
    }
}

fn read_u64(input: &[u8]) -> u64 {
    let mut bytes = [0; 8];
    bytes.copy_from_slice(input);
    u64::from_le_bytes(bytes)
}

fn crash_reason(reason: CrashReason) -> u8 {
    match reason {
        CrashReason::Panic => 1,
        CrashReason::ProtectionFault => 2,
        CrashReason::IllegalInstruction => 3,
        CrashReason::Watchdog => 4,
        CrashReason::UnexpectedExit => 5,
    }
}
