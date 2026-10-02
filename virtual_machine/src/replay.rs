//! Deterministic VM input recording and replay.
//!
//! One ordered stream carries every value that can come from outside the
//! interpreter: device reads, host input, time, interrupt delivery, and DMA
//! completions. Replay consumes the same stream and reports the first point
//! where the guest or device model diverges.

use std::cell::RefCell;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::rc::Rc;

const FILE_HEADER_BYTES: usize = 20;
const EVENT_HEADER_BYTES: usize = 56;
const MAX_REPLAY_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_EVENT_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;
const DEFAULT_MAX_EVENTS: usize = 1_000_000;

#[repr(C)]
#[derive(Default)]
struct CReplayEvent {
    sequence: u64,
    a: u64,
    b: u64,
    c: u64,
    d: u64,
    payload_length: u64,
    kind: u8,
}

unsafe extern "C" {
    fn ghostos_vm_replay_file_decode(bytes: *const u8, length: usize, count: *mut u64) -> u32;
    fn ghostos_vm_replay_file_encode(bytes: *mut u8, count: u64);
    fn ghostos_vm_replay_event_decode(bytes: *const u8, length: usize, sequence: u64, event: *mut CReplayEvent) -> u32;
    fn ghostos_vm_replay_event_encode(bytes: *mut u8, event: *const CReplayEvent) -> u32;
}

fn wire_result(result: u32) -> Result<(), ReplayError> {
    match result {
        0 => Ok(()),
        2 => Err(ReplayError::Capacity),
        3 => Err(ReplayError::PayloadTooLarge),
        _ => Err(ReplayError::Corrupt),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayMode {
    Disabled,
    Recording,
    Replaying,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ReplayEventKind {
    InstructionInput = 1,
    HostInput = 2,
    DeviceCompletion = 3,
    Clock = 4,
    Timer = 5,
    Interrupt = 6,
}

impl ReplayEventKind {
    fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::InstructionInput),
            2 => Some(Self::HostInput),
            3 => Some(Self::DeviceCompletion),
            4 => Some(Self::Clock),
            5 => Some(Self::Timer),
            6 => Some(Self::Interrupt),
            _ => None,
        }
    }
}

