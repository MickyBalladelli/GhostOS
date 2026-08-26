use core::fmt;

use host_filesystems::{FileSystemKind, Partition};
use ghostos_ipc::{Envelope, SharedBuffer};
use ghostos_observability::{
    CapabilityDomain, CapabilityTrace, CapabilityTraceStage, Level, ProfileDomain, ProfileSample,
    record_profile_sample,
};
use ghostos_path_pattern::{Pattern, PatternError};
use ghostos_status::{facility, IntoStatus, Severity, Status};
use ghostos_ghostfs::{
    CheckpointInfo, DirectoryEntry, Error as SynFsError, FileType, LinkEntry, SynFs,
    SynFsDiagnostics, SynFsTransaction, TransactionCommit, VersionSelector, VersionedPath,
};

use crate::namespace::{
    HostMountAuthority, MountCapability, MountSource, Namespace, NamespaceError, RootActivation,
    RootFilesystem,
};

use crate::protocol::{
    Capability, Flags, LockMode, LockRange, Operation, ProcessId, ProtocolError, Request,
    Response, MAX_IPC_BUFFER_BYTES,
};

pub const DEFAULT_MAX_PROCESSES: usize = 64;
pub const DEFAULT_MAX_OPEN_FILES: usize = 256;
pub const DEFAULT_MAX_SNAPSHOTS: usize = 16;
pub const DEFAULT_MAX_MOUNTS: usize = 16;
pub const DEFAULT_SCRATCH_BYTES: usize = MAX_IPC_BUFFER_BYTES;
const MAX_DIRECTORY_ENTRIES: usize = 256;
const MAX_NAME_BYTES: usize = ghostos_ghostfs::MAX_PATH_BYTES;
const ROOT_MOUNT_NAME: &str = "SYS$ROOT";
const INTERNAL_MAPPING_CAPABILITY: u64 = 1 << 32;
const MAPPING_CAPABILITY_BIT: u32 = 1 << 31;
const MAPPED_FILE_PAGE_SIZE: u64 = 4096;

