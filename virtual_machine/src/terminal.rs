//! Host terminal session for the guest serial console.

use std::cell::{Cell, RefCell};
use std::io::{self, IsTerminal, Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

#[path = "terminal_platform.rs"]
mod terminal_platform;

const TERMINAL_SIZE_POLL_INTERVAL: Duration = Duration::from_millis(250);

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
        self.events
            .iter()
            .map(|event| match event {
                TerminalTranscriptEvent::Input { bytes, .. }
                | TerminalTranscriptEvent::Eof { bytes } => TerminalInput {
                    bytes: bytes.clone(),
                    resize: None,
                },
                TerminalTranscriptEvent::Resize(resize) => TerminalInput {
                    bytes: Vec::new(),
                    resize: Some(*resize),
                },
            })
            .collect()
    }

    fn record(&mut self, event: TerminalTranscriptEvent) {
        self.events.push(event)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalExit {
    GuestShutdown,
}

/// Owns host input polling and terminal restoration for one VM session.
pub struct TerminalSession {
    events: Receiver<InputEvent>,
    output: RefCell<Box<dyn Write + Send>>,
    raw_mode: RawMode,
    transcript: RefCell<TerminalTranscript>,
    has_terminal: bool,
    last_size: Cell<Option<(u16, u16)>>,
    last_size_check: Cell<Option<Instant>>,
    diagnostics: Cell<TerminalSessionDiagnostics>,
}

impl TerminalSession {
    /// Create a session. `Some(true)` requests interactive behavior, while
    /// `Some(false)` keeps stdin usable without changing terminal settings.
    /// With `None`, raw mode is selected only when stdin and stdout are TTYs.
    pub fn new(requested_interactive: Option<bool>) -> Result<Self, TerminalError> {
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
            transcript: RefCell::new(TerminalTranscript::default()),
            has_terminal: is_tty,
            last_size: Cell::new(None),
            last_size_check: Cell::new(None),
            diagnostics: Cell::new(TerminalSessionDiagnostics::default()),
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
        Self {
            events: spawn_input_reader(Box::new(input)),
            output: RefCell::new(Box::new(output)),
            raw_mode: RawMode::inactive(),
            transcript: RefCell::new(TerminalTranscript::default()),
            has_terminal: false,
            last_size: Cell::new(None),
            last_size_check: Cell::new(None),
            diagnostics: Cell::new(TerminalSessionDiagnostics::default()),
        }
    }

    /// Drain available host input without blocking the VM execution loop.
    pub fn poll(&self) -> Result<TerminalInput, TerminalError> {
        self.update_diagnostics(|diagnostics| diagnostics.polls += 1);
        let mut input = TerminalInput::default();
        let mut pending = Vec::new();

        while let Ok(event) = self.events.try_recv() {
            pending.push(event)
        }

        let size_check_due = self.last_size.get().is_none()
            || self
                .last_size_check
                .get()
                .is_none_or(|last| last.elapsed() >= TERMINAL_SIZE_POLL_INTERVAL);
        if self.has_terminal && size_check_due {
            self.last_size_check.set(Some(Instant::now()));
            if let Some((rows, columns)) = terminal_platform::size() {
                let resize = TerminalResize { rows, columns };
                if self.last_size.get() != Some((rows, columns)) {
                    input.resize = Some(resize);
                    self.last_size.set(Some((rows, columns)));
                    self.transcript
                        .borrow_mut()
                        .record(TerminalTranscriptEvent::Resize(resize));
                    self.update_diagnostics(|diagnostics| diagnostics.resize_events += 1);
                }
            }
        }

        for event in pending {
            match event {
                InputEvent::Bytes(raw) => {
                    let bytes = translate_input_bytes(&raw);
                    input.bytes.extend(&bytes);
                    self.update_diagnostics(|diagnostics| {
                        diagnostics.input_bytes += raw.len() as u64
                    });
                    self.transcript
                        .borrow_mut()
                        .record(TerminalTranscriptEvent::Input { raw, bytes });
                }
                InputEvent::Eof => {
                    self.update_diagnostics(|diagnostics| diagnostics.eof_events += 1);
                    let bytes = vec![0x04];
                    input.bytes.extend(&bytes);
                    self.transcript
                        .borrow_mut()
                        .record(TerminalTranscriptEvent::Eof { bytes });
                    if let Err(error) = self.raw_mode.restore() {
                        return Err(self.record_failure(error))
                    }
                }
                InputEvent::Error(diagnostic) => {
                    return Err(self.record_failure(TerminalError { diagnostic }))
                }
            }
        }

        Ok(input)
    }

    pub fn transcript(&self) -> TerminalTranscript {
        self.transcript.borrow().clone()
    }

    pub fn diagnostics(&self) -> TerminalSessionDiagnostics {
        self.diagnostics.get()
    }

    pub fn flush_output(&self) -> Result<(), TerminalError> {
        let result = self.output
            .borrow_mut()
            .flush()
            .map_err(|error| TerminalError::from_io(TerminalOperation::FlushOutput, error));
        match result {
            Ok(()) => {
                self.update_diagnostics(|diagnostics| diagnostics.output_flushes += 1);
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
                self.update_diagnostics(|diagnostics| {
                    diagnostics.output_bytes += bytes.len() as u64
                });
                Ok(())
            }
            Err(error) => Err(self.record_failure(error)),
        }
    }

    fn update_diagnostics(&self, update: impl FnOnce(&mut TerminalSessionDiagnostics)) {
        let mut diagnostics = self.diagnostics.get();
        update(&mut diagnostics);
        self.diagnostics.set(diagnostics)
    }

    fn record_failure(&self, error: TerminalError) -> TerminalError {
        self.update_diagnostics(|diagnostics| {
            diagnostics.last_failure = Some(error.diagnostic())
        });
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
    bytes
        .iter()
        .map(|byte| if *byte == 0x7F { 0x08 } else { *byte })
        .collect()
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