impl fmt::Display for ReplayEventKind {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::InstructionInput => "instruction input",
            Self::HostInput => "host input",
            Self::DeviceCompletion => "device completion",
            Self::Clock => "clock",
            Self::Timer => "timer",
            Self::Interrupt => "interrupt",
        };
        output.write_str(name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayEvent {
    pub sequence: u64,
    pub kind: ReplayEventKind,
    pub a: u64,
    pub b: u64,
    pub c: u64,
    pub d: u64,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplayTrace {
    events: Vec<ReplayEvent>,
}

impl ReplayTrace {
    pub fn new(events: Vec<ReplayEvent>) -> Result<Self, ReplayError> {
        validate_events(&events)?;
        Ok(Self { events })
    }

    pub fn events(&self) -> &[ReplayEvent] {
        &self.events
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ReplayError> {
        save_events(&self.events, path.as_ref())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ReplayError> {
        Ok(Self {
            events: load_events(path.as_ref())?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplayError {
    WrongMode(ReplayMode),
    EndOfTrace { expected: ReplayEventKind, sequence: u64 },
    UnexpectedKind {
        expected: ReplayEventKind,
        actual: ReplayEventKind,
        sequence: u64,
    },
    InputMismatch {
        kind: ReplayEventKind,
        sequence: u64,
    },
    Capacity,
    Corrupt,
    PayloadTooLarge,
    Io(String),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongMode(mode) => write!(output, "replay operation is invalid in {mode:?} mode"),
            Self::EndOfTrace { expected, sequence } => {
                write!(output, "replay ended before {expected} event at sequence {sequence}")
            }
            Self::UnexpectedKind {
                expected,
                actual,
                sequence,
            } => write!(
                output,
                "replay expected {expected}, found {actual} at sequence {sequence}"
            ),
            Self::InputMismatch { kind, sequence } => {
                write!(output, "replay input mismatch for {kind} at sequence {sequence}")
            }
            Self::Capacity => output.write_str("replay recording capacity exceeded"),
            Self::Corrupt => output.write_str("corrupt replay trace"),
            Self::PayloadTooLarge => output.write_str("replay event payload is too large"),
            Self::Io(error) => write!(output, "replay I/O failed: {error}"),
        }
    }
}

impl std::error::Error for ReplayError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayDmaWrite {
    pub address: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayHostInput {
    pub channel: u64,
    pub rows: Option<u16>,
    pub columns: Option<u16>,
    pub bytes: Vec<u8>,
}

pub type SharedReplay = Rc<RefCell<ReplaySession>>;

pub fn shared_replay() -> SharedReplay {
    Rc::new(RefCell::new(ReplaySession::new()))
}

/// Ordered replay controller shared by the VM, MMU, and port bus.
pub struct ReplaySession {
    mode: ReplayMode,
    events: Vec<ReplayEvent>,
    cursor: usize,
    max_events: usize,
    last_error: Option<ReplayError>,
}

impl ReplaySession {
    pub fn new() -> Self {
        Self::with_max_events(DEFAULT_MAX_EVENTS)
    }

    pub fn with_max_events(max_events: usize) -> Self {
        Self {
            mode: ReplayMode::Disabled,
            events: Vec::new(),
            cursor: 0,
            max_events: max_events.max(1),
            last_error: None,
        }
    }

    pub fn mode(&self) -> ReplayMode {
        self.mode
    }

    pub fn position(&self) -> usize {
        self.cursor
    }

    pub fn pending(&self) -> usize {
        self.events.len().saturating_sub(self.cursor)
    }

    pub fn begin_recording(&mut self) {
        self.events.clear();
        self.cursor = 0;
        self.last_error = None;
        self.mode = ReplayMode::Recording;
    }

    pub fn begin_replay(&mut self, trace: ReplayTrace) -> Result<(), ReplayError> {
        validate_events(&trace.events)?;
        if trace.events.len() > self.max_events {
            return Err(ReplayError::Capacity)
        }
        self.events = trace.events;
        self.cursor = 0;
        self.last_error = None;
        self.mode = ReplayMode::Replaying;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.mode = ReplayMode::Disabled;
        self.cursor = 0;
        self.last_error = None;
    }

    pub fn trace(&self) -> ReplayTrace {
        ReplayTrace {
            events: self.events.clone(),
        }
    }

    pub fn take_error(&mut self) -> Option<ReplayError> {
        self.last_error.take()
    }

    pub fn peek_kind(&self) -> Option<ReplayEventKind> {
        (self.mode == ReplayMode::Replaying)
            .then(|| self.events.get(self.cursor).map(|event| event.kind))
            .flatten()
    }

    pub fn next(&mut self, expected: ReplayEventKind) -> Result<ReplayEvent, ReplayError> {
        if self.mode != ReplayMode::Replaying {
            return self.fail(ReplayError::WrongMode(self.mode))
        }
        let Some(event) = self.events.get(self.cursor).cloned() else {
            return self.fail(ReplayError::EndOfTrace {
                expected,
                sequence: self.cursor as u64,
            })
        };
        if event.kind != expected {
            return self.fail(ReplayError::UnexpectedKind {
                expected,
                actual: event.kind,
                sequence: event.sequence,
            })
        }
        self.cursor += 1;
        Ok(event)
    }

    /// Record an external device read, or return the recorded value during
    /// replay. `a`, `b`, and `c` identify the instruction, address, and size.
    pub fn instruction_input(
        &mut self,
        instruction_ip: u64,
        address: u64,
        size: u8,
        value: u64,
    ) -> Result<u64, ReplayError> {
        match self.mode {
            ReplayMode::Disabled => Ok(value),
            ReplayMode::Recording => {
                self.record_event(ReplayEventKind::InstructionInput, instruction_ip, address, size as u64, value, Vec::new())?;
                Ok(value)
            }
            ReplayMode::Replaying => {
                let event = self.next(ReplayEventKind::InstructionInput)?;
                if event.a != instruction_ip || event.b != address || event.c != size as u64 {
                    return self.fail(ReplayError::InputMismatch {
                        kind: event.kind,
                        sequence: event.sequence,
                    })
                }
                Ok(event.d)
            }
        }
    }

    pub fn clock(&mut self, now_ns: u64) -> Result<u64, ReplayError> {
        match self.mode {
            ReplayMode::Disabled => Ok(now_ns),
            ReplayMode::Recording => {
                self.record_event(ReplayEventKind::Clock, now_ns, 0, 0, 0, Vec::new())?;
                Ok(now_ns)
            }
            ReplayMode::Replaying => Ok(self.next(ReplayEventKind::Clock)?.a),
        }
    }

    pub fn timer(&mut self, now_ns: u64, pending_vector: Option<u8>) -> Result<(), ReplayError> {
        let vector = pending_vector.map(u64::from).unwrap_or(u64::MAX);
        match self.mode {
            ReplayMode::Disabled => Ok(()),
            ReplayMode::Recording => {
                self.record_event(ReplayEventKind::Timer, now_ns, vector, 0, 0, Vec::new())?;
                Ok(())
            }
            ReplayMode::Replaying => {
                let event = self.next(ReplayEventKind::Timer)?;
                if event.a != now_ns || event.b != vector {
                    return self.fail(ReplayError::InputMismatch {
                        kind: event.kind,
                        sequence: event.sequence,
                    })
                }
                Ok(())
            }
        }
    }

    pub fn interrupt(&mut self, vector: u8) -> Result<(), ReplayError> {
        match self.mode {
            ReplayMode::Disabled => Ok(()),
            ReplayMode::Recording => {
                self.record_event(ReplayEventKind::Interrupt, vector as u64, 0, 0, 0, Vec::new())?;
                Ok(())
            }
            ReplayMode::Replaying => {
                let event = self.next(ReplayEventKind::Interrupt)?;
                if event.a != vector as u64 {
                    return self.fail(ReplayError::InputMismatch {
                        kind: event.kind,
                        sequence: event.sequence,
                    })
                }
                Ok(())
            }
        }
    }

    pub fn host_input(
        &mut self,
        channel: u64,
        rows: Option<u16>,
        columns: Option<u16>,
        bytes: &[u8],
    ) -> Result<(), ReplayError> {
        let packed_resize = match (rows, columns) {
            (Some(rows), Some(columns)) => u64::from(rows) << 32 | u64::from(columns),
            (None, None) => 0,
            _ => return self.fail(ReplayError::InputMismatch {
                kind: ReplayEventKind::HostInput,
                sequence: self.events.len() as u64,
            }),
        };
        match self.mode {
            ReplayMode::Disabled => Ok(()),
            ReplayMode::Recording => {
                self.record_event(
                    ReplayEventKind::HostInput,
                    channel,
                    packed_resize,
                    0,
                    bytes.len() as u64,
                    bytes.to_vec(),
                )?;
                Ok(())
            }
            ReplayMode::Replaying => {
                let event = self.next(ReplayEventKind::HostInput)?;
                if event.a != channel
                    || event.b != packed_resize
                    || event.d != bytes.len() as u64
                    || event.data != bytes
                {
                    return self.fail(ReplayError::InputMismatch {
                        kind: event.kind,
                        sequence: event.sequence,
                    })
                }
                Ok(())
            }
        }
    }

    pub fn next_host_input(&mut self) -> Result<Option<ReplayHostInput>, ReplayError> {
        if self.mode != ReplayMode::Replaying || self.peek_kind() != Some(ReplayEventKind::HostInput) {
            return Ok(None)
        }
        let event = self.next(ReplayEventKind::HostInput)?;
        let (rows, columns) = if event.b == 0 {
            (None, None)
        } else {
            let rows = (event.b >> 32) as u16;
            let columns = event.b as u16;
            if rows == 0 || columns == 0 {
                return self.fail(ReplayError::Corrupt)
            }
            (Some(rows), Some(columns))
        };
        if event.d != event.data.len() as u64 {
            return self.fail(ReplayError::Corrupt)
        }
        Ok(Some(ReplayHostInput {
            channel: event.a,
            rows,
            columns,
            bytes: event.data,
        }))
    }

    pub fn device_completion(
        &mut self,
        writes: &[ReplayDmaWrite],
    ) -> Result<Vec<ReplayDmaWrite>, ReplayError> {
        match self.mode {
            ReplayMode::Disabled => Ok(Vec::new()),
            ReplayMode::Recording => {
                let payload = encode_dma_writes(writes)?;
                let total_bytes = writes
                    .iter()
                    .try_fold(0u64, |total, write| total.checked_add(write.bytes.len() as u64))
                    .ok_or(ReplayError::PayloadTooLarge)?;
                self.record_event(
                    ReplayEventKind::DeviceCompletion,
                    writes.len() as u64,
                    total_bytes,
                    0,
                    0,
                    payload,
                )?;
                Ok(Vec::new())
            }
            ReplayMode::Replaying => {
                let event = self.next(ReplayEventKind::DeviceCompletion)?;
                // Host-backed devices may complete with different bytes or
                // timing on replay. The recorded DMA writes are authoritative.
                decode_dma_writes(&event.data)
            }
        }
    }

    fn record_event(
        &mut self,
        kind: ReplayEventKind,
        a: u64,
        b: u64,
        c: u64,
        d: u64,
        data: Vec<u8>,
    ) -> Result<(), ReplayError> {
        if self.mode != ReplayMode::Recording {
            return self.fail(ReplayError::WrongMode(self.mode))
        }
        if self.events.len() >= self.max_events {
            return self.fail(ReplayError::Capacity)
        }
        if data.len() > MAX_EVENT_PAYLOAD_BYTES {
            return self.fail(ReplayError::PayloadTooLarge)
        }
        self.events.push(ReplayEvent {
            sequence: self.events.len() as u64,
            kind,
            a,
            b,
            c,
            d,
            data,
        });
        Ok(())
    }

    fn fail<T>(&mut self, error: ReplayError) -> Result<T, ReplayError> {
        self.last_error = Some(error.clone());
        Err(error)
    }
}

impl Default for ReplaySession {
    fn default() -> Self {
        Self::new()
    }
}

fn encode_dma_writes(writes: &[ReplayDmaWrite]) -> Result<Vec<u8>, ReplayError> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(writes.len() as u32).to_le_bytes());
    for write in writes {
        let length = u32::try_from(write.bytes.len()).map_err(|_| ReplayError::PayloadTooLarge)?;
        payload.extend_from_slice(&write.address.to_le_bytes());
        payload.extend_from_slice(&length.to_le_bytes());
        payload.extend_from_slice(&write.bytes);
        if payload.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(ReplayError::PayloadTooLarge)
        }
    }
    Ok(payload)
}

fn decode_dma_writes(input: &[u8]) -> Result<Vec<ReplayDmaWrite>, ReplayError> {
    if input.len() < 4 {
        return Err(ReplayError::Corrupt)
    }
    let count = u32::from_le_bytes(
        input[..4].try_into().map_err(|_| ReplayError::Corrupt)?,
    ) as usize;
    let mut offset = 4usize;
    let mut writes = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let header_end = offset.checked_add(12).ok_or(ReplayError::Corrupt)?;
        if header_end > input.len() {
            return Err(ReplayError::Corrupt)
        }
        let address = u64::from_le_bytes(
            input[offset..offset + 8]
                .try_into()
                .map_err(|_| ReplayError::Corrupt)?,
        );
        let length = u32::from_le_bytes(
            input[offset + 8..header_end]
                .try_into()
                .map_err(|_| ReplayError::Corrupt)?,
        ) as usize;
        offset = header_end;
        let end = offset.checked_add(length).ok_or(ReplayError::Corrupt)?;
        if end > input.len() {
            return Err(ReplayError::Corrupt)
        }
        writes.push(ReplayDmaWrite {
            address,
            bytes: input[offset..end].to_vec(),
        });
        offset = end;
    }
    if offset != input.len() {
        return Err(ReplayError::Corrupt)
    }
    Ok(writes)
}

fn validate_events(events: &[ReplayEvent]) -> Result<(), ReplayError> {
    for (index, event) in events.iter().enumerate() {
        if event.sequence != index as u64 || event.data.len() > MAX_EVENT_PAYLOAD_BYTES {
            return Err(ReplayError::Corrupt)
        }
    }
    Ok(())
}

fn save_events(events: &[ReplayEvent], path: &Path) -> Result<(), ReplayError> {
    validate_events(events)?;
    let mut file_bytes = FILE_HEADER_BYTES as u64;
    for event in events {
        file_bytes = file_bytes
            .checked_add(EVENT_HEADER_BYTES as u64)
            .and_then(|size| size.checked_add(event.data.len() as u64))
            .ok_or(ReplayError::Capacity)?;
        if file_bytes > MAX_REPLAY_FILE_BYTES {
            return Err(ReplayError::Capacity)
        }
    }
    let mut file = File::create(path).map_err(io_error)?;
    let mut header = [0u8; FILE_HEADER_BYTES];
    unsafe { ghostos_vm_replay_file_encode(header.as_mut_ptr(), events.len() as u64) };
    file.write_all(&header).map_err(io_error)?;
    for event in events {
        let wire = CReplayEvent {
            sequence: event.sequence,
            a: event.a,
            b: event.b,
            c: event.c,
            d: event.d,
            payload_length: event.data.len() as u64,
            kind: event.kind as u8,
        };
        let mut header = [0u8; EVENT_HEADER_BYTES];
        wire_result(unsafe { ghostos_vm_replay_event_encode(header.as_mut_ptr(), &wire) })?;
        file.write_all(&header).map_err(io_error)?;
        file.write_all(&event.data).map_err(io_error)?;
    }
    file.sync_all().map_err(io_error)
}

fn load_events(path: &Path) -> Result<Vec<ReplayEvent>, ReplayError> {
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    if metadata.len() < FILE_HEADER_BYTES as u64 || metadata.len() > MAX_REPLAY_FILE_BYTES {
        return Err(ReplayError::Corrupt)
    }
    let mut file = File::open(path).map_err(io_error)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes).map_err(io_error)?;
    let mut count = 0;
    wire_result(unsafe { ghostos_vm_replay_file_decode(bytes.as_ptr(), bytes.len(), &mut count) })?;
    let mut offset = FILE_HEADER_BYTES;
    let mut events = Vec::with_capacity(count as usize);
    for sequence in 0..count {
        let mut wire = CReplayEvent::default();
        let remaining = bytes.get(offset..).ok_or(ReplayError::Corrupt)?;
        wire_result(unsafe {
            ghostos_vm_replay_event_decode(remaining.as_ptr(), remaining.len(), sequence, &mut wire)
        })?;
        let data_start = offset + EVENT_HEADER_BYTES;
        let data_end = data_start + wire.payload_length as usize;
        events.push(ReplayEvent {
            sequence: wire.sequence,
            kind: ReplayEventKind::from_raw(wire.kind).ok_or(ReplayError::Corrupt)?,
            a: wire.a,
            b: wire.b,
            c: wire.c,
            d: wire.d,
            data: bytes[data_start..data_end].to_vec(),
        });
        offset = data_end;
    }
    if offset != bytes.len() {
        return Err(ReplayError::Corrupt)
    }
    Ok(events)
}

fn io_error(error: io::Error) -> ReplayError {
    ReplayError::Io(error.to_string())
}