fn pattern_error(error: PatternError) -> DaemonError {
    match error {
        PatternError::InvalidPath => DaemonError::InvalidPath,
        PatternError::TooLong => DaemonError::File(SynFsError::BufferTooSmall {
            required: ghostos_path_pattern::MAX_PATTERN_BYTES.saturating_add(1),
        }),
        PatternError::Empty
        | PatternError::TrailingEscape
        | PatternError::UnterminatedClass
        | PatternError::EmptyClass
        | PatternError::InvalidRange => DaemonError::InvalidPattern,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct FileRights(u16);

impl FileRights {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const DELETE: Self = Self(1 << 2);
    pub const ADMIN: Self = Self(1 << 3);
    const TRAVERSE: Self = Self(1 << 4);

    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn from_flags(flags: Flags) -> Self {
        let mut rights = Self(0);
        if flags.contains(Flags::READ) {
            rights = rights.union(Self::READ)
        }
        if flags.contains(Flags::WRITE) {
            rights = rights.union(Self::WRITE)
        }
        if flags.contains(Flags::DELETE) {
            rights = rights.union(Self::DELETE)
        }
        if flags.contains(Flags::ADMIN) {
            rights = rights.union(Self::ADMIN)
        }
        rights
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ProcessRights(u16);

impl ProcessRights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const DELETE: Self = Self(1 << 2);
    pub const ADMIN: Self = Self(1 << 3);

    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    const fn contains_file(self, rights: FileRights) -> bool {
        let process = Self(rights.bits());
        self.contains(process)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileInfo {
    pub capability: Capability,
    pub file: ghostos_ghostfs::FileName,
    pub version: u32,
    pub size: u64,
    pub checksum: u64,
    pub created_at: u64,
    pub rights: FileRights,
    pub file_type: FileType,
    pub link_count: u32,
    pub mode: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileMappingInfo {
    pub capability: Capability,
    pub file: Capability,
    pub offset: u64,
    pub length: u64,
    pub writable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LockInfo {
    pub capability: Capability,
    pub file: Capability,
    pub range: LockRange,
    pub mode: LockMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryRemovalInfo {
    pub file: FileInfo,
    pub removal_generation: u64,
    pub storage_reclamation_pending: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeleteInfo {
    pub file: FileInfo,
    pub shared_data_reachable: bool,
    pub status: Status,
    pub matched_count: usize,
    pub processed_count: usize,
    pub failed_count: usize,
    pub failure_status: Option<Status>,
    pub cancelled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotInfo {
    pub capability: Capability,
    pub generation: u64,
    pub owner: ProcessId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct MountId(u32);

impl MountId {
    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MountInfo {
    pub id: MountId,
    pub name: [u8; MAX_NAME_BYTES],
    pub name_length: u16,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GcReport {
    pub live_blocks: usize,
    pub freed_blocks: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DaemonError {
    Protocol(ProtocolError),
    InvalidRequest,
    ProcessNotRegistered,
    AccessDenied,
    InvalidCapability,
    CapabilityExhausted,
    HandleExhausted,
    SnapshotExhausted,
    MountExhausted,
    NotFound,
    ReadOnly,
    CrossVolume,
    PartialMatch,
    BufferTooSmall { required: usize },
    InvalidPath,
    InvalidPattern,
    InvalidRename,
    ScratchTooSmall { required: usize },
    LockBusy,
    LockExhausted,
    InvalidLock,
    Namespace(NamespaceError),
    File(SynFsError),
}

impl From<SynFsError> for DaemonError {
    fn from(error: SynFsError) -> Self {
        Self::File(error)
    }
}

impl From<ProtocolError> for DaemonError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

impl IntoStatus for DaemonError {
    fn status(self) -> Status {
        match self {
            Self::Protocol(_) | Self::InvalidRequest | Self::InvalidRename => {
                Status::INVALID_ARGUMENT
            }
            Self::InvalidPath => Status::INVALID_PATH,
            Self::InvalidPattern => Status::INVALID_PATTERN,
            Self::ProcessNotRegistered
            | Self::AccessDenied
            | Self::InvalidCapability => Status::ACCESS_DENIED,
            Self::ReadOnly => Status::READ_ONLY,
            Self::CrossVolume => Status::INVALID_ARGUMENT,
            Self::PartialMatch => Status::PARTIAL_MATCH,
            Self::CapabilityExhausted
            | Self::HandleExhausted
            | Self::SnapshotExhausted
            | Self::MountExhausted
            | Self::ScratchTooSmall { .. } => Status::NO_SPACE,
            Self::LockExhausted => Status::NO_SPACE,
            Self::LockBusy => Status::BUSY,
            Self::InvalidLock => Status::INVALID_ARGUMENT,
            Self::NotFound => Status::NOT_FOUND,
            Self::BufferTooSmall { .. } => Status::new(Severity::Error, facility::FILESYSTEM, 1, 0)
                .unwrap_or(Status::INVALID_ARGUMENT),
            Self::Namespace(error) => match error {
                NamespaceError::InvalidPath => Status::INVALID_PATH,
                NamespaceError::InvalidPartition => Status::INVALID_ARGUMENT,
                NamespaceError::Capacity => Status::NO_SPACE,
                NamespaceError::InvalidCapability | NamespaceError::AccessDenied => {
                    Status::ACCESS_DENIED
                }
                NamespaceError::AlreadyMounted => Status::INVALID_ARGUMENT,
                NamespaceError::RootBusy => Status::ACCESS_DENIED,
                NamespaceError::Inactive
                | NamespaceError::AlreadyActive
                | NamespaceError::NotFound => Status::NOT_FOUND,
            },
            Self::File(error) => error.status(),
        }
    }
}

impl fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Protocol(_) => "filesystem protocol error",
            Self::InvalidRequest => "invalid filesystem request",
            Self::ProcessNotRegistered => "process is not registered",
            Self::AccessDenied => "filesystem capability denied",
            Self::InvalidCapability => "invalid filesystem capability",
            Self::CapabilityExhausted => "process capability table is full",
            Self::HandleExhausted => "open file table is full",
            Self::SnapshotExhausted => "snapshot table is full",
            Self::MountExhausted => "mount table is full",
            Self::NotFound => "filesystem object not found",
            Self::ReadOnly => "filesystem is read-only",
            Self::CrossVolume => "hard links cannot cross volumes",
            Self::PartialMatch => "wildcard operation matched only part of the input",
            Self::BufferTooSmall { .. } => "shared buffer is too small",
            Self::InvalidPath => "invalid filesystem path",
            Self::InvalidPattern => "malformed wildcard pattern",
            Self::InvalidRename => "invalid rename payload",
            Self::ScratchTooSmall { .. } => "daemon scratch space is too small",
            Self::LockBusy => "filesystem lock is busy",
            Self::LockExhausted => "filesystem lock table is full",
            Self::InvalidLock => "invalid filesystem lock",
            Self::Namespace(_) => "invalid filesystem namespace operation",
            Self::File(_) => "GhostFS operation failed",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Name {
    bytes: [u8; MAX_NAME_BYTES],
    length: u16,
}

impl Name {
    const EMPTY: Self = Self {
        bytes: [0; MAX_NAME_BYTES],
        length: 0,
    };

    fn from_bytes(bytes: &[u8], allow_empty: bool) -> Result<Self, DaemonError> {
        if (!allow_empty && bytes.is_empty())
            || bytes.len() > MAX_NAME_BYTES
            || bytes.contains(&0)
            || bytes.ends_with(b"/")
            || bytes.windows(2).any(|pair| pair == b"//")
        {
            return Err(DaemonError::InvalidPath);
        }
        core::str::from_utf8(bytes).map_err(|_| DaemonError::InvalidPath)?;
        let mut name = Self::EMPTY;
        name.bytes[..bytes.len()].copy_from_slice(bytes);
        name.length = bytes.len() as u16;
        Ok(name)
    }

    fn from_str(value: &str) -> Result<Self, DaemonError> {
        Self::from_bytes(value.as_bytes(), false)
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length as usize]
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or("")
    }
}

#[derive(Clone, Copy)]
struct ProcessSlot {
    occupied: bool,
    generation: u32,
    process: Option<ProcessId>,
    rights: ProcessRights,
}

impl ProcessSlot {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        process: None,
        rights: ProcessRights::NONE,
    };
}

#[derive(Clone, Copy)]
struct OpenFileSlot {
    occupied: bool,
    generation: u32,
    owner: ProcessId,
    path: Name,
    rights: FileRights,
    read_only_mount: bool,
    append: bool,
}

impl OpenFileSlot {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        owner: ProcessId::from_valid_raw(1),
        path: Name::EMPTY,
        rights: FileRights(0),
        read_only_mount: false,
        append: false,
    };
}

#[derive(Clone, Copy)]
struct MappingSlot {
    occupied: bool,
    generation: u32,
    owner: ProcessId,
    file: Capability,
    path: Name,
    offset: u64,
    length: u64,
    writable: bool,
}

impl MappingSlot {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        owner: ProcessId::from_valid_raw(1),
        file: Capability::from_valid_raw(1),
        path: Name::EMPTY,
        offset: 0,
        length: 0,
        writable: false,
    };
}

#[derive(Clone, Copy)]
struct LockSlot {
    occupied: bool,
    generation: u32,
    owner: ProcessId,
    file: Capability,
    path: Name,
    range: LockRange,
    mode: LockMode,
}

impl LockSlot {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        owner: ProcessId::from_valid_raw(1),
        file: Capability::from_valid_raw(1),
        path: Name::EMPTY,
        range: LockRange::WholeFile,
        mode: LockMode::Shared,
    };
}

#[derive(Clone, Copy)]
struct SnapshotSlot {
    occupied: bool,
    generation: u32,
    owner: ProcessId,
    checkpoint: CheckpointInfo,
}

impl SnapshotSlot {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        owner: ProcessId::from_valid_raw(1),
        checkpoint: CheckpointInfo {
            id: ghostos_ghostfs::CheckpointId::from_valid_raw(1),
            generation: 0,
        },
    };
}

#[derive(Clone, Copy)]
struct MountSlot {
    occupied: bool,
    generation: u32,
    id: MountId,
    name: Name,
    read_only: bool,
}

impl MountSlot {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        id: MountId(0),
        name: Name::EMPTY,
        read_only: false,
    };
}

/// Ring 3 owner of a GhostFS volume.
///
/// The daemon owns the mutable GhostFS root and all externally visible handles.
/// Kernel IPC validates the shared mapping before this type is called; this
/// type validates the descriptor length, operation shape, process authority,
/// and daemon-issued capability generation again.
pub struct Daemon<
    const MAX_BLOCKS: usize,
    const MAX_PROCESSES: usize = DEFAULT_MAX_PROCESSES,
    const MAX_OPEN_FILES: usize = DEFAULT_MAX_OPEN_FILES,
    const MAX_SNAPSHOTS: usize = DEFAULT_MAX_SNAPSHOTS,
    const MAX_MOUNTS: usize = DEFAULT_MAX_MOUNTS,
    const SCRATCH_BYTES: usize = DEFAULT_SCRATCH_BYTES,
> {
    filesystem: SynFs<MAX_BLOCKS>,
    processes: [ProcessSlot; MAX_PROCESSES],
    open_files: [OpenFileSlot; MAX_OPEN_FILES],
    mappings: [MappingSlot; MAX_OPEN_FILES],
    locks: [LockSlot; MAX_OPEN_FILES],
    snapshots: [SnapshotSlot; MAX_SNAPSHOTS],
    mounts: [MountSlot; MAX_MOUNTS],
    next_mount_id: u32,
    namespace: Namespace<MAX_MOUNTS>,
    root_activation: RootActivation,
    scratch: [u8; SCRATCH_BYTES],
}

impl<
        const MAX_BLOCKS: usize,
        const MAX_PROCESSES: usize,
        const MAX_OPEN_FILES: usize,
        const MAX_SNAPSHOTS: usize,
        const MAX_MOUNTS: usize,
        const SCRATCH_BYTES: usize,
    > Daemon<MAX_BLOCKS, MAX_PROCESSES, MAX_OPEN_FILES, MAX_SNAPSHOTS, MAX_MOUNTS, SCRATCH_BYTES>
{
    pub fn new(mut filesystem: SynFs<MAX_BLOCKS>) -> Result<Self, DaemonError> {
        for path in ["/packages", "/logs", "/data", "/tmp"] {
            match filesystem.lookup(path) {
                Ok(metadata) if metadata.file_type != FileType::Directory => {
                    return Err(DaemonError::File(SynFsError::NotDirectory))
                }
                Err(SynFsError::NotFound) => {
                    filesystem.create_directory(path, true)?;
                }
                Ok(_) => {}
                Err(error) => return Err(DaemonError::File(error)),
            }
        }
        let root = Name::from_str(ROOT_MOUNT_NAME)?;
        let mut mounts = [MountSlot::EMPTY; MAX_MOUNTS];
        if MAX_MOUNTS == 0 {
            return Err(DaemonError::MountExhausted);
        }
        mounts[0] = MountSlot {
            occupied: true,
            generation: 1,
            id: MountId(1),
            name: root,
            read_only: false,
        };
        let mut namespace = Namespace::new();
        let root_activation = RootFilesystem::new()
            .activate(&mut namespace, filesystem.generation())
            .map_err(|_| DaemonError::MountExhausted)?;
        Ok(Self {
            filesystem,
            processes: [ProcessSlot::EMPTY; MAX_PROCESSES],
            open_files: [OpenFileSlot::EMPTY; MAX_OPEN_FILES],
            mappings: [MappingSlot::EMPTY; MAX_OPEN_FILES],
            locks: [LockSlot::EMPTY; MAX_OPEN_FILES],
            snapshots: [SnapshotSlot::EMPTY; MAX_SNAPSHOTS],
            mounts,
            next_mount_id: 2,
            namespace,
            root_activation,
            scratch: [0; SCRATCH_BYTES],
        })
    }

    pub fn filesystem(&self) -> &SynFs<MAX_BLOCKS> {
        &self.filesystem
    }

    pub fn filesystem_mut(&mut self) -> &mut SynFs<MAX_BLOCKS> {
        &mut self.filesystem
    }

    /// Namespace and root mounts are activated as part of daemon startup.
    pub fn namespace(&self) -> &Namespace<MAX_MOUNTS> {
        &self.namespace
    }

    pub fn root_activation(&self) -> RootActivation {
        self.root_activation
    }

    /// Mount a host partition with the boot-issued host-mount authority.
    /// Host filesystem sources are always read-only.
    pub fn mount_host(
        &mut self,
        process: ProcessId,
        authority: Capability,
        host_authority: HostMountAuthority,
        path: &str,
        filesystem: FileSystemKind,
        partition: Partition,
    ) -> Result<(crate::namespace::MountInfo, MountCapability), DaemonError> {
        self.authorize_process(process, authority, FileRights::ADMIN)?;
        self.namespace
            .mount_host(host_authority, path, filesystem, partition)
            .map_err(DaemonError::Namespace)
    }

    pub fn unmount_host(
        &mut self,
        process: ProcessId,
        authority: Capability,
        mount: MountCapability,
    ) -> Result<(), DaemonError> {
        self.authorize_process(process, authority, FileRights::ADMIN)?;
        self.namespace.unmount(mount).map_err(DaemonError::Namespace)
    }

    pub fn register_process(
        &mut self,
        process: ProcessId,
        rights: ProcessRights,
    ) -> Result<Capability, DaemonError> {
        if let Some((index, slot)) = self
            .processes
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.occupied && slot.process == Some(process))
        {
            slot.rights = rights;
            return Ok(token(index, slot.generation));
        }
        let (index, slot) = self
            .processes
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| !slot.occupied)
            .ok_or(DaemonError::CapabilityExhausted)?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.occupied = true;
        slot.process = Some(process);
        slot.rights = rights;
        let capability = token(index, slot.generation);
        if let Some(trace) = CapabilityTrace::new(
            CapabilityDomain::Filesystem,
            CapabilityTraceStage::Created,
            capability.raw(),
            Operation::Open.raw(),
        ) {
            trace.emit(Level::Info)
        }
        Ok(capability)
    }

    pub fn unregister_process(&mut self, process: ProcessId) -> Result<(), DaemonError> {
        let index = self.process_index(process, None)?;
        let capability = token(index, self.processes[index].generation);
        for file in &mut self.open_files {
            if file.occupied && file.owner == process {
                file.occupied = false
            }
        }
        for mapping in &mut self.mappings {
            if mapping.occupied && mapping.owner == process {
                mapping.occupied = false
            }
        }
        for lock in &mut self.locks {
            if lock.occupied && lock.owner == process {
                lock.occupied = false
            }
        }
        for index in 0..self.snapshots.len() {
            if self.snapshots[index].occupied && self.snapshots[index].owner == process {
                let checkpoint = self.snapshots[index].checkpoint;
                let _ = self.filesystem.release_checkpoint(checkpoint.id);
                self.snapshots[index].occupied = false
            }
        }
        self.processes[index].occupied = false;
        self.processes[index].process = None;
        if let Some(trace) = CapabilityTrace::new(
            CapabilityDomain::Filesystem,
            CapabilityTraceStage::Revoked,
            capability.raw(),
            Operation::Close.raw(),
        ) {
            trace.emit(Level::Info)
        }
        Ok(())
    }

    pub fn open(
        &mut self,
        process: ProcessId,
        authority: Capability,
        path: &str,
        flags: Flags,
    ) -> Result<FileInfo, DaemonError> {
        let requested = FileRights::from_flags(flags);
        if requested.bits() == 0 || !self.authorize_process(process, authority, requested)? {
            return Err(DaemonError::AccessDenied);
        }
        let name = Name::from_str(path)?;
        if path.contains(';') && requested.contains(FileRights::WRITE) {
            return Err(DaemonError::InvalidPath);
        }
        let namespace_read_only = path
            .starts_with('/')
            .then(|| self.namespace.is_read_only(mount_path(path)))
            .transpose()
            .map_err(DaemonError::Namespace)?
            .unwrap_or(false);
        let read_only_mount = self.mount_is_read_only(path) || namespace_read_only;
        if read_only_mount && requested.contains(FileRights::WRITE) {
            return Err(DaemonError::ReadOnly);
        }
        if flags.contains(Flags::CREATE) && flags.contains(Flags::EXCLUSIVE) {
            return self.create_file_with_name(
                process,
                authority,
                name,
                requested,
                read_only_mount,
                true,
            );
        }
        let exists = self.filesystem.lookup(path).is_ok();
        if !exists && !flags.contains(Flags::CREATE) {
            return Err(DaemonError::NotFound);
        }
        if !exists {
            self.ensure_writable(process, authority)?;
            self.check_parent_access(process, path, FileRights::WRITE)?;
            self.filesystem.write(path, &[])?;
            let metadata = self.filesystem.lookup_following(path)?;
            self.check_mode_access(process, metadata.mode, requested)?;
            return self.open_file_handle(
                process,
                Name::from_str(metadata.file.as_str())?,
                requested,
                read_only_mount,
                flags.contains(Flags::APPEND),
                metadata,
            )
        } else {
            let metadata = self.filesystem.lookup_following(path)?;
            self.check_mode_access(process, metadata.mode, requested)?;
            let resolved_name = Name::from_str(metadata.file.as_str())?;
            if flags.contains(Flags::TRUNCATE) {
                self.ensure_writable(process, authority)?;
                self.check_io_lock(process, resolved_name, LockRange::WholeFile, LockMode::Exclusive)?;
                self.filesystem.write(resolved_name.as_str(), &[])?;
            }
            let metadata = self.filesystem.lookup_following(path)?;
            if flags.contains(Flags::CREATE) && metadata.file_type != FileType::Regular {
                return Err(DaemonError::File(SynFsError::NotDirectory));
            }
            self.check_mode_access(process, metadata.mode, requested)?;
            return self.open_file_handle(
                process,
                resolved_name,
                requested,
                read_only_mount,
                flags.contains(Flags::APPEND),
                metadata,
            )
        }
    }

    pub fn create_file(
        &mut self,
        process: ProcessId,
        authority: Capability,
        path: &str,
    ) -> Result<FileInfo, DaemonError> {
        if path.contains(';') {
            return Err(DaemonError::InvalidPath);
        }
        let name = Name::from_str(path)?;
        let read_only_mount = self.path_is_read_only(path)?;
        self.create_file_with_name(
            process,
            authority,
            name,
            FileRights::READ.union(FileRights::WRITE),
            read_only_mount,
            false,
        )
    }

    fn create_file_with_name(
        &mut self,
        process: ProcessId,
        authority: Capability,
        name: Name,
        rights: FileRights,
        read_only_mount: bool,
        exclusive: bool,
    ) -> Result<FileInfo, DaemonError> {
        self.authorize_process(process, authority, FileRights::WRITE)?;
        if read_only_mount {
            return Err(DaemonError::ReadOnly);
        }
        if !self.open_files.iter().any(|slot| !slot.occupied) {
            return Err(DaemonError::HandleExhausted);
        }
        if exclusive && self.filesystem.lookup(name.as_str()).is_ok() {
            return Err(DaemonError::File(SynFsError::AlreadyExists));
        }
        self.check_parent_access(process, name.as_str(), FileRights::WRITE)?;
        self.filesystem.write(name.as_str(), &[])?;
        let metadata = self.filesystem.lookup(name.as_str())?;
        self.check_mode_access(process, metadata.mode, rights)?;
        self.open_file_handle(
            process,
            name,
            rights,
            false,
            false,
            metadata,
        )
    }

    fn open_file_handle(
        &mut self,
        process: ProcessId,
        name: Name,
        rights: FileRights,
        read_only_mount: bool,
        append: bool,
        metadata: ghostos_ghostfs::FileVersion,
    ) -> Result<FileInfo, DaemonError> {
        let (index, slot) = self
            .open_files
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| !slot.occupied)
            .ok_or(DaemonError::HandleExhausted)?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.occupied = true;
        slot.owner = process;
        slot.path = name;
        slot.rights = rights;
        slot.read_only_mount = read_only_mount;
        slot.append = append;
        let capability = token(index, slot.generation);
        Ok(FileInfo {
            capability,
            file: metadata.file,
            version: metadata.version,
            size: metadata.size,
            checksum: metadata.checksum,
            created_at: metadata.created_at,
            rights,
            file_type: metadata.file_type,
            link_count: metadata.link_count,
            mode: metadata.mode,
        })
    }

    fn path_is_read_only(&self, path: &str) -> Result<bool, DaemonError> {
        let namespace_read_only = path
            .starts_with('/')
            .then(|| self.namespace.is_read_only(path))
            .transpose()
            .map_err(DaemonError::Namespace)?
            .unwrap_or(false);
        Ok(self.mount_is_read_only(path) || namespace_read_only)
    }

    pub fn close(&mut self, process: ProcessId, capability: Capability) -> Result<(), DaemonError> {
        let index = self.file_index(process, capability, FileRights(0))?;
        self.open_files[index].occupied = false;
        for mapping in &mut self.mappings {
            if mapping.occupied && mapping.file == capability {
                mapping.occupied = false
            }
        }
        for lock in &mut self.locks {
            if lock.occupied && lock.file == capability {
                lock.occupied = false
            }
        }
        Ok(())
    }

    /// Create a process-owned mapping capability for a regular file range.
    ///
    /// The open file capability is checked again here. Read mappings need
    /// READ; writable mappings need both READ and WRITE. The returned mapping
    /// capability is a separate namespace, so it cannot be used as a file
    /// capability or confused with another open handle.
    pub fn map(
        &mut self,
        process: ProcessId,
        file: Capability,
        offset: u64,
        length: u64,
        writable: bool,
    ) -> Result<FileMappingInfo, DaemonError> {
        if offset % MAPPED_FILE_PAGE_SIZE != 0
            || length == 0
            || length % MAPPED_FILE_PAGE_SIZE != 0
            || length > (u64::from(u32::MAX) << 16)
        {
            return Err(DaemonError::InvalidRequest);
        }
        let required = if writable {
            FileRights::READ.union(FileRights::WRITE)
        } else {
            FileRights::READ
        };
        let file_index = self.file_index(process, file, required)?;
        if writable && self.open_files[file_index].read_only_mount {
            return Err(DaemonError::ReadOnly);
        }
        let path = self.open_files[file_index].path;
        let metadata = self.filesystem.lookup_following(path.as_str())?;
        if metadata.file_type != FileType::Regular
            || offset > metadata.size
            || offset.checked_add(length).is_none()
        {
            return Err(DaemonError::InvalidRequest);
        }
        self.check_mode_access(process, metadata.mode, required)?;
        self.check_io_lock(
            process,
            path,
            LockRange::WholeFile,
            if writable {
                LockMode::Exclusive
            } else {
                LockMode::Shared
            },
        )?;
        let (index, slot) = self
            .mappings
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| !slot.occupied)
            .ok_or(DaemonError::HandleExhausted)?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.occupied = true;
        slot.owner = process;
        slot.file = file;
        slot.path = path;
        slot.offset = offset;
        slot.length = length;
        slot.writable = writable;
        let capability = mapping_token(index, slot.generation);
        Ok(FileMappingInfo {
            capability,
            file,
            offset,
            length,
            writable,
        })
    }

    pub fn unmap(&mut self, process: ProcessId, capability: Capability) -> Result<(), DaemonError> {
        let index = self.mapping_index(process, capability)?;
        self.mappings[index].occupied = false;
        Ok(())
    }

    pub fn lock(
        &mut self,
        process: ProcessId,
        file: Capability,
        range: LockRange,
        mode: LockMode,
    ) -> Result<LockInfo, DaemonError> {
        let required = match mode {
            LockMode::Shared => FileRights::READ,
            LockMode::Exclusive => FileRights::WRITE,
        };
        let file_index = self.file_index(process, file, required)?;
        let path = self.open_files[file_index].path;
        if self.locks.iter().any(|lock| {
            lock.occupied
                && lock.owner != process
                && lock.path == path
                && ranges_overlap(lock.range, range)
                && (lock.mode == LockMode::Exclusive || mode == LockMode::Exclusive)
        }) {
            return Err(DaemonError::LockBusy);
        }
        let (index, slot) = self
            .locks
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| !slot.occupied)
            .ok_or(DaemonError::LockExhausted)?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.occupied = true;
        slot.owner = process;
        slot.file = file;
        slot.path = path;
        slot.range = range;
        slot.mode = mode;
        Ok(LockInfo {
            capability: token(index, slot.generation),
            file,
            range,
            mode,
        })
    }

    pub fn unlock(&mut self, process: ProcessId, capability: Capability) -> Result<(), DaemonError> {
        let index = self.lock_index(process, capability)?;
        self.locks[index].occupied = false;
        Ok(())
    }

    pub fn read(
        &mut self,
        process: ProcessId,
        capability: Capability,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, DaemonError> {
        if destination.len() > MAX_IPC_BUFFER_BYTES {
            return Err(DaemonError::BufferTooSmall {
                required: MAX_IPC_BUFFER_BYTES,
            });
        }
        let index = self.file_index(process, capability, FileRights::READ)?;
        let path = self.open_files[index].path;
        let metadata = self.filesystem.lookup_following(path.as_str())?;
        self.check_mode_access(process, metadata.mode, FileRights::READ)?;
        self.check_io_lock(process, path, LockRange::Record(offset), LockMode::Shared)?;
        Ok(self
            .filesystem
            .read_at(path.as_str(), offset, destination)?
            .bytes_read)
    }

    /// Publish a write in the daemon's GhostFS view. This does not promise
    /// power-loss durability; the mount owner must complete `SynFs::sync`.
    pub fn write(
        &mut self,
        process: ProcessId,
        capability: Capability,
        offset: u64,
        contents: &[u8],
    ) -> Result<usize, DaemonError> {
        if contents.len() > MAX_IPC_BUFFER_BYTES {
            return Err(DaemonError::BufferTooSmall {
                required: MAX_IPC_BUFFER_BYTES,
            });
        }
        let index = self.file_index(process, capability, FileRights::WRITE)?;
        if self.open_files[index].read_only_mount {
            return Err(DaemonError::ReadOnly);
        }
        let path = self.open_files[index].path;
        let metadata = self.filesystem.lookup_following(path.as_str())?;
        self.check_mode_access(process, metadata.mode, FileRights::WRITE)?;
        let offset = if self.open_files[index].append {
            self.filesystem.lookup(path.as_str())?.size
        } else {
            offset
        };
        self.check_io_lock(process, path, LockRange::Record(offset), LockMode::Exclusive)?;
        self.write_range(path.as_str(), offset, contents)?;
        Ok(contents.len())
    }

    pub fn metadata(
        &self,
        process: ProcessId,
        capability: Capability,
    ) -> Result<FileInfo, DaemonError> {
        let index = self.file_index(process, capability, FileRights::READ)?;
        let slot = self.open_files[index];
        let metadata = self.filesystem.lookup(slot.path.as_str())?;
        self.check_mode_access(process, metadata.mode, FileRights::READ)?;
        let link_count = self.filesystem.current_link_count(slot.path.as_str())?;
        Ok(FileInfo {
            capability,
            file: metadata.file,
            version: metadata.version,
            size: metadata.size,
            checksum: metadata.checksum,
            created_at: metadata.created_at,
            rights: slot.rights,
            file_type: metadata.file_type,
            link_count,
            mode: metadata.mode,
        })
    }

    pub fn delete(
        &mut self,
        process: ProcessId,
        capability: Capability,
    ) -> Result<DeleteInfo, DaemonError> {
        let index = self.file_index(process, capability, FileRights::DELETE)?;
        if self.open_files[index].read_only_mount {
            return Err(DaemonError::ReadOnly);
        }
        let path = self.open_files[index].path;
        self.check_parent_access(process, path.as_str(), FileRights::DELETE)?;
        self.check_io_lock(process, path, LockRange::WholeFile, LockMode::Exclusive)?;
        let deleted = self.filesystem.delete(path.as_str())?;
        Ok(DeleteInfo {
            file: FileInfo {
                capability,
                file: deleted.file,
                version: deleted.version,
                size: deleted.size,
                checksum: deleted.checksum,
                created_at: deleted.created_at,
                rights: self.open_files[index].rights,
                file_type: deleted.file_type,
                link_count: deleted.link_count,
                mode: deleted.mode,
            },
            shared_data_reachable: deleted.link_count != 0,
            status: Status::NORMAL,
            matched_count: 1,
            processed_count: 1,
            failed_count: 0,
            failure_status: None,
            cancelled: false,
        })
    }

    pub fn delete_path(
        &mut self,
        process: ProcessId,
        authority: Capability,
        path: &str,
    ) -> Result<DeleteInfo, DaemonError> {
        self.authorize_process(
            process,
            authority,
            FileRights::DELETE.union(FileRights::WRITE).union(FileRights::ADMIN),
        )?;
        let path = Name::from_str(path)?;
        let versioned = VersionedPath::parse(path.as_str()).map_err(|_| DaemonError::InvalidPath)?;
        let pattern = Pattern::parse(versioned.file.as_str()).map_err(pattern_error)?;
        let mut matches = [None; MAX_DIRECTORY_ENTRIES];
        let mut failure_status = None;
        let match_count = if pattern.has_magic() {
            match self.filesystem.expand_paths(path.as_str(), &mut matches) {
                Ok(count) => count,
                Err(error @ SynFsError::BufferTooSmall { .. }) => {
                    let count = matches.iter().flatten().count();
                    failure_status = Some(error.status());
                    count
                }
                Err(error) => return Err(DaemonError::File(error)),
            }
        } else {
            matches[0] = Some(versioned.file);
            1
        };
        if match_count == 0 {
            return Err(failure_status.map_or(DaemonError::NotFound, |_| {
                DaemonError::File(SynFsError::BufferTooSmall { required: 1 })
            }))
        }
        let mut deleted = None;
        let mut processed_count = 0;
        for matched in matches[..match_count].iter().flatten() {
            let selected = versioned_name(matched.as_str(), versioned.version)?;
            if self.path_is_read_only(selected.as_str())? {
                failure_status = Some(Status::READ_ONLY);
                break
            }
            self.check_parent_access(process, selected.as_str(), FileRights::DELETE)?;
            match self.filesystem.delete(selected.as_str()) {
                Ok(current) => {
                    deleted = Some(current);
                    processed_count += 1;
                }
                Err(error) if processed_count != 0 => {
                    failure_status = Some(error.status());
                    break
                }
                Err(error) => return Err(DaemonError::File(error)),
            }
        }
        let deleted = deleted.ok_or_else(|| {
            failure_status
                .map_or(DaemonError::NotFound, |status| match status {
                    status if status.raw() == Status::READ_ONLY.raw() => DaemonError::ReadOnly,
                    _ => DaemonError::NotFound,
                })
        })?;
        let status = if failure_status.is_some() {
            Status::PARTIAL_MATCH
        } else {
            Status::NORMAL
        };
        Ok(DeleteInfo {
            file: FileInfo {
                capability: authority,
                file: deleted.file,
                version: deleted.version,
                size: deleted.size,
                checksum: deleted.checksum,
                created_at: deleted.created_at,
                rights: FileRights::DELETE,
                file_type: deleted.file_type,
                link_count: deleted.link_count,
                mode: deleted.mode,
            },
            shared_data_reachable: deleted.link_count != 0,
            status,
            matched_count: match_count,
            processed_count,
            failed_count: if failure_status.is_some() { 1 } else { 0 },
            failure_status,
            cancelled: false,
        })
    }

    pub fn create_directory(
        &mut self,
        process: ProcessId,
        authority: Capability,
        path: &str,
        recursive: bool,
    ) -> Result<FileInfo, DaemonError> {
        self.authorize_process(
            process,
            authority,
            FileRights::WRITE.union(FileRights::ADMIN),
        )?;
        let path = Name::from_str(path)?;
        if self.mount_is_read_only(path.as_str()) {
            return Err(DaemonError::ReadOnly);
        }
        self.check_parent_access(process, path.as_str(), FileRights::WRITE)?;
        let metadata = self.filesystem.create_directory(path.as_str(), recursive)?;
        Ok(FileInfo {
            capability: authority,
            file: metadata.file,
            version: metadata.version,
            size: metadata.size,
            checksum: metadata.checksum,
            created_at: metadata.created_at,
            rights: FileRights::READ.union(FileRights::WRITE),
            file_type: metadata.file_type,
            link_count: metadata.link_count,
            mode: metadata.mode,
        })
    }

    pub fn remove_directory(
        &mut self,
        process: ProcessId,
        authority: Capability,
        path: &str,
    ) -> Result<DirectoryRemovalInfo, DaemonError> {
        self.authorize_process(
            process,
            authority,
            FileRights::DELETE.union(FileRights::WRITE).union(FileRights::ADMIN),
        )?;
        let path = Name::from_str(path)?;
        if path.as_str() == "/" || self.is_mount_root(path.as_str()) {
            return Err(DaemonError::AccessDenied);
        }
        if self.path_is_read_only(path.as_str())? {
            return Err(DaemonError::ReadOnly);
        }
        self.check_parent_access(process, path.as_str(), FileRights::DELETE)?;
        let removed = self.filesystem.remove_directory(path.as_str())?;
        let removal_generation = self.filesystem.diagnostics()?.generation;
        Ok(DirectoryRemovalInfo {
            file: FileInfo {
                capability: authority,
                file: removed.file,
                version: removed.version,
                size: removed.size,
                checksum: removed.checksum,
                created_at: removed.created_at,
                rights: FileRights::DELETE,
                file_type: removed.file_type,
                link_count: removed.link_count,
                mode: removed.mode,
            },
            removal_generation,
            storage_reclamation_pending: true,
        })
    }

    pub fn link(
        &mut self,
        process: ProcessId,
        capability: Capability,
        new_path: &str,
    ) -> Result<FileInfo, DaemonError> {
        let index = self.file_index(process, capability, FileRights::READ)?;
        self.require_process_rights(
            process,
            FileRights::WRITE.union(FileRights::ADMIN),
        )?;
        if self.open_files[index].read_only_mount {
            return Err(DaemonError::ReadOnly);
        }
        let old_path = self.open_files[index].path;
        self.check_io_lock(process, old_path, LockRange::WholeFile, LockMode::Exclusive)?;
        let new_path = Name::from_str(new_path)?;
        if self.path_is_read_only(new_path.as_str())? {
            return Err(DaemonError::ReadOnly);
        }
        self.check_parent_access(process, new_path.as_str(), FileRights::WRITE)?;
        if !same_volume(
            self.namespace.resolve(mount_path(old_path.as_str())),
            self.namespace.resolve(mount_path(new_path.as_str())),
        ) {
            return Err(DaemonError::CrossVolume);
        }
        let metadata = self.filesystem.link(old_path.as_str(), new_path.as_str())?;
        Ok(FileInfo {
            capability,
            file: metadata.file,
            version: metadata.version,
            size: metadata.size,
            checksum: metadata.checksum,
            created_at: metadata.created_at,
            rights: self.open_files[index].rights,
            file_type: metadata.file_type,
            link_count: metadata.link_count,
            mode: metadata.mode,
        })
    }

    pub fn symlink(
        &mut self,
        process: ProcessId,
        authority: Capability,
        target: &str,
        link_path: &str,
    ) -> Result<FileInfo, DaemonError> {
        self.authorize_process(process, authority, FileRights::WRITE)?;
        let link_path = Name::from_str(link_path)?;
        if self.path_is_read_only(link_path.as_str())? {
            return Err(DaemonError::ReadOnly);
        }
        self.check_parent_access(process, link_path.as_str(), FileRights::WRITE)?;
        let metadata = self.filesystem.symlink(target, link_path.as_str())?;
        Ok(FileInfo {
            capability: authority,
            file: metadata.file,
            version: metadata.version,
            size: metadata.size,
            checksum: metadata.checksum,
            created_at: metadata.created_at,
            rights: FileRights::READ,
            file_type: metadata.file_type,
            link_count: metadata.link_count,
            mode: metadata.mode,
        })
    }

    pub fn read_link(
        &self,
        process: ProcessId,
        authority: Capability,
        path: &str,
        output: &mut [u8],
    ) -> Result<(FileInfo, usize), DaemonError> {
        self.authorize_process(process, authority, FileRights::READ)?;
        let path = Name::from_str(path)?;
        self.check_parent_access(process, path.as_str(), FileRights::READ)?;
        let result = self.filesystem.read_link(path.as_str(), output)?;
        Ok((
            FileInfo {
                capability: authority,
                file: result.file.file,
                version: result.file.version,
                size: result.file.size,
                checksum: result.file.checksum,
                created_at: result.file.created_at,
                rights: FileRights::READ,
                file_type: result.file.file_type,
                link_count: result.file.link_count,
                mode: result.file.mode,
            },
            result.bytes_read,
        ))
    }

    pub fn chmod(
        &mut self,
        process: ProcessId,
        capability: Capability,
        mode: u16,
    ) -> Result<FileInfo, DaemonError> {
        self.require_process_rights(process, FileRights::ADMIN)?;
        let index = self.file_index(process, capability, FileRights(0))?;
        let path = self.open_files[index].path;
        let metadata = self.filesystem.set_mode(path.as_str(), mode)?;
        Ok(FileInfo {
            capability,
            file: metadata.file,
            version: metadata.version,
            size: metadata.size,
            checksum: metadata.checksum,
            created_at: metadata.created_at,
            rights: self.open_files[index].rights,
            file_type: metadata.file_type,
            link_count: metadata.link_count,
            mode: metadata.mode,
        })
    }

    pub fn list_links(
        &self,
        process: ProcessId,
        authority: Capability,
        path: &str,
        output: &mut [u8],
    ) -> Result<(FileInfo, usize), DaemonError> {
        self.authorize_process(process, authority, FileRights::READ)?;
        let path = Name::from_str(path)?;
        let versioned = VersionedPath::parse(path.as_str()).map_err(|_| DaemonError::InvalidPath)?;
        let pattern = Pattern::parse(versioned.file.as_str()).map_err(pattern_error)?;
        let mut matches = [None; MAX_DIRECTORY_ENTRIES];
        let match_count = if pattern.has_magic() {
            match self.filesystem.expand_paths(path.as_str(), &mut matches) {
                Ok(count) => count,
                Err(SynFsError::BufferTooSmall { .. })
                    if matches.iter().any(Option::is_some) =>
                {
                    return Err(DaemonError::PartialMatch)
                }
                Err(error) => return Err(DaemonError::File(error)),
            }
        } else {
            matches[0] = Some(versioned.file);
            1
        };
        if match_count == 0 {
            return Err(DaemonError::NotFound)
        }
        let mut metadata = None;
        let mut links = [LinkEntry::EMPTY; MAX_DIRECTORY_ENTRIES];
        let mut link_count = 0;
        let mut processed_count = 0;
        for matched in matches[..match_count].iter().flatten() {
            let selected = versioned_name(matched.as_str(), versioned.version)?;
            let current = match self.filesystem.lookup(selected.as_str()) {
                Ok(current) => current,
                Err(_error) if processed_count != 0 => return Err(DaemonError::PartialMatch),
                Err(error) => return Err(DaemonError::File(error)),
            };
            metadata = Some(current);
            let mut entries = [LinkEntry::EMPTY; MAX_DIRECTORY_ENTRIES];
            let count = match self.filesystem.list_links(selected.as_str(), &mut entries) {
                Ok(count) => count,
                Err(_error) if processed_count != 0 => return Err(DaemonError::PartialMatch),
                Err(error) => return Err(DaemonError::File(error)),
            };
            for entry in entries.iter().take(count) {
                if insert_link_entry(&mut links, &mut link_count, *entry).is_err() {
                    if processed_count != 0 {
                        return Err(DaemonError::PartialMatch)
                    }
                    return Err(DaemonError::BufferTooSmall { required: link_count + 1 })
                }
            }
            processed_count += 1;
        }
        let metadata = metadata.ok_or(DaemonError::NotFound)?;
        let mut required = 0usize;
        for entry in links.iter().take(link_count) {
            let suffix = if entry.version == 0 { 0 } else { 1 + digits(entry.version) };
            required = required
                .checked_add(entry.path.as_bytes().len() + suffix + 1)
                .ok_or(DaemonError::BufferTooSmall { required: usize::MAX })?;
        }
        if required > output.len() {
            return Err(DaemonError::BufferTooSmall { required })
        }
        let mut written = 0;
        for entry in links.iter().take(link_count) {
            let end = written + entry.path.as_bytes().len();
            output[written..end].copy_from_slice(entry.path.as_bytes());
            written = end;
            if entry.version != 0 {
                output[written] = b';';
                written += 1;
                written += write_decimal(&mut output[written..], entry.version);
            }
            output[written] = b'\n';
            written += 1;
        }
        Ok((
            FileInfo {
                capability: authority,
                file: metadata.file,
                version: metadata.version,
                size: metadata.size,
                checksum: metadata.checksum,
                created_at: metadata.created_at,
                rights: FileRights::READ,
                file_type: metadata.file_type,
                link_count: metadata.link_count,
                mode: metadata.mode,
            },
            written,
        ))
    }

    /// Atomically publish a rename in GhostFS memory. The rename is durable only
    /// after the generation reaches the block-device sync barrier.
    pub fn rename(
        &mut self,
        process: ProcessId,
        capability: Capability,
        new_path: &str,
    ) -> Result<(), DaemonError> {
        let index = self.file_index(
            process,
            capability,
            FileRights::DELETE.union(FileRights::WRITE),
        )?;
        if self.open_files[index].read_only_mount {
            return Err(DaemonError::ReadOnly);
        }
        let old_path = self.open_files[index].path;
        let new_path = Name::from_str(new_path)?;
        self.check_parent_access(process, new_path.as_str(), FileRights::WRITE)?;
        if self.filesystem.lookup(new_path.as_str()).is_ok() {
            return Err(DaemonError::File(SynFsError::AlreadyExists));
        }
        self.filesystem.rename(old_path.as_str(), new_path.as_str())?;
        self.open_files[index].path = new_path;
        Ok(())
    }

    pub fn snapshot_create(
        &mut self,
        process: ProcessId,
        authority: Capability,
    ) -> Result<SnapshotInfo, DaemonError> {
        self.authorize_process(process, authority, FileRights::ADMIN)?;
        let (index, slot) = self
            .snapshots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| !slot.occupied)
            .ok_or(DaemonError::SnapshotExhausted)?;
        let checkpoint = self.filesystem.create_checkpoint()?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.occupied = true;
        slot.owner = process;
        slot.checkpoint = checkpoint;
        Ok(SnapshotInfo {
            capability: token(index, slot.generation),
            generation: checkpoint.generation,
            owner: process,
        })
    }

    pub fn snapshot_release(
        &mut self,
        process: ProcessId,
        capability: Capability,
    ) -> Result<(), DaemonError> {
        let index = self.snapshot_index(process, capability)?;
        let checkpoint = self.snapshots[index].checkpoint;
        self.filesystem.release_checkpoint(checkpoint.id)?;
        self.snapshots[index].occupied = false;
        Ok(())
    }

    pub fn garbage_collect(
        &mut self,
        process: ProcessId,
        authority: Capability,
        work_budget: usize,
    ) -> Result<GcReport, DaemonError> {
        self.authorize_process(process, authority, FileRights::ADMIN)?;
        let before = self.filesystem.used_blocks();
        let report = self.filesystem.collect_garbage();
        let freed_blocks = before
            .saturating_sub(self.filesystem.used_blocks())
            .min(work_budget);
        Ok(GcReport {
            live_blocks: report.live_blocks,
            freed_blocks,
        })
    }

    pub fn diagnostics(&self) -> Result<SynFsDiagnostics, DaemonError> {
        Ok(self.filesystem.diagnostics()?)
    }

    /// Run one atomic group of filesystem mutations. The callback must finish
    /// successfully for the transaction to commit; every error rolls it back.
    pub fn transact(
        &mut self,
        operation: impl FnOnce(&mut SynFsTransaction<'_, MAX_BLOCKS>) -> Result<(), SynFsError>,
    ) -> Result<TransactionCommit, DaemonError> {
        let mut transaction = self.filesystem.transaction();
        operation(&mut transaction)?;
        Ok(transaction.commit()?)
    }

    pub fn mount(
        &mut self,
        process: ProcessId,
        authority: Capability,
        name: &str,
        read_only: bool,
    ) -> Result<(MountId, Capability), DaemonError> {
        self.authorize_process(process, authority, FileRights::ADMIN)?;
        let name = Name::from_str(name)?;
        if self
            .mounts
            .iter()
            .any(|mount| mount.occupied && mount.name == name)
        {
            return Err(DaemonError::File(SynFsError::AlreadyExists));
        }
        let (index, slot) = self
            .mounts
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| !slot.occupied)
            .ok_or(DaemonError::MountExhausted)?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.occupied = true;
        slot.id = MountId(self.next_mount_id);
        self.next_mount_id = self.next_mount_id.wrapping_add(1).max(2);
        slot.name = name;
        slot.read_only = read_only;
        Ok((slot.id, token(index, slot.generation)))
    }

    pub fn unmount(
        &mut self,
        process: ProcessId,
        authority: Capability,
        mount: Capability,
    ) -> Result<(), DaemonError> {
        self.authorize_process(process, authority, FileRights::ADMIN)?;
        let index = self.mount_index(mount)?;
        if index == 0 {
            return Err(DaemonError::AccessDenied);
        }
        self.mounts[index].occupied = false;
        Ok(())
    }

    pub fn dispatch(&mut self, request: Request, buffer: Option<&mut [u8]>) -> Response {
        record_profile_sample(ProfileSample::single(
            ProfileDomain::SynFs,
            request.operation.raw() as u64,
            0,
            0x4001,
        ));
        if buffer
            .as_ref()
            .is_some_and(|buffer| buffer.len() > MAX_IPC_BUFFER_BYTES)
        {
            return Response::error(
                DaemonError::BufferTooSmall {
                    required: MAX_IPC_BUFFER_BYTES,
                }
                .status(),
            );
        }
        if let Some(capability) = request.capability {
            if let Some(trace) = CapabilityTrace::new(
                CapabilityDomain::Filesystem,
                CapabilityTraceStage::KernelIpc,
                capability.raw(),
                request.operation.raw(),
            ) {
                trace.emit(Level::Trace)
            }
        }
        match self.execute(request, buffer) {
            Ok(response) => response,
            Err(error) => Response::error(error.status()),
        }
    }

    pub fn dispatch_envelope(&mut self, envelope: Envelope, buffer: Option<&mut [u8]>) -> Envelope {
        let correlation = envelope.correlation;
        let response = match Request::from_envelope(envelope) {
            Ok((request, descriptor)) => {
                match validate_buffer(request.operation, descriptor, buffer) {
                    Ok(buffer) => self.dispatch(request, buffer),
                    Err(error) => Response::error(error.status()),
                }
            }
            Err(error) => Response::error(DaemonError::Protocol(error).status()),
        };
        response.to_envelope(correlation)
    }

    fn execute(
        &mut self,
        request: Request,
        buffer: Option<&mut [u8]>,
    ) -> Result<Response, DaemonError> {
        match request.operation {
            Operation::Open => {
                let path = input_name(buffer)?;
                let info = self.open(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    path.as_str(),
                    request.flags,
                )?;
                Ok(Response::success()
                    .with_value(0, info.capability.raw())
                    .with_value(1, info.size))
            }
            Operation::Close => {
                self.close(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                )?;
                Ok(Response::success())
            }
            Operation::Read => {
                let output = output_buffer(buffer)?;
                let bytes = self.read(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    request.offset,
                    output,
                )?;
                Ok(Response::success().with_value(0, bytes as u64))
            }
            Operation::Write => {
                let input = input_buffer(buffer)?;
                let bytes = self.write(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    request.offset,
                    input,
                )?;
                Ok(Response::success().with_value(0, bytes as u64))
            }
            Operation::Map => {
                if request.flags.bits() & !Flags::WRITE.bits() != 0 {
                    return Err(DaemonError::InvalidRequest);
                }
                let mapping = self.map(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    request.offset,
                    request.length,
                    request.flags.contains(Flags::WRITE),
                )?;
                Ok(Response::success()
                    .with_value(0, mapping.capability.raw())
                    .with_value(1, mapping.offset)
                    .with_value(2, mapping.length)
                    .with_value(3, mapping.writable as u64))
            }
            Operation::Unmap => {
                if request.flags.bits() != 0 || request.offset != 0 || request.length != 0 {
                    return Err(DaemonError::InvalidRequest);
                }
                self.unmap(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                )?;
                Ok(Response::success())
            }
            Operation::Metadata => {
                let info = self.metadata(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                )?;
                Ok(Response::success()
                    .with_value(0, info.size)
                    .with_value(1, info.version as u64)
                    .with_value(2, info.checksum)
                    .with_value(3, info.created_at))
            }
            Operation::Delete => {
                let path = input_name(buffer)?;
                let deleted = self.delete_path(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    path.as_str(),
                )?;
                Ok(Response {
                    status: deleted.status,
                    values: [
                        deleted.file.version as u64,
                        deleted.file.file_type as u64,
                        deleted.file.link_count as u64,
                        deleted.shared_data_reachable as u64,
                    ],
                })
            }
            Operation::Rename => {
                let input = input_buffer(buffer)?;
                let separator = input
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or(DaemonError::InvalidRename)?;
                let old_path = Name::from_bytes(&input[..separator], false)?;
                let new_path = Name::from_bytes(&input[separator + 1..], false)?;
                let capability = request.capability.ok_or(DaemonError::InvalidCapability)?;
                let index = self.file_index(
                    request.process,
                    capability,
                    FileRights::DELETE.union(FileRights::WRITE),
                )?;
                if self.open_files[index].path != old_path {
                    return Err(DaemonError::InvalidRename);
                }
                self.rename(request.process, capability, new_path.as_str())?;
                Ok(Response::success())
            }
            Operation::List => {
                let output = output_buffer(buffer)?;
                let prefix_end = output
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(output.len());
                let prefix = if &output[..prefix_end] == b"/" {
                    Name::EMPTY
                } else {
                    Name::from_bytes(&output[..prefix_end], true)?
                };
                let bytes = self.list_current(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    prefix.as_str(),
                    request.offset as usize,
                    output,
                )?;
                Ok(Response::success()
                    .with_value(0, bytes.0 as u64)
                    .with_value(1, bytes.1 as u64))
            }
            Operation::SnapshotCreate => {
                let info = self.snapshot_create(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                )?;
                Ok(Response::success()
                    .with_value(0, info.capability.raw())
                    .with_value(1, info.generation))
            }
            Operation::SnapshotRelease => {
                self.snapshot_release(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                )?;
                Ok(Response::success())
            }
            Operation::SnapshotList => {
                let output = output_buffer(buffer)?;
                let prefix_end = output
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(output.len());
                let prefix = Name::from_bytes(&output[..prefix_end], true)?;
                let bytes = self.list_snapshot(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    prefix.as_str(),
                    request.offset as usize,
                    output,
                )?;
                Ok(Response::success()
                    .with_value(0, bytes.0 as u64)
                    .with_value(1, bytes.1 as u64))
            }
            Operation::Mount => {
                let name = input_name(buffer)?;
                let (id, capability) = self.mount(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    name.as_str(),
                    request.flags.contains(Flags::READ_ONLY),
                )?;
                Ok(Response::success()
                    .with_value(0, id.raw() as u64)
                    .with_value(1, capability.raw()))
            }
            Operation::Unmount => {
                self.unmount(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    Capability::from_raw(request.offset).ok_or(DaemonError::InvalidCapability)?,
                )?;
                Ok(Response::success())
            }
            Operation::MountList => {
                let output = output_buffer(buffer)?;
                self.authorize_process(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    FileRights::ADMIN,
                )?;
                let bytes = self.list_mounts(output)?;
                Ok(Response::success().with_value(0, bytes as u64))
            }
            Operation::GarbageCollect => {
                let report = self.garbage_collect(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    request.offset as usize,
                )?;
                Ok(Response::success()
                    .with_value(0, report.freed_blocks as u64)
                    .with_value(1, report.live_blocks as u64))
            }
            Operation::Mkdir => {
                let path = input_name(buffer)?;
                let info = self.create_directory(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    path.as_str(),
                    request.flags.contains(Flags::RECURSIVE),
                )?;
                Ok(Response::success()
                    .with_value(0, info.size)
                    .with_value(1, info.version as u64)
                    .with_value(2, info.created_at))
            }
            Operation::Rmdir => {
                let path = input_name(buffer)?;
                let removed = self.remove_directory(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    path.as_str(),
                )?;
                Ok(Response::success()
                    .with_value(0, removed.file.size)
                    .with_value(1, removed.file.version as u64)
                    .with_value(2, removed.removal_generation)
                    .with_value(3, removed.storage_reclamation_pending as u64))
            }
            Operation::Link => {
                let new_path = input_name(buffer)?;
                let info = self.link(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    new_path.as_str(),
                )?;
                Ok(Response::success()
                    .with_value(0, info.version as u64)
                    .with_value(1, info.size)
                    .with_value(2, info.link_count as u64))
            }
            Operation::Symlink => {
                let input = input_buffer(buffer)?;
                let separator = input
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or(DaemonError::InvalidPath)?;
                let target = core::str::from_utf8(&input[..separator])
                    .map_err(|_| DaemonError::InvalidPath)?;
                let link_path = Name::from_bytes(&input[separator + 1..], false)?;
                let info = self.symlink(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    target,
                    link_path.as_str(),
                )?;
                Ok(Response::success()
                    .with_value(0, info.size)
                    .with_value(1, info.version as u64))
            }
            Operation::ReadLink => {
                let buffer = output_buffer(buffer)?;
                let separator = buffer
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or(DaemonError::InvalidPath)?;
                let path = Name::from_bytes(&buffer[..separator], false)?;
                let output = &mut buffer[separator + 1..];
                let (_, bytes) = self.read_link(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    path.as_str(),
                    output,
                )?;
                Ok(Response::success().with_value(0, bytes as u64))
            }
            Operation::Chmod => {
                let info = self.chmod(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    request.offset as u16,
                )?;
                Ok(Response::success().with_value(0, info.mode as u64))
            }
            Operation::Links => {
                let output = output_buffer(buffer)?;
                let path_end = output
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(output.len());
                let path = Name::from_bytes(&output[..path_end], false)?;
                let (_, bytes) = self.list_links(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    path.as_str(),
                    output,
                )?;
                Ok(Response::success().with_value(0, bytes as u64))
            }
            Operation::Lock => {
                let capability = request.capability.ok_or(DaemonError::InvalidCapability)?;
                let mode = lock_mode(request.flags)?;
                let range = if request.flags.contains(Flags::LOCK_RECORD) {
                    LockRange::Record(request.offset)
                } else {
                    LockRange::WholeFile
                };
                let lock = self.lock(request.process, capability, range, mode)?;
                Ok(Response::success().with_value(0, lock.capability.raw()))
            }
            Operation::Unlock => {
                self.unlock(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                )?;
                Ok(Response::success())
            }
        }
    }

    fn write_range(&mut self, path: &str, offset: u64, contents: &[u8]) -> Result<(), DaemonError> {
        let current = self.filesystem.lookup(path);
        let current_size = match current {
            Ok(file) => usize::try_from(file.size).map_err(|_| DaemonError::ScratchTooSmall {
                required: usize::MAX,
            })?,
            Err(SynFsError::NotFound) if offset == 0 => 0,
            Err(error) => return Err(error.into()),
        };
        let offset = usize::try_from(offset).map_err(|_| DaemonError::ScratchTooSmall {
            required: usize::MAX,
        })?;
        if offset > current_size {
            return Err(DaemonError::InvalidRequest);
        }
        let new_size = current_size.max(offset.saturating_add(contents.len()));
        if new_size > SCRATCH_BYTES {
            return Err(DaemonError::ScratchTooSmall { required: new_size });
        }
        if current_size != 0 {
            self.filesystem
                .read(path, &mut self.scratch[..current_size])?;
        }
        if new_size > current_size {
            self.scratch[current_size..new_size].fill(0);
        }
        self.scratch[offset..offset + contents.len()].copy_from_slice(contents);
        self.filesystem.write(path, &self.scratch[..new_size])?;
        Ok(())
    }

    fn list_current(
        &mut self,
        process: ProcessId,
        authority: Capability,
        prefix: &str,
        continuation: usize,
        output: &mut [u8],
    ) -> Result<(usize, usize), DaemonError> {
        self.authorize_process(process, authority, FileRights::READ)?;
        let directory = if prefix.is_empty() || prefix == "/" {
            None
        } else {
            Some(self.filesystem.lookup_following(prefix)?)
        };
        if let Some(directory) = directory {
            if directory.file_type != FileType::Directory {
                return Err(DaemonError::File(SynFsError::NotDirectory))
            }
            self.check_mode_access(process, directory.mode, FileRights::READ)?;
        } else {
            self.check_mode_access(process, 0o777, FileRights::READ)?;
        }
        let checkpoint = self.filesystem.create_checkpoint()?;
        let listing = {
            match self
                .filesystem
                .checkpoint_snapshot(checkpoint.id, rms_capability())
            {
                Ok(snapshot) => {
                    Self::write_snapshot_listing(&snapshot, prefix, continuation, output)
                }
                Err(error) => Err(DaemonError::File(error)),
            }
        };
        let released = self.filesystem.release_checkpoint(checkpoint.id);
        match (listing, released) {
            (Ok(bytes), Ok(())) => Ok(bytes),
            (Err(error), _) => Err(error),
            (_, Err(error)) => Err(error.into()),
        }
    }

    fn list_snapshot(
        &self,
        process: ProcessId,
        capability: Capability,
        prefix: &str,
        continuation: usize,
        output: &mut [u8],
    ) -> Result<(usize, usize), DaemonError> {
        let index = self.snapshot_index(process, capability)?;
        let snapshot = self
            .filesystem
            .checkpoint_snapshot(self.snapshots[index].checkpoint.id, rms_capability())?;
        Ok(Self::write_snapshot_listing(
            &snapshot,
            prefix,
            continuation,
            output,
        )?)
    }

    fn list_mounts(&self, output: &mut [u8]) -> Result<usize, DaemonError> {
        let mut written = 0;
        for mount in self.mounts.iter().filter(|mount| mount.occupied) {
            written = Self::append_mount_name(written, mount.name.as_bytes(), output)?;
        }
        for mount in self.namespace.mounts() {
            written = Self::append_mount_name(written, mount.path.as_bytes(), output)?;
        }
        Ok(written)
    }

    fn append_mount_name(
        written: usize,
        name: &[u8],
        output: &mut [u8],
    ) -> Result<usize, DaemonError> {
        let required = name.len() + 1;
        if written + required > output.len() {
            return Err(DaemonError::BufferTooSmall {
                required: written + required,
            });
        }
        output[written..written + name.len()].copy_from_slice(name);
        output[written + name.len()] = b'\n';
        Ok(written + required)
    }

    fn write_snapshot_listing<const BLOCKS: usize>(
        snapshot: &ghostos_ghostfs::ReadOnlySnapshot<'_, BLOCKS>,
        prefix: &str,
        continuation: usize,
        output: &mut [u8],
    ) -> Result<(usize, usize), DaemonError> {
        let root = prefix.is_empty() || prefix == "/";
        let path = if root { "/" } else { prefix };
        let mut entries = [DirectoryEntry::EMPTY; MAX_DIRECTORY_ENTRIES];
        let mut wildcard = false;
        let mut wildcard_next = None;
        let count = if root {
            snapshot.list_directory(path, &mut entries)?
        } else {
            let versioned = VersionedPath::parse(path).map_err(|_| DaemonError::InvalidPath)?;
            let pattern = Pattern::parse(versioned.file.as_str()).map_err(pattern_error)?;
            wildcard = pattern.has_magic();
            if wildcard {
                let mut matches = [None; MAX_DIRECTORY_ENTRIES];
                let (count, next) = match snapshot.expand_paths_page(path, continuation, &mut matches) {
                    Ok(page) => page,
                    Err(SynFsError::BufferTooSmall { .. }) => {
                        return Err(DaemonError::PartialMatch)
                    }
                    Err(error) => return Err(DaemonError::File(error)),
                };
                wildcard_next = next;
                for (index, matched) in matches[..count].iter().flatten().enumerate() {
                    let selected = versioned_name(matched.as_str(), versioned.version)?;
                    let file = snapshot.lookup(selected.as_str())?;
                    entries[index] = DirectoryEntry {
                        name: *matched,
                        file_type: file.file_type,
                        size: file.size,
                        version: file.version,
                        link_count: file.link_count,
                        mode: file.mode,
                    };
                }
                if count == 0 {
                    return Err(DaemonError::NotFound)
                }
                count
            } else {
                snapshot.list_directory(path, &mut entries)?
            }
        };
        let mut written = 0;
        let mut next = 0;
        let mut stopped_for_buffer = false;
        let entry_start = if wildcard { 0 } else { continuation };
        for (index, entry) in entries.iter().take(count).enumerate().skip(entry_start) {
            let name = entry.name.as_bytes();
            let required = 22usize
                .checked_add(name.len())
                .ok_or(DaemonError::BufferTooSmall { required: usize::MAX })?;
            if written + required > output.len() {
                if written == 0 {
                    return Err(DaemonError::BufferTooSmall { required })
                }
                next = if wildcard {
                    continuation.saturating_add(index)
                } else {
                    index
                };
                stopped_for_buffer = true;
                break
            }
            let name_length = u16::try_from(name.len()).map_err(|_| DaemonError::InvalidPath)?;
            output[written..written + 2].copy_from_slice(&name_length.to_le_bytes());
            output[written + 2] = entry.file_type as u8;
            output[written + 3] = 0;
            output[written + 4..written + 12].copy_from_slice(&entry.size.to_le_bytes());
            output[written + 12..written + 16].copy_from_slice(&entry.version.to_le_bytes());
            output[written + 16..written + 20]
                .copy_from_slice(&entry.link_count.to_le_bytes());
            output[written + 20..written + 22].copy_from_slice(&entry.mode.to_le_bytes());
            output[written + 22..written + required].copy_from_slice(name);
            written += required;
            next = index + 1;
        }
        if wildcard && !stopped_for_buffer {
            next = wildcard_next.unwrap_or(0);
        }
        if stopped_for_buffer || wildcard_next.is_some() || next < count {
            Ok((written, next))
        } else {
            Ok((written, 0))
        }
    }

    fn ensure_writable(
        &self,
        process: ProcessId,
        authority: Capability,
    ) -> Result<(), DaemonError> {
        self.authorize_process(process, authority, FileRights::WRITE)
            .map(|_| ())
    }

    fn check_mode_access(
        &self,
        process: ProcessId,
        mode: u16,
        required: FileRights,
    ) -> Result<(), DaemonError> {
        let process_index = self.process_index(process, None)?;
        if self.processes[process_index]
            .rights
            .contains_file(FileRights::ADMIN)
        {
            return Ok(())
        }
        let class = if process.raw() == 1 {
            (mode >> 6) & 0o7
        } else {
            mode & 0o7
        };
        if required.contains(FileRights::READ) && class & 0o4 == 0 {
            return Err(DaemonError::AccessDenied)
        }
        if (required.contains(FileRights::WRITE) || required.contains(FileRights::DELETE))
            && class & 0o2 == 0
        {
            return Err(DaemonError::AccessDenied)
        }
        if required.contains(FileRights::TRAVERSE) && class & 0o1 == 0 {
            return Err(DaemonError::AccessDenied)
        }
        Ok(())
    }

    fn check_parent_access(
        &self,
        process: ProcessId,
        path: &str,
        required: FileRights,
    ) -> Result<(), DaemonError> {
        let parent = path
            .rsplit_once('/')
            .map_or("/", |(parent, _)| if parent.is_empty() { "/" } else { parent });
        if parent == "/" {
            return self.check_mode_access(process, 0o777, required.union(FileRights::TRAVERSE))
        }
        let mut end = 1;
        while end <= parent.len() {
            if end == parent.len() || parent.as_bytes()[end] == b'/' {
                let metadata = self
                    .filesystem
                    .lookup_following(&parent[..end])?;
                if metadata.file_type != FileType::Directory {
                    return Err(DaemonError::File(SynFsError::NotDirectory))
                }
                let access = if end == parent.len() {
                    required.union(FileRights::TRAVERSE)
                } else {
                    FileRights::TRAVERSE
                };
                self.check_mode_access(process, metadata.mode, access)?;
            }
            end += 1;
        }
        Ok(())
    }

    fn require_process_rights(
        &self,
        process: ProcessId,
        required: FileRights,
    ) -> Result<(), DaemonError> {
        let index = self.process_index(process, None)?;
        if !self.processes[index].rights.contains_file(required) {
            return Err(DaemonError::AccessDenied);
        }
        Ok(())
    }

    fn authorize_process(
        &self,
        process: ProcessId,
        capability: Capability,
        required: FileRights,
    ) -> Result<bool, DaemonError> {
        let index = self.process_index(process, Some(capability))?;
        if !self.processes[index].rights.contains_file(required) {
            return Err(DaemonError::AccessDenied);
        }
        if let Some(trace) = CapabilityTrace::new(
            CapabilityDomain::Filesystem,
            CapabilityTraceStage::DaemonAuthorized,
            capability.raw(),
            required.bits(),
        ) {
            trace.emit(Level::Trace)
        }
        Ok(true)
    }

    fn process_index(
        &self,
        process: ProcessId,
        capability: Option<Capability>,
    ) -> Result<usize, DaemonError> {
        let index = self
            .processes
            .iter()
            .position(|slot| slot.occupied && slot.process == Some(process))
            .ok_or(DaemonError::ProcessNotRegistered)?;
        if let Some(capability) = capability {
            if decode_slot(capability, self.processes[index].generation) != Some(index) {
                return Err(DaemonError::InvalidCapability);
            }
        }
        Ok(index)
    }

    fn file_index(
        &self,
        process: ProcessId,
        capability: Capability,
        required: FileRights,
    ) -> Result<usize, DaemonError> {
        let index = decode_slot(capability, 0).ok_or(DaemonError::InvalidCapability)?;
        let slot = self
            .open_files
            .get(index)
            .ok_or(DaemonError::InvalidCapability)?;
        if !slot.occupied
            || slot.generation != capability_generation(capability)
            || slot.owner != process
            || !slot.rights.contains(required)
        {
            return Err(DaemonError::AccessDenied);
        }
        Ok(index)
    }

    fn lock_index(
        &self,
        process: ProcessId,
        capability: Capability,
    ) -> Result<usize, DaemonError> {
        let index = decode_slot(capability, 0).ok_or(DaemonError::InvalidCapability)?;
        let slot = self
            .locks
            .get(index)
            .ok_or(DaemonError::InvalidCapability)?;
        if !slot.occupied
            || slot.generation != capability_generation(capability)
            || slot.owner != process
        {
            return Err(DaemonError::AccessDenied);
        }
        Ok(index)
    }

    fn mapping_index(
        &self,
        process: ProcessId,
        capability: Capability,
    ) -> Result<usize, DaemonError> {
        let index = decode_mapping_slot(capability).ok_or(DaemonError::InvalidCapability)?;
        let slot = self
            .mappings
            .get(index)
            .ok_or(DaemonError::InvalidCapability)?;
        if !slot.occupied
            || slot.generation != capability_generation(capability)
            || slot.owner != process
        {
            return Err(DaemonError::AccessDenied);
        }
        Ok(index)
    }

    fn check_io_lock(
        &self,
        process: ProcessId,
        path: Name,
        range: LockRange,
        requested: LockMode,
    ) -> Result<(), DaemonError> {
        if self.locks.iter().any(|lock| {
            lock.occupied
                && lock.owner != process
                && lock.path == path
                && ranges_overlap(lock.range, range)
                && (lock.mode == LockMode::Exclusive || requested == LockMode::Exclusive)
        }) {
            Err(DaemonError::LockBusy)
        } else {
            Ok(())
        }
    }

    fn snapshot_index(
        &self,
        process: ProcessId,
        capability: Capability,
    ) -> Result<usize, DaemonError> {
        let index = decode_slot(capability, 0).ok_or(DaemonError::InvalidCapability)?;
        let slot = self
            .snapshots
            .get(index)
            .ok_or(DaemonError::InvalidCapability)?;
        if !slot.occupied
            || slot.generation != capability_generation(capability)
            || slot.owner != process
        {
            return Err(DaemonError::AccessDenied);
        }
        Ok(index)
    }

    fn mount_index(&self, capability: Capability) -> Result<usize, DaemonError> {
        let index = decode_slot(capability, 0).ok_or(DaemonError::InvalidCapability)?;
        let slot = self
            .mounts
            .get(index)
            .ok_or(DaemonError::InvalidCapability)?;
        if !slot.occupied || slot.generation != capability_generation(capability) {
            return Err(DaemonError::InvalidCapability);
        }
        Ok(index)
    }

    fn mount_is_read_only(&self, _path: &str) -> bool {
        self.mounts.iter().any(|mount| {
            mount.occupied
                && mount.read_only
                && (_path == mount.name.as_str()
                    || _path
                        .strip_prefix(mount.name.as_str())
                        .is_some_and(|rest| rest.starts_with(':') || rest.starts_with('/')))
        })
    }

    fn is_mount_root(&self, path: &str) -> bool {
        self.namespace
            .resolve(path)
            .is_ok_and(|mount| mount.path.as_str() == path)
    }
}

impl<
        const MAX_BLOCKS: usize,
        const MAX_PROCESSES: usize,
        const MAX_OPEN_FILES: usize,
        const MAX_SNAPSHOTS: usize,
        const MAX_MOUNTS: usize,
        const SCRATCH_BYTES: usize,
    > Default
    for Daemon<MAX_BLOCKS, MAX_PROCESSES, MAX_OPEN_FILES, MAX_SNAPSHOTS, MAX_MOUNTS, SCRATCH_BYTES>
{
    fn default() -> Self {
        Self::new(SynFs::new()).unwrap_or_else(|_| Self {
            filesystem: SynFs::new(),
            processes: [ProcessSlot::EMPTY; MAX_PROCESSES],
            open_files: [OpenFileSlot::EMPTY; MAX_OPEN_FILES],
            mappings: [MappingSlot::EMPTY; MAX_OPEN_FILES],
            locks: [LockSlot::EMPTY; MAX_OPEN_FILES],
            snapshots: [SnapshotSlot::EMPTY; MAX_SNAPSHOTS],
            mounts: [MountSlot::EMPTY; MAX_MOUNTS],
            next_mount_id: 1,
            namespace: Namespace::new(),
            root_activation: RootActivation::default(),
            scratch: [0; SCRATCH_BYTES],
        })
    }
}

fn token(index: usize, generation: u32) -> Capability {
    Capability::from_valid_raw(((generation as u64) << 32) | (index as u64 + 1))
}

fn mapping_token(index: usize, generation: u32) -> Capability {
    Capability::from_valid_raw(
        ((generation as u64) << 32) | (u64::from(MAPPING_CAPABILITY_BIT) | index as u64 + 1),
    )
}

fn capability_generation(capability: Capability) -> u32 {
    (capability.raw() >> 32) as u32
}

fn decode_slot(capability: Capability, expected_generation: u32) -> Option<usize> {
    let raw_slot = capability.raw() as u32;
    if raw_slot == 0 {
        return None;
    }
    let generation = capability_generation(capability);
    if expected_generation != 0 && generation != expected_generation {
        return None;
    }
    Some(raw_slot.saturating_sub(1) as usize)
}

fn decode_mapping_slot(capability: Capability) -> Option<usize> {
    let raw_slot = capability.raw() as u32;
    if raw_slot & MAPPING_CAPABILITY_BIT == 0 {
        return None;
    }
    Some((raw_slot & !MAPPING_CAPABILITY_BIT).checked_sub(1)? as usize)
}

fn lock_mode(flags: Flags) -> Result<LockMode, DaemonError> {
    match (
        flags.contains(Flags::LOCK_SHARED),
        flags.contains(Flags::LOCK_EXCLUSIVE),
    ) {
        (true, false) => Ok(LockMode::Shared),
        (false, true) => Ok(LockMode::Exclusive),
        _ => Err(DaemonError::InvalidLock),
    }
}

fn ranges_overlap(left: LockRange, right: LockRange) -> bool {
    matches!(left, LockRange::WholeFile)
        || matches!(right, LockRange::WholeFile)
        || left == right
}

fn input_buffer(buffer: Option<&mut [u8]>) -> Result<&mut [u8], DaemonError> {
    buffer.ok_or(DaemonError::Protocol(ProtocolError::MissingBuffer))
}

fn output_buffer(buffer: Option<&mut [u8]>) -> Result<&mut [u8], DaemonError> {
    input_buffer(buffer)
}

fn input_name(buffer: Option<&mut [u8]>) -> Result<Name, DaemonError> {
    let buffer = input_buffer(buffer)?;
    Name::from_bytes(buffer, false)
}

fn validate_buffer(
    operation: Operation,
    descriptor: Option<SharedBuffer>,
    buffer: Option<&mut [u8]>,
) -> Result<Option<&mut [u8]>, DaemonError> {
    let needs_buffer = matches!(
        operation,
        Operation::Open
            | Operation::Read
            | Operation::Write
            | Operation::Delete
            | Operation::Rename
            | Operation::List
            | Operation::SnapshotList
            | Operation::Mount
            | Operation::MountList
            | Operation::Mkdir
            | Operation::Rmdir
            | Operation::Link
            | Operation::Symlink
            | Operation::ReadLink
            | Operation::Links
    );
    if !needs_buffer {
        if descriptor.is_some() || buffer.is_some() {
            return Err(DaemonError::Protocol(ProtocolError::InvalidBuffer));
        }
        return Ok(None);
    }
    let descriptor = descriptor.ok_or(DaemonError::Protocol(ProtocolError::MissingBuffer))?;
    let buffer = buffer.ok_or(DaemonError::Protocol(ProtocolError::MissingBuffer))?;
    if descriptor.length as usize != buffer.len() || buffer.len() > MAX_IPC_BUFFER_BYTES {
        return Err(DaemonError::Protocol(ProtocolError::InvalidBuffer));
    }
    if descriptor.region.raw() == 0 {
        return Err(DaemonError::Protocol(ProtocolError::InvalidBuffer));
    }
        let expected_writable = matches!(
            operation,
        Operation::Read
            | Operation::List
            | Operation::Links
            | Operation::ReadLink
            | Operation::SnapshotList
            | Operation::MountList
        );
    if descriptor.writable != expected_writable {
        return Err(DaemonError::Protocol(ProtocolError::InvalidBuffer));
    }
    Ok(Some(buffer))
}

fn rms_capability() -> ghostos_ghostfs::RmsMapHandle {
    ghostos_ghostfs::RmsMapHandle::from_valid_capability(INTERNAL_MAPPING_CAPABILITY)
}

fn mount_path(path: &str) -> &str {
    path.rsplit_once(';').map_or(path, |(path, _)| path)
}

fn versioned_name(path: &str, selector: VersionSelector) -> Result<Name, DaemonError> {
    let VersionSelector::Exact(version) = selector else {
        return Name::from_str(path)
    };
    let required = path
        .len()
        .checked_add(1)
        .and_then(|length| length.checked_add(digits(version)))
        .ok_or(DaemonError::InvalidPath)?;
    if required > MAX_NAME_BYTES {
        return Err(DaemonError::InvalidPath)
    }
    let mut bytes = [0; MAX_NAME_BYTES];
    bytes[..path.len()].copy_from_slice(path.as_bytes());
    bytes[path.len()] = b';';
    let end = path.len() + 1;
    write_decimal(&mut bytes[end..required], version);
    Name::from_bytes(&bytes[..required], false)
}

fn insert_link_entry(
    entries: &mut [LinkEntry],
    count: &mut usize,
    entry: LinkEntry,
) -> Result<(), DaemonError> {
    if entries[..*count]
        .iter()
        .any(|current| current.path == entry.path && current.version == entry.version)
    {
        return Ok(())
    }
    if *count == entries.len() {
        return Err(DaemonError::BufferTooSmall {
            required: (*count).saturating_add(1),
        })
    }
    let insert_at = entries[..*count]
        .iter()
        .position(|current| {
            current.path > entry.path
                || current.path == entry.path && current.version > entry.version
        })
        .unwrap_or(*count);
    for index in (insert_at..*count).rev() {
        entries[index + 1] = entries[index];
    }
    entries[insert_at] = entry;
    *count += 1;
    Ok(())
}

fn same_volume(
    left: Result<crate::namespace::MountInfo, NamespaceError>,
    right: Result<crate::namespace::MountInfo, NamespaceError>,
) -> bool {
    let (Ok(left), Ok(right)) = (left, right) else {
        return false
    };
    match (left.source, right.source) {
        (
            MountSource::SynFs { volume: left, .. },
            MountSource::SynFs { volume: right, .. },
        ) => left == right,
        (
            MountSource::Host {
                filesystem: left_filesystem,
                partition: left_partition,
            },
            MountSource::Host {
                filesystem: right_filesystem,
                partition: right_partition,
            },
        ) => left_filesystem == right_filesystem && left_partition == right_partition,
        _ => false,
    }
}

fn digits(mut value: u32) -> usize {
    let mut count = 1;
    while value >= 10 {
        value /= 10;
        count += 1;
    }
    count
}

fn write_decimal(destination: &mut [u8], mut value: u32) -> usize {
    let count = digits(value);
    let mut index = count;
    while index != 0 {
        index -= 1;
        destination[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    count
}
