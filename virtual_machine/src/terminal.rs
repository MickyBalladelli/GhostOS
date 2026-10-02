//! Host terminal session for the guest serial console.

use std::cell::{Cell, RefCell};
use std::io::{self, IsTerminal, Read, Write};
use std::sync::mpsc::{self, Receiver};
use crate::{HostMonotonicClock, SharedMonotonicClock};
use std::rc::Rc;

#[path = "terminal_platform.rs"]
mod terminal_platform;

#[repr(C)]
#[derive(Default)]
struct CTerminalCounters {
    polls: u64,
    input_bytes: u64,
    output_bytes: u64,
    output_flushes: u64,
    eof_events: u64,
    resize_events: u64,
}

#[repr(C)]
struct CTerminalInput {
    bytes: *const u8,
    length: usize,
    rows: u16,
    columns: u16,
    resize: bool,
}

impl Default for CTerminalInput {
    fn default() -> Self {
        Self { bytes: std::ptr::null(), length: 0, rows: 0, columns: 0, resize: false }
    }
}

#[repr(C)]
struct CTerminalEvent {
    raw: *const u8,
    raw_length: usize,
    bytes: *const u8,
    length: usize,
    rows: u16,
    columns: u16,
    kind: u8,
}

impl Default for CTerminalEvent {
    fn default() -> Self {
        Self { raw: std::ptr::null(), raw_length: 0, bytes: std::ptr::null(), length: 0, rows: 0, columns: 0, kind: 0 }
    }
}

type CTerminal = std::ffi::c_void;

const _: () = assert!(std::mem::size_of::<CTerminalCounters>() == 48);
const _: () = assert!(std::mem::offset_of!(CTerminalInput, rows) == std::mem::size_of::<*const u8>() + std::mem::size_of::<usize>());
const _: () = assert!(std::mem::offset_of!(CTerminalEvent, rows) == 2 * (std::mem::size_of::<*const u8>() + std::mem::size_of::<usize>()));

unsafe extern "C" {
    fn ghostos_vm_terminal_new() -> *mut CTerminal;
    fn ghostos_vm_terminal_free(terminal: *mut CTerminal);
    fn ghostos_vm_terminal_poll_begin(terminal: *mut CTerminal, now: u64, has_terminal: bool) -> bool;
    fn ghostos_vm_terminal_resize(terminal: *mut CTerminal, rows: u16, columns: u16) -> bool;
    fn ghostos_vm_terminal_accept_input(terminal: *mut CTerminal, bytes: *const u8, length: usize) -> bool;
    fn ghostos_vm_terminal_accept_eof(terminal: *mut CTerminal) -> bool;
    fn ghostos_vm_terminal_poll_input(terminal: *const CTerminal, input: *mut CTerminalInput);
    fn ghostos_vm_terminal_counters_get(terminal: *const CTerminal, counters: *mut CTerminalCounters);
    fn ghostos_vm_terminal_output_written(terminal: *mut CTerminal, count: usize);
    fn ghostos_vm_terminal_output_flushed(terminal: *mut CTerminal);
    fn ghostos_vm_terminal_event_count(terminal: *const CTerminal) -> usize;
    fn ghostos_vm_terminal_event_get(terminal: *const CTerminal, index: usize, event: *mut CTerminalEvent) -> bool;
    fn ghostos_vm_terminal_replay_event(event: *const CTerminalEvent, input: *mut CTerminalInput) -> bool;
    fn ghostos_vm_terminal_translate(bytes: *const u8, length: usize, previous_cr: *mut bool, output: *mut u8, capacity: usize, output_length: *mut usize) -> bool;
}

struct CTerminalState(std::ptr::NonNull<CTerminal>);

impl CTerminalState {
    fn new() -> Self {
        Self(std::ptr::NonNull::new(unsafe { ghostos_vm_terminal_new() }).expect("C terminal allocation"))
    }

    fn pointer(&self) -> *mut CTerminal { self.0.as_ptr() }
}

impl Drop for CTerminalState {
    fn drop(&mut self) { unsafe { ghostos_vm_terminal_free(self.pointer()) } }
}

unsafe fn terminal_bytes(bytes: *const u8, length: usize) -> Vec<u8> {
    if length == 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(bytes, length).to_vec() } }
}

