//! Host terminal session for the guest serial console.

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
    pub host_interrupt: bool,
    pub eof: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalExit {
    HostInterrupt,
    Eof,
    GuestShutdown,
    GuestReboot,
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
        })
    }

    /// Drain available host input without blocking the VM execution loop.
    pub fn poll(&self) -> Result<TerminalInput, TerminalError> {
        let mut input = TerminalInput::default();

        while let Ok(event) = self.events.try_recv() {
            match event {
                InputEvent::Bytes(bytes) => translate_input(&bytes, &mut input),
                InputEvent::Eof => {
                    input.eof = true;
                    input.bytes.push(0x04)
                }
                InputEvent::Error(error) => return Err(TerminalError::Io(error)),
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
            // Ctrl-C is a host escape. It never reaches SynOS.
            0x03 => input.host_interrupt = true,
            // TTYs commonly report Backspace as DEL; SynOS serial consoles
            // conventionally consume BS.
            0x7F => input.bytes.push(0x08),
            // Enter, Tab, Ctrl-D, and escape sequences pass through.
            byte => input.bytes.push(byte),
        }
    }
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
        use std::process::Command;

        let saved = Command::new("stty")
            .arg("-g")
            .output()
            .map_err(TerminalError::Io)?;
        if !saved.status.success() {
            return Err(TerminalError::RawMode("cannot read terminal settings".to_string()))
        }
        let saved = String::from_utf8_lossy(&saved.stdout).trim().to_string();
        let status = Command::new("stty")
            .args(["raw", "-echo", "min", "0", "time", "0"])
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

impl Drop for RawMode {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Self::Unix { saved } = self {
            let _ = std::process::Command::new("stty").arg(saved).status();
        }
    }
}
