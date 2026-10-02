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

#[repr(C)]
struct CReplayView {
    event: CReplayEvent,
    payload: *const u8,
}

impl Default for CReplayView {
    fn default() -> Self {
        Self { event: CReplayEvent::default(), payload: std::ptr::null() }
    }
}

#[repr(C)]
#[derive(Default)]
struct CReplayError {
    sequence: u64,
    code: u32,
    mode: u32,
    expected: u8,
    actual: u8,
    kind: u8,
}

#[repr(C)]
struct CReplayDmaWrite {
    address: u64,
    bytes: *const u8,
    length: usize,
}

type CSession = std::ffi::c_void;
type DmaSink = unsafe extern "C" fn(*mut std::ffi::c_void, u64, *const u8, usize);

const _: () = assert!(std::mem::size_of::<CReplayEvent>() == 56);
const _: () = assert!(std::mem::offset_of!(CReplayEvent, kind) == 48);
const _: () = assert!(std::mem::size_of::<CReplayError>() == 24);

unsafe extern "C" {
    fn ghostos_vm_replay_file_decode(bytes: *const u8, length: usize, count: *mut u64) -> u32;
    fn ghostos_vm_replay_file_encode(bytes: *mut u8, count: u64);
    fn ghostos_vm_replay_event_decode(bytes: *const u8, length: usize, sequence: u64, event: *mut CReplayEvent) -> u32;
    fn ghostos_vm_replay_event_encode(bytes: *mut u8, event: *const CReplayEvent) -> u32;
    fn ghostos_vm_replay_validate(events: *const CReplayView, count: usize) -> u32;
    fn ghostos_vm_replay_file_size(events: *const CReplayView, count: usize, size: *mut u64) -> u32;
    fn ghostos_vm_replay_session_new(max_events: usize) -> *mut CSession;
    fn ghostos_vm_replay_session_free(session: *mut CSession);
    fn ghostos_vm_replay_mode_get(session: *const CSession) -> u32;
    fn ghostos_vm_replay_position(session: *const CSession) -> usize;
    fn ghostos_vm_replay_pending(session: *const CSession) -> usize;
    fn ghostos_vm_replay_length(session: *const CSession) -> usize;
    fn ghostos_vm_replay_begin_recording(session: *mut CSession);
    fn ghostos_vm_replay_begin(session: *mut CSession, events: *const CReplayView, count: usize) -> u32;
    fn ghostos_vm_replay_stop(session: *mut CSession);
    fn ghostos_vm_replay_take_error(session: *mut CSession, error: *mut CReplayError) -> bool;
    fn ghostos_vm_replay_error_get(session: *const CSession, error: *mut CReplayError);
    fn ghostos_vm_replay_peek_kind(session: *const CSession) -> u8;
    fn ghostos_vm_replay_event_get(session: *const CSession, index: usize, view: *mut CReplayView) -> bool;
    fn ghostos_vm_replay_next(session: *mut CSession, expected: u8, view: *mut CReplayView) -> u32;
    fn ghostos_vm_replay_instruction_input(session: *mut CSession, ip: u64, address: u64, size: u8, value: u64, output: *mut u64) -> u32;
    fn ghostos_vm_replay_clock(session: *mut CSession, now: u64, output: *mut u64) -> u32;
    fn ghostos_vm_replay_timer(session: *mut CSession, now: u64, vector: u64) -> u32;
    fn ghostos_vm_replay_interrupt(session: *mut CSession, vector: u8) -> u32;
    fn ghostos_vm_replay_host_input(session: *mut CSession, channel: u64, has_rows: bool, rows: u16, has_columns: bool, columns: u16, bytes: *const u8, length: usize) -> u32;
    fn ghostos_vm_replay_next_host_input(session: *mut CSession, present: *mut bool, view: *mut CReplayView, has_resize: *mut bool, rows: *mut u16, columns: *mut u16) -> u32;
    fn ghostos_vm_replay_device_completion(session: *mut CSession, writes: *const CReplayDmaWrite, count: usize, sink: DmaSink, context: *mut std::ffi::c_void) -> u32;
}

fn wire_result(result: u32) -> Result<(), ReplayError> {
    match result {
        0 => Ok(()),
        2 => Err(ReplayError::Capacity),
        3 => Err(ReplayError::PayloadTooLarge),
        _ => Err(ReplayError::Corrupt),
    }
}

fn replay_mode(mode: u32) -> ReplayMode {
    match mode {
        1 => ReplayMode::Recording,
        2 => ReplayMode::Replaying,
        _ => ReplayMode::Disabled,
    }
}

