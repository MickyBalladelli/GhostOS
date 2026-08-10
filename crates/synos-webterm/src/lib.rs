#![no_std]
#![forbid(unsafe_code)]

//! Heap-free remote terminal frontend and Ring 3 SSH session gateway.
//!
//! The terminal model accepts the same VT byte stream as the serial console.
//! Its fixed cell grid can be compiled to WebAssembly and uploaded directly as
//! WebGPU instances. SSH transport and cryptography stay behind traits so the
//! daemon can use SynOS networking, crypto, and `syn-authd` capabilities.

pub use synos_protocol::{ProtocolError, ProtocolGuard, ProtocolLimits, TrafficClass, VersionRange};

pub fn validate_remote_terminal_message(
    guard: &mut ProtocolGuard,
    sequence: u64,
    bytes: usize,
) -> Result<(), ProtocolError> {
    guard.require_class(TrafficClass::RemoteTerminal)?;
    guard.validate_message(bytes)?;
    guard.accept_sequence(sequence)
}

mod frontend;
mod ssh;
mod terminal;

pub use frontend::{GpuCell, UploadError, UploadRange, WebGpuFrontend};
pub use ssh::{
    AuthenticatedPrincipal, ShellBackend, SshAuthenticator, SshDaemon, SshError, SshSessionId,
    TerminalSize,
};
pub use terminal::{Cell, CellAttributes, Color, Cursor, Terminal, TerminalError, TerminalModes};
