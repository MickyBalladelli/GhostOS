//! Host terminal session for the guest serial console.

use std::cell::Cell;
use std::io::{self, IsTerminal, Read, Write};
use std::sync::mpsc::{self, Receiver};

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalExit {
    GuestShutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalInputMode {
    Serial,
    Ps2,
}

/// Owns host input polling and terminal restoration for one VM session.
pub struct TerminalSession {
    events: Receiver<InputEvent>,
    _raw_mode: RawMode,
    has_terminal: bool,
    last_size: Cell<Option<(u16, u16)>>,
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
            RawMode::Inactive
        };

        let (sender, events) = mpsc::channel();
        std::thread::spawn(move || {
            let mut input = io::stdin().lock();
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

        Ok(Self {
            events,
            _raw_mode: raw_mode,
            has_terminal: is_tty,
            last_size: Cell::new(None),
        })
    }

    /// Drain available host input without blocking the VM execution loop.
    pub fn poll(&self) -> Result<TerminalInput, TerminalError> {
        self.poll_for_mode(TerminalInputMode::Serial)
    }

    pub fn poll_for_mode(
        &self,
        input_mode: TerminalInputMode,
    ) -> Result<TerminalInput, TerminalError> {
        let mut input = TerminalInput::default();

        while let Ok(event) = self.events.try_recv() {
            match event {
                InputEvent::Bytes(bytes) => translate_input(&bytes, &mut input),
                InputEvent::Eof => {
                    input.bytes.push(0x04)
                }
                InputEvent::Error(error) => return Err(TerminalError::Io(error)),
            }
        }

        if input_mode == TerminalInputMode::Serial
            && self.has_terminal
            && (!input.bytes.is_empty() || self.last_size.get().is_none())
        {
            if let Some((rows, columns)) = terminal_size() {
                let size = (rows, columns);
                if self.last_size.get() != Some(size) {
                    let mut resized = format!("\x1b[8;{};{}t", rows, columns).into_bytes();
                    resized.extend(input.bytes);
                    input.bytes = resized;
                    self.last_size.set(Some(size));
                }
            }
        }

        Ok(input)
    }

    pub fn flush_output(&self) -> Result<(), TerminalError> {
        io::stdout().flush().map_err(TerminalError::Io)
    }
}

fn translate_input(bytes: &[u8], input: &mut TerminalInput) {
    for &byte in bytes {
        match byte {
            // Ctrl-C belongs to the guest shell. It cancels the current input
            // without stopping the VM.
            0x03 => input.bytes.push(0x03),
            // TTYs commonly report Backspace as DEL; SynOS serial consoles
            // conventionally consume BS.
            0x7F => input.bytes.push(0x08),
            // Enter, Tab, Ctrl-D, and escape sequences pass through.
            byte => input.bytes.push(byte),
        }
    }
}

#[cfg(unix)]
fn terminal_size() -> Option<(u16, u16)> {
    let output = stty_command().arg("size").output().ok()?;
    if !output.status.success() {
        return None
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let mut values = text.split_whitespace().map(|value| value.parse::<u16>().ok());
    Some((values.next()??, values.next()??))
}

#[cfg(not(unix))]
fn terminal_size() -> Option<(u16, u16)> {
    None
}

/// Convert one ASCII byte to PS/2 set-1 make/break bytes.
pub fn ascii_to_scancodes(byte: u8) -> Vec<u8> {
    let (byte, control) = if (1..=26).contains(&byte) {
        (b'a' + byte - 1, true)
    } else {
        (byte, false)
    };
    let (code, shift) = match byte {
        b'a'..=b'z' => {
            let codes = [
                0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24,
                0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1F, 0x14,
                0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
            ];
            (codes[(byte - b'a') as usize], false)
        }
        b'A'..=b'Z' => {
            let codes = [
                0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24,
                0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1F, 0x14,
                0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
            ];
            (codes[(byte - b'A') as usize], true)
        }
        b'1' => (0x02, false),
        b'2' => (0x03, false),
        b'3' => (0x04, false),
        b'4' => (0x05, false),
        b'5' => (0x06, false),
        b'6' => (0x07, false),
        b'7' => (0x08, false),
        b'8' => (0x09, false),
        b'9' => (0x0A, false),
        b'0' => (0x0B, false),
        b'!' => (0x02, true),
        b'@' => (0x03, true),
        b'#' => (0x04, true),
        b'$' => (0x05, true),
        b'%' => (0x06, true),
        b'^' => (0x07, true),
        b'&' => (0x08, true),
        b'*' => (0x09, true),
        b'(' => (0x0A, true),
        b')' => (0x0B, true),
        b' ' => (0x39, false),
        b'\n' | b'\r' => (0x1C, false),
        b'\t' => (0x0F, false),
        0x08 | 0x7F => (0x0E, false),
        b'-' => (0x0C, false),
        b'_' => (0x0C, true),
        b'=' => (0x0D, false),
        b'+' => (0x0D, true),
        b'[' => (0x1A, false),
        b'{' => (0x1A, true),
        b']' => (0x1B, false),
        b'}' => (0x1B, true),
        b'\\' => (0x2B, false),
        b'|' => (0x2B, true),
        b';' => (0x27, false),
        b':' => (0x27, true),
        b'\'' => (0x28, false),
        b'"' => (0x28, true),
        b',' => (0x33, false),
        b'<' => (0x33, true),
        b'.' => (0x34, false),
        b'>' => (0x34, true),
        b'/' => (0x35, false),
        b'?' => (0x35, true),
        b'`' => (0x29, false),
        b'~' => (0x29, true),
        _ => return Vec::new(),
    };

    let mut result = Vec::with_capacity(if shift || control { 4 } else { 2 });
    if shift {
        result.push(0x2A)
    }
    if control {
        result.push(0x1D)
    }
    result.push(code);
    result.push(code | 0x80);
    if control {
        result.push(0x9D)
    }
    if shift {
        result.push(0xAA)
    }
    result
}

enum RawMode {
    Inactive,
    #[cfg(unix)]
    Unix { saved: String },
}

impl RawMode {
    #[cfg(unix)]
    fn enter() -> Result<Self, TerminalError> {
        let saved = stty_command()
            .arg("-g")
            .output()
            .map_err(TerminalError::Io)?;
        if !saved.status.success() {
            return Err(TerminalError::RawMode(stty_error(
                "cannot read terminal settings",
                &saved.stderr,
            )))
        }
        let saved = String::from_utf8_lossy(&saved.stdout).trim().to_string();
        let status = stty_command()
            .args(["raw", "-echo", "min", "1", "time", "0"])
            .status()
            .map_err(TerminalError::Io)?;
        if !status.success() {
            return Err(TerminalError::RawMode("cannot enable raw mode".to_string()))
        }
        Ok(Self::Unix { saved })
    }

    #[cfg(not(unix))]
    fn enter() -> Result<Self, TerminalError> {
        Ok(Self::Inactive)
    }
}

#[cfg(unix)]
fn stty_command() -> std::process::Command {
    let mut command = std::process::Command::new("stty");
    #[cfg(target_os = "macos")]
    command.args(["-f", "/dev/tty"]);
    command
}

#[cfg(unix)]
fn stty_error(context: &str, stderr: &[u8]) -> String {
    let detail = String::from_utf8_lossy(stderr).trim().to_string();
    if detail.is_empty() {
        context.to_string()
    } else {
        format!("{context}: {detail}")
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Self::Unix { saved } = self {
            let _ = stty_command().arg(saved).status();
        }
    }
}