fn replay_error(error: CReplayError) -> ReplayError {
    match error.code {
        4 => ReplayError::WrongMode(replay_mode(error.mode)),
        5 => ReplayError::EndOfTrace {
            expected: ReplayEventKind::from_raw(error.expected).expect("C replay kind"),
            sequence: error.sequence,
        },
        6 => ReplayError::UnexpectedKind {
            expected: ReplayEventKind::from_raw(error.expected).expect("C replay kind"),
            actual: ReplayEventKind::from_raw(error.actual).expect("C replay kind"),
            sequence: error.sequence,
        },
        7 => ReplayError::InputMismatch {
            kind: ReplayEventKind::from_raw(error.kind).expect("C replay kind"),
            sequence: error.sequence,
        },
        code => wire_result(code).expect_err("C replay error"),
    }
}

fn event_views(events: &[ReplayEvent]) -> Vec<CReplayView> {
    events.iter().map(|event| CReplayView {
        event: CReplayEvent {
            sequence: event.sequence,
            a: event.a,
            b: event.b,
            c: event.c,
            d: event.d,
            payload_length: event.data.len() as u64,
            kind: event.kind as u8,
        },
        payload: event.data.as_ptr(),
    }).collect()
}

// C owns these bytes. Copy them before a recording/replay reset or destruction.
unsafe fn copy_payload(view: &CReplayView) -> Vec<u8> {
    if view.event.payload_length == 0 { return Vec::new() }
    unsafe { std::slice::from_raw_parts(view.payload, view.event.payload_length as usize).to_vec() }
}

unsafe fn copy_event(view: &CReplayView) -> ReplayEvent {
    ReplayEvent {
        sequence: view.event.sequence,
        kind: ReplayEventKind::from_raw(view.event.kind).expect("C replay kind"),
        a: view.event.a,
        b: view.event.b,
        c: view.event.c,
        d: view.event.d,
        data: unsafe { copy_payload(view) },
    }
}

unsafe extern "C" fn collect_dma(context: *mut std::ffi::c_void, address: u64, bytes: *const u8, length: usize) {
    let writes = unsafe { &mut *context.cast::<Vec<ReplayDmaWrite>>() };
    let bytes = unsafe { std::slice::from_raw_parts(bytes, length).to_vec() };
    writes.push(ReplayDmaWrite { address, bytes });
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
    session: std::ptr::NonNull<CSession>,
}

impl Drop for ReplaySession {
    fn drop(&mut self) {
        unsafe { ghostos_vm_replay_session_free(self.session.as_ptr()) }
    }
}

impl ReplaySession {
    pub fn new() -> Self {
        Self::with_max_events(DEFAULT_MAX_EVENTS)
    }

    pub fn with_max_events(max_events: usize) -> Self {
        let session = unsafe { ghostos_vm_replay_session_new(max_events) };
        Self { session: std::ptr::NonNull::new(session).expect("C replay session allocation") }
    }

    fn result(&self, code: u32) -> Result<(), ReplayError> {
        if code < 4 { return wire_result(code) }
        let mut error = CReplayError::default();
        unsafe { ghostos_vm_replay_error_get(self.session.as_ptr(), &mut error) };
        Err(replay_error(error))
    }

    pub fn mode(&self) -> ReplayMode {
        replay_mode(unsafe { ghostos_vm_replay_mode_get(self.session.as_ptr()) })
    }

    pub fn position(&self) -> usize {
        unsafe { ghostos_vm_replay_position(self.session.as_ptr()) }
    }

    pub fn pending(&self) -> usize {
        unsafe { ghostos_vm_replay_pending(self.session.as_ptr()) }
    }

    pub fn begin_recording(&mut self) {
        unsafe { ghostos_vm_replay_begin_recording(self.session.as_ptr()) }
    }

    pub fn begin_replay(&mut self, trace: ReplayTrace) -> Result<(), ReplayError> {
        let views = event_views(&trace.events);
        wire_result(unsafe { ghostos_vm_replay_begin(self.session.as_ptr(), views.as_ptr(), views.len()) })
    }

    pub fn stop(&mut self) {
        unsafe { ghostos_vm_replay_stop(self.session.as_ptr()) }
    }

    pub fn trace(&self) -> ReplayTrace {
        let count = unsafe { ghostos_vm_replay_length(self.session.as_ptr()) };
        let mut events = Vec::with_capacity(count);
        for index in 0..count {
            let mut view = CReplayView::default();
            assert!(unsafe { ghostos_vm_replay_event_get(self.session.as_ptr(), index, &mut view) });
            events.push(unsafe { copy_event(&view) });
        }
        ReplayTrace { events }
    }

