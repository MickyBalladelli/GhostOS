//! Host terminal session for the guest serial console.

use std::cell::{Cell, RefCell};
use std::io::{self, IsTerminal, Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

#[path = "terminal_platform.rs"]
mod terminal_platform;

const TERMINAL_SIZE_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub enum TerminalError {
    Io(io::Error),
    RawMode(String),
}

impl std::fmt::Display for TerminalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "terminal I/O failed: {error}"),
            Self::RawMode(error) => write!(f, "terminal setup failed: {error}"),
        }
    }
}

impl std::error::Error for TerminalError {}

impl From<io::Error> for TerminalError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

enum InputEvent {
    Bytes(Vec<u8>),
    Eof,
    Error(io::Error),
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
        }
    }

    /// Drain available host input without blocking the VM execution loop.
    pub fn poll(&self) -> Result<TerminalInput, TerminalError> {
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
                }
            }
        }

        for event in pending {
            match event {
                InputEvent::Bytes(raw) => {
                    let bytes = translate_input_bytes(&raw);
                    input.bytes.extend(&bytes);
                    self.transcript
                        .borrow_mut()
                        .record(TerminalTranscriptEvent::Input { raw, bytes });
                }
                InputEvent::Eof => {
                    let bytes = vec![0x04];
                    input.bytes.extend(&bytes);
                    self.transcript
                        .borrow_mut()
                        .record(TerminalTranscriptEvent::Eof { bytes });
                    self.raw_mode.restore()?
                }
                InputEvent::Error(error) => return Err(TerminalError::Io(error)),
            }
        }

        Ok(input)
    }

    pub fn transcript(&self) -> TerminalTranscript {
        self.transcript.borrow().clone()
    }

    pub fn flush_output(&self) -> Result<(), TerminalError> {
        self.output
            .borrow_mut()
            .flush()
            .map_err(TerminalError::Io)
    }

    pub fn write_output(&self, bytes: &[u8]) -> Result<(), TerminalError> {
        self.output
            .borrow_mut()
            .write_all(bytes)
            .map_err(TerminalError::Io)
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
                    let _ = sender.send(InputEvent::Error(error));
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
                    TerminalError::RawMode(format!("cannot enable raw mode: {error}"))
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
        mode.restore().map_err(|error| {
            TerminalError::RawMode(format!("cannot restore terminal settings: {error}"))
        })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
