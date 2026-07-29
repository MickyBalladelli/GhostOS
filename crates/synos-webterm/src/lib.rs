#![no_std]
#![forbid(unsafe_code)]

//! Heap-free remote terminal frontend and Ring 3 SSH session gateway.
//!
//! The terminal model accepts the same VT byte stream as the serial console.
//! Its fixed cell grid can be compiled to WebAssembly and uploaded directly as
//! WebGPU instances. SSH transport and cryptography stay behind traits so the
//! daemon can use SynOS networking, crypto, and `syn-authd` capabilities.

mod frontend;
mod ssh;
mod terminal;

pub use frontend::{GpuCell, UploadError, UploadRange, WebGpuFrontend};
pub use ssh::{
    AuthenticatedPrincipal, ShellBackend, SshAuthenticator, SshDaemon, SshError, SshSessionId,
    TerminalSize,
};
pub use terminal::{Cell, CellAttributes, Color, Cursor, Terminal, TerminalError, TerminalModes};
