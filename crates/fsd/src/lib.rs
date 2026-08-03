#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
extern crate alloc;

mod daemon;
mod namespace;
mod protocol;

#[cfg(test)]
mod tests;

pub use daemon::{
    Daemon, DaemonError, DirectoryRemovalInfo, FileInfo, FileRights, GcReport, MountId, MountInfo, ProcessRights,
    SnapshotInfo, DEFAULT_MAX_MOUNTS, DEFAULT_MAX_OPEN_FILES, DEFAULT_MAX_PROCESSES,
    DEFAULT_MAX_SNAPSHOTS, DEFAULT_SCRATCH_BYTES,
};
pub use synos_path_pattern::{Pattern, PatternError};
pub use namespace::{
    HostMountAuthority, MountCapability, MountInfo as NamespaceMountInfo, MountSource, Namespace,
    NamespaceError, NamespaceMountId, NamespacePath, RootActivation, RootFilesystem, SynFsVolume,
    DEFAULT_NAMESPACE_MOUNTS, LOGS_PATH, MAX_NAMESPACE_PATH_BYTES, PACKAGE_STORE_PATH, ROOT_PATH,
    TEMPORARY_PATH, USER_DATA_PATH,
};
pub use protocol::{
    Capability, Flags, Operation, ProcessId, ProtocolError, Request, Response,
    MAX_IPC_BUFFER_BYTES, REQUEST_LABEL, RESPONSE_LABEL,
};