unsafe fn terminal_input(input: &CTerminalInput) -> TerminalInput {
    TerminalInput {
        bytes: unsafe { terminal_bytes(input.bytes, input.length) },
        resize: input.resize.then_some(TerminalResize { rows: input.rows, columns: input.columns }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalOperation {
    EnableRawMode,
    RestoreTerminal,
    ReadInput,
    WriteOutput,
    FlushOutput,
}

impl std::fmt::Display for TerminalOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EnableRawMode => f.write_str("enable raw mode"),
            Self::RestoreTerminal => f.write_str("restore terminal settings"),
            Self::ReadInput => f.write_str("read input"),
            Self::WriteOutput => f.write_str("write output"),
            Self::FlushOutput => f.write_str("flush output"),
        }
    }
}

/// Safe, structured description of a terminal failure.
///
/// It deliberately excludes the `io::Error` message and terminal byte data.
/// Those strings can contain guest-controlled escape sequences when supplied
/// by an embedding stream and must not reach host logs or error displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalFailure {
    pub operation: TerminalOperation,
    pub error_kind: io::ErrorKind,
    pub os_error: Option<i32>,
}

impl TerminalFailure {
    fn from_io(operation: TerminalOperation, error: &io::Error) -> Self {
        Self {
            operation,
            error_kind: error.kind(),
            os_error: error.raw_os_error(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalError {
    diagnostic: TerminalFailure,
}

impl TerminalError {
    fn from_io(operation: TerminalOperation, error: io::Error) -> Self {
        Self {
            diagnostic: TerminalFailure::from_io(operation, &error),
        }
    }

    pub fn diagnostic(&self) -> TerminalFailure {
        self.diagnostic
    }
}

impl std::fmt::Display for TerminalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let diagnostic = self.diagnostic;
        write!(f, "terminal {} failed ({:?}", diagnostic.operation, diagnostic.error_kind)?;
        if let Some(code) = diagnostic.os_error {
            write!(f, ", OS error {code}")?;
        }
        f.write_str(")")
    }
}

impl std::error::Error for TerminalError {}

impl From<io::Error> for TerminalError {
    fn from(error: io::Error) -> Self {
        Self::from_io(TerminalOperation::ReadInput, error)
    }
}

enum InputEvent {
    Bytes(Vec<u8>),
    Eof,
    Error(TerminalFailure),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerminalSessionDiagnostics {
    pub polls: u64,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub output_flushes: u64,
    pub eof_events: u64,
    pub resize_events: u64,
    pub last_failure: Option<TerminalFailure>,
}

/// Input collected since the previous VM poll.
#[derive(Debug, Default)]
pub struct TerminalInput {
    pub bytes: Vec<u8>,
    pub resize: Option<TerminalResize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalResize {
    pub rows: u16,
    pub columns: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalTranscriptEvent {
    Input { raw: Vec<u8>, bytes: Vec<u8> },
    Eof { bytes: Vec<u8> },
    Resize(TerminalResize),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalTranscript {
    events: Vec<TerminalTranscriptEvent>,
}

impl TerminalTranscript {
    pub fn events(&self) -> &[TerminalTranscriptEvent] {
        &self.events
    }

    /// Replay terminal policy events in deterministic order. The result is
    /// independent of wall clock, host terminal size, and process input.
    pub fn replay(&self) -> Vec<TerminalInput> {
        self.events.iter().map(|event| {
            let mut wire = CTerminalEvent::default();
            match event {
                TerminalTranscriptEvent::Input { raw, bytes } => {
                    wire.kind = 1;
                    wire.raw = raw.as_ptr();
                    wire.raw_length = raw.len();
                    wire.bytes = bytes.as_ptr();
                    wire.length = bytes.len();
                }
                TerminalTranscriptEvent::Eof { bytes } => {
                    wire.kind = 2;
                    wire.bytes = bytes.as_ptr();
                    wire.length = bytes.len();
                }
                TerminalTranscriptEvent::Resize(resize) => {
                    wire.kind = 3;
                    wire.rows = resize.rows;
                    wire.columns = resize.columns;
                }
            }
            let mut input = CTerminalInput::default();
            assert!(unsafe { ghostos_vm_terminal_replay_event(&wire, &mut input) });
            unsafe { terminal_input(&input) }
        }).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalExit {
    GuestShutdown,
    Interrupted,
}

/// Owns host input polling and terminal restoration for one VM session.
pub struct TerminalSession {
    events: Receiver<InputEvent>,
    output: RefCell<Box<dyn Write + Send>>,
    raw_mode: RawMode,
    state: RefCell<CTerminalState>,
    has_terminal: bool,
    clock: SharedMonotonicClock,
    last_failure: Cell<Option<TerminalFailure>>,
}

impl TerminalSession {
    /// Create a session. `Some(true)` requests interactive behavior, while
    /// `Some(false)` keeps stdin usable without changing terminal settings.
    /// With `None`, raw mode is selected only when stdin and stdout are TTYs.
    pub fn new(requested_interactive: Option<bool>) -> Result<Self, TerminalError> {
        Self::new_with_clock(requested_interactive, Rc::new(HostMonotonicClock::new()))
    }

    /// Create a terminal session with an injected monotonic clock.
    pub fn new_with_clock(
        requested_interactive: Option<bool>,
        clock: SharedMonotonicClock,
    ) -> Result<Self, TerminalError> {
        let is_tty = io::stdin().is_terminal() && io::stdout().is_terminal();
        let interactive = requested_interactive.unwrap_or(is_tty);
        let raw_mode = if interactive && is_tty {
            RawMode::enter()?
        } else {
            RawMode::inactive()
        };

        let events = spawn_input_reader(Box::new(io::stdin()));

        Ok(Self {
            events,
            output: RefCell::new(Box::new(io::stdout())),
            raw_mode,
            state: RefCell::new(CTerminalState::new()),
            has_terminal: is_tty,
            clock,
            last_failure: Cell::new(None),
        })
    }

    /// Create a session over supplied streams without changing host terminal
    /// settings. Intended for embedders and deterministic tests using pipes,
    /// cursors, or fake readers and writers.
    pub fn new_with_io<I, O>(input: I, output: O) -> Self
    where
        I: Read + Send + 'static,
        O: Write + Send + 'static,
    {
        Self::new_with_io_and_clock(input, output, Rc::new(HostMonotonicClock::new()))
    }

    /// Create a non-interactive session with an injected monotonic clock.
    pub fn new_with_io_and_clock<I, O>(input: I, output: O, clock: SharedMonotonicClock) -> Self
    where
        I: Read + Send + 'static,
        O: Write + Send + 'static,
    {
        Self {
            events: spawn_input_reader(Box::new(input)),
            output: RefCell::new(Box::new(output)),
            raw_mode: RawMode::inactive(),
            state: RefCell::new(CTerminalState::new()),
            has_terminal: false,
            clock,
            last_failure: Cell::new(None),
        }
    }

    /// Drain available host input without blocking the VM execution loop.
    pub fn poll(&self) -> Result<TerminalInput, TerminalError> {
        self.poll_at(self.clock.now_ns())
    }

    /// Drain available input at a caller-provided VM time.
    pub fn poll_at(&self, now_ns: u64) -> Result<TerminalInput, TerminalError> {
        let state = self.state.borrow_mut();
        let check_size = unsafe { ghostos_vm_terminal_poll_begin(state.pointer(), now_ns, self.has_terminal) };
        let mut pending = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            pending.push(event)
        }
        if check_size {
            if let Some((rows, columns)) = terminal_platform::size() {
                assert!(unsafe { ghostos_vm_terminal_resize(state.pointer(), rows, columns) }, "C terminal transcript allocation");
            }
        }
        for event in pending {
            match event {
                InputEvent::Bytes(raw) => {
                    assert!(unsafe { ghostos_vm_terminal_accept_input(state.pointer(), raw.as_ptr(), raw.len()) },
                        "C terminal input allocation");
                }
                InputEvent::Eof => {
                    assert!(unsafe { ghostos_vm_terminal_accept_eof(state.pointer()) }, "C terminal EOF allocation");
                    if let Err(error) = self.raw_mode.restore() {
                        return Err(self.record_failure(error))
                    }
                }
                InputEvent::Error(diagnostic) => {
                    return Err(self.record_failure(TerminalError { diagnostic }))
                }
            }
        }
        let mut input = CTerminalInput::default();
        unsafe { ghostos_vm_terminal_poll_input(state.pointer(), &mut input) };
        Ok(unsafe { terminal_input(&input) })
    }

    pub fn transcript(&self) -> TerminalTranscript {
        let state = self.state.borrow();
        let count = unsafe { ghostos_vm_terminal_event_count(state.pointer()) };
        let mut events = Vec::with_capacity(count);
        for index in 0..count {
            let mut event = CTerminalEvent::default();
            assert!(unsafe { ghostos_vm_terminal_event_get(state.pointer(), index, &mut event) });
            events.push(match event.kind {
                1 => TerminalTranscriptEvent::Input {
                    raw: unsafe { terminal_bytes(event.raw, event.raw_length) },
                    bytes: unsafe { terminal_bytes(event.bytes, event.length) },
                },
                2 => TerminalTranscriptEvent::Eof { bytes: unsafe { terminal_bytes(event.bytes, event.length) } },
                3 => TerminalTranscriptEvent::Resize(TerminalResize { rows: event.rows, columns: event.columns }),
                _ => panic!("C terminal event kind"),
            });
        }
        TerminalTranscript { events }
    }

    pub fn diagnostics(&self) -> TerminalSessionDiagnostics {
        let mut counters = CTerminalCounters::default();
        unsafe { ghostos_vm_terminal_counters_get(self.state.borrow().pointer(), &mut counters) };
        TerminalSessionDiagnostics {
            polls: counters.polls,
            input_bytes: counters.input_bytes,
            output_bytes: counters.output_bytes,
            output_flushes: counters.output_flushes,
            eof_events: counters.eof_events,
            resize_events: counters.resize_events,
            last_failure: self.last_failure.get(),
        }
    }

    pub fn flush_output(&self) -> Result<(), TerminalError> {
        let result = self.output
            .borrow_mut()
            .flush()
            .map_err(|error| TerminalError::from_io(TerminalOperation::FlushOutput, error));
        match result {
            Ok(()) => {
                unsafe { ghostos_vm_terminal_output_flushed(self.state.borrow_mut().pointer()) };
                Ok(())
            }
            Err(error) => Err(self.record_failure(error)),
        }
    }

    pub fn write_output(&self, bytes: &[u8]) -> Result<(), TerminalError> {
        let result = self.output
            .borrow_mut()
            .write_all(bytes)
            .map_err(|error| TerminalError::from_io(TerminalOperation::WriteOutput, error));
        match result {
            Ok(()) => {
                unsafe { ghostos_vm_terminal_output_written(self.state.borrow_mut().pointer(), bytes.len()) };
                Ok(())
            }
            Err(error) => Err(self.record_failure(error)),
        }
    }

    fn record_failure(&self, error: TerminalError) -> TerminalError {
        self.last_failure.set(Some(error.diagnostic()));
        error
    }
}

fn spawn_input_reader(mut input: Box<dyn Read + Send>) -> Receiver<InputEvent> {
    let (sender, events) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 1024];
        loop {
            match input.read(&mut buffer) {
                Ok(0) => {
                    let _ = sender.send(InputEvent::Eof);
                    break
                }
                Ok(count) => {
                    if sender.send(InputEvent::Bytes(buffer[..count].to_vec())).is_err() {
                        break
                    }
                }
                Err(error) => {
                    let diagnostic = TerminalFailure::from_io(TerminalOperation::ReadInput, &error);
                    let _ = sender.send(InputEvent::Error(diagnostic));
                    break
                }
            }
        }
    });
    events
}

/// Apply host terminal policy to raw input bytes.
///
/// This stays independent from stdin so the exact input policy can be tested
/// without taking ownership of the process terminal.
pub fn translate_input_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut previous_cr = false;
    let mut translated = vec![0; bytes.len()];
    let mut length = 0;
    assert!(unsafe {
        ghostos_vm_terminal_translate(bytes.as_ptr(), bytes.len(), &mut previous_cr,
            translated.as_mut_ptr(), translated.len(), &mut length)
    });
    translated.truncate(length);
    translated
}

struct RawMode {
    mode: RefCell<Option<terminal_platform::TerminalMode>>,
}

impl RawMode {
    fn enter() -> Result<Self, TerminalError> {
        Ok(Self {
            mode: RefCell::new(Some(
                terminal_platform::TerminalMode::enter().map_err(|error| {
                    TerminalError::from_io(TerminalOperation::EnableRawMode, error)
                })?,
            )),
        })
    }

    fn inactive() -> Self {
        Self {
            mode: RefCell::new(None),
        }
    }

    fn restore(&self) -> Result<(), TerminalError> {
        let Some(mut mode) = self.mode.borrow_mut().take() else {
            return Ok(())
        };
        mode.restore()
            .map_err(|error| TerminalError::from_io(TerminalOperation::RestoreTerminal, error))
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