    pub fn take_error(&mut self) -> Option<ReplayError> {
        let mut error = CReplayError::default();
        unsafe { ghostos_vm_replay_take_error(self.session.as_ptr(), &mut error) }
            .then(|| replay_error(error))
    }

    pub fn peek_kind(&self) -> Option<ReplayEventKind> {
        ReplayEventKind::from_raw(unsafe { ghostos_vm_replay_peek_kind(self.session.as_ptr()) })
    }

    pub fn next(&mut self, expected: ReplayEventKind) -> Result<ReplayEvent, ReplayError> {
        let mut view = CReplayView::default();
        let code = unsafe { ghostos_vm_replay_next(self.session.as_ptr(), expected as u8, &mut view) };
        self.result(code)?;
        Ok(unsafe { copy_event(&view) })
    }

    pub fn instruction_input(
        &mut self,
        instruction_ip: u64,
        address: u64,
        size: u8,
        value: u64,
    ) -> Result<u64, ReplayError> {
        let mut output = 0;
        let code = unsafe {
            ghostos_vm_replay_instruction_input(self.session.as_ptr(), instruction_ip, address, size, value, &mut output)
        };
        self.result(code)?;
        Ok(output)
    }

    pub fn clock(&mut self, now_ns: u64) -> Result<u64, ReplayError> {
        let mut output = 0;
        let code = unsafe { ghostos_vm_replay_clock(self.session.as_ptr(), now_ns, &mut output) };
        self.result(code)?;
        Ok(output)
    }

    pub fn timer(&mut self, now_ns: u64, pending_vector: Option<u8>) -> Result<(), ReplayError> {
        let vector = pending_vector.map(u64::from).unwrap_or(u64::MAX);
        self.result(unsafe { ghostos_vm_replay_timer(self.session.as_ptr(), now_ns, vector) })
    }

    pub fn interrupt(&mut self, vector: u8) -> Result<(), ReplayError> {
        self.result(unsafe { ghostos_vm_replay_interrupt(self.session.as_ptr(), vector) })
    }

    pub fn host_input(
        &mut self,
        channel: u64,
        rows: Option<u16>,
        columns: Option<u16>,
        bytes: &[u8],
    ) -> Result<(), ReplayError> {
        self.result(unsafe {
            ghostos_vm_replay_host_input(self.session.as_ptr(), channel, rows.is_some(), rows.unwrap_or(0),
                columns.is_some(), columns.unwrap_or(0), bytes.as_ptr(), bytes.len())
        })
    }

    pub fn next_host_input(&mut self) -> Result<Option<ReplayHostInput>, ReplayError> {
        let mut present = false;
        let mut view = CReplayView::default();
        let mut has_resize = false;
        let mut rows = 0;
        let mut columns = 0;
        let code = unsafe {
            ghostos_vm_replay_next_host_input(self.session.as_ptr(), &mut present, &mut view,
                &mut has_resize, &mut rows, &mut columns)
        };
        self.result(code)?;
        Ok(present.then(|| ReplayHostInput {
            channel: view.event.a,
            rows: has_resize.then_some(rows),
            columns: has_resize.then_some(columns),
            bytes: unsafe { copy_payload(&view) },
        }))
    }

    pub fn device_completion(
        &mut self,
        writes: &[ReplayDmaWrite],
    ) -> Result<Vec<ReplayDmaWrite>, ReplayError> {
        let views: Vec<_> = writes.iter().map(|write| CReplayDmaWrite {
            address: write.address, bytes: write.bytes.as_ptr(), length: write.bytes.len(),
        }).collect();
        let mut output = Vec::new();
        let code = unsafe {
            ghostos_vm_replay_device_completion(self.session.as_ptr(), views.as_ptr(), views.len(),
                collect_dma, (&mut output as *mut Vec<ReplayDmaWrite>).cast())
        };
        self.result(code)?;
        Ok(output)
    }
}

impl Default for ReplaySession {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_events(events: &[ReplayEvent]) -> Result<(), ReplayError> {
    let views = event_views(events);
    wire_result(unsafe { ghostos_vm_replay_validate(views.as_ptr(), views.len()) })
}

fn save_events(events: &[ReplayEvent], path: &Path) -> Result<(), ReplayError> {
    let views = event_views(events);
    let mut file_bytes = 0;
    wire_result(unsafe { ghostos_vm_replay_file_size(views.as_ptr(), views.len(), &mut file_bytes) })?;
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
