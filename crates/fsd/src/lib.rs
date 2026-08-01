#![no_std]
#![forbid(unsafe_code)]

mod daemon;
mod protocol;

pub use daemon::{
    DEFAULT_MAX_MOUNTS, DEFAULT_MAX_OPEN_FILES, DEFAULT_MAX_PROCESSES, DEFAULT_MAX_SNAPSHOTS,
    DEFAULT_SCRATCH_BYTES, Daemon, DaemonError, FileInfo, FileRights, GcReport, MountId, MountInfo,
    ProcessRights, SnapshotInfo,
};
pub use protocol::{
    Capability, Flags, MAX_IPC_BUFFER_BYTES, Operation, ProcessId, ProtocolError, REQUEST_LABEL,
    RESPONSE_LABEL, Request, Response,
};
