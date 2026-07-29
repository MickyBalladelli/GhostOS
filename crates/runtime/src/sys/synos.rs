//! SynOS platform layer used by the native Rust `std` port.
//!
//! This module is intentionally capability-first. Files, threads, mappings,
//! waits, and IPC endpoints are handles supplied by the process loader. The
//! standard library PAL can implement its file, thread, time, and futex
//! primitives without exposing a global POSIX syscall namespace.

pub use crate::abi::{Capability, GateFn, NativeGate, Operation, Request, Response, SystemCall};
pub use crate::fs::{File, Metadata, OpenOptions};
pub use crate::ipc::{IpcAccess, IpcMapping};
pub use crate::thread::{Thread, ThreadStart, WaitWord};
pub use crate::{Error, Runtime};
pub use synos_ipc::{
    ChannelId, Envelope, Ring, RingError, SharedArena, SharedBuffer, SharedRegionId, SharedView,
};
