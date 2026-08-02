use core::fmt;

use host_filesystems::{FileSystemKind, Partition};
use synos_ipc::{Envelope, SharedBuffer};
use synos_status::{facility, IntoStatus, Severity, Status};
use synos_synfs::{
    CheckpointInfo, DirectoryEntry, Error as SynFsError, FileType, LinkEntry, SynFs,
    SynFsDiagnostics, SynFsTransaction, TransactionCommit,
};

use crate::namespace::{
    HostMountAuthority, MountCapability, MountSource, Namespace, NamespaceError, RootActivation,
    RootFilesystem,
};

use crate::protocol::{
    Capability, Flags, Operation, ProcessId, ProtocolError, Request, Response, MAX_IPC_BUFFER_BYTES,
};

pub const DEFAULT_MAX_PROCESSES: usize = 64;
pub const DEFAULT_MAX_OPEN_FILES: usize = 256;
pub const DEFAULT_MAX_SNAPSHOTS: usize = 16;
pub const DEFAULT_MAX_MOUNTS: usize = 16;
pub const DEFAULT_SCRATCH_BYTES: usize = MAX_IPC_BUFFER_BYTES;
const MAX_DIRECTORY_ENTRIES: usize = 256;
const MAX_NAME_BYTES: usize = synos_synfs::MAX_PATH_BYTES;
const ROOT_MOUNT_NAME: &str = "SYS$ROOT";
const INTERNAL_MAPPING_CAPABILITY: u64 = 1 << 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct FileRights(u16);

impl FileRights {
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
    pub file: synos_synfs::FileName,
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
    BufferTooSmall { required: usize },
    InvalidPath,
    InvalidRename,
    ScratchTooSmall { required: usize },
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
            Self::Protocol(_) | Self::InvalidRequest | Self::InvalidPath | Self::InvalidRename => {
                Status::INVALID_ARGUMENT
            }
            Self::ProcessNotRegistered
            | Self::AccessDenied
            | Self::InvalidCapability
            | Self::ReadOnly => Status::ACCESS_DENIED,
            Self::CrossVolume => Status::INVALID_ARGUMENT,
            Self::CapabilityExhausted
            | Self::HandleExhausted
            | Self::SnapshotExhausted
            | Self::MountExhausted
            | Self::ScratchTooSmall { .. } => Status::NO_SPACE,
            Self::NotFound => Status::NOT_FOUND,
            Self::BufferTooSmall { .. } => Status::new(Severity::Error, facility::FILESYSTEM, 1, 0)
                .expect("valid filesystem status"),
            Self::Namespace(error) => match error {
                NamespaceError::InvalidPath | NamespaceError::InvalidPartition => {
                    Status::INVALID_ARGUMENT
                }
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
            Self::BufferTooSmall { .. } => "shared buffer is too small",
            Self::InvalidPath => "invalid filesystem path",
            Self::InvalidRename => "invalid rename payload",
            Self::ScratchTooSmall { .. } => "daemon scratch space is too small",
            Self::Namespace(_) => "invalid filesystem namespace operation",
            Self::File(_) => "SynFS operation failed",
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
        core::str::from_utf8(self.as_bytes()).expect("Name invariant")
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
        owner: ProcessId::new(1).expect("non-zero placeholder process"),
        path: Name::EMPTY,
        rights: FileRights(0),
        read_only_mount: false,
        append: false,
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
        owner: ProcessId::new(1).expect("non-zero placeholder process"),
        checkpoint: CheckpointInfo {
            id: synos_synfs::CheckpointId::from_raw(1).expect("non-zero checkpoint id"),
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

/// Ring 3 owner of a SynFS volume.
///
/// The daemon owns the mutable SynFS root and all externally visible handles.
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
        Ok(token(index, slot.generation))
    }

    pub fn unregister_process(&mut self, process: ProcessId) -> Result<(), DaemonError> {
        let index = self.process_index(process, None)?;
        for file in &mut self.open_files {
            if file.occupied && file.owner == process {
                file.occupied = false
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
            return self.create_file_with_name(process, authority, name, read_only_mount, true);
        }
        let exists = self.filesystem.lookup(path).is_ok();
        if !exists && !flags.contains(Flags::CREATE) {
            return Err(DaemonError::NotFound);
        }
        if !exists {
            self.ensure_writable(process, authority)?;
            self.filesystem.write(path, &[])?;
        } else if flags.contains(Flags::TRUNCATE) {
            self.ensure_writable(process, authority)?;
            self.filesystem.write(path, &[])?;
        }
        let metadata = self.filesystem.lookup(path)?;
        if flags.contains(Flags::CREATE) && metadata.file_type != FileType::Regular {
            return Err(DaemonError::File(SynFsError::NotDirectory));
        }
        self.open_file_handle(
            process,
            name,
            requested,
            read_only_mount,
            flags.contains(Flags::APPEND),
            metadata,
        )
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
        self.create_file_with_name(process, authority, name, read_only_mount, false)
    }

    fn create_file_with_name(
        &mut self,
        process: ProcessId,
        authority: Capability,
        name: Name,
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
        self.filesystem.write(name.as_str(), &[])?;
        let metadata = self.filesystem.lookup(name.as_str())?;
        self.open_file_handle(
            process,
            name,
            FileRights::READ.union(FileRights::WRITE),
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
        metadata: synos_synfs::FileVersion,
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
        Ok(self
            .filesystem
            .read_at(path.as_str(), offset, destination)?
            .bytes_read)
    }

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
        let offset = if self.open_files[index].append {
            self.filesystem.lookup(path.as_str())?.size
        } else {
            offset
        };
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
        Ok(FileInfo {
            capability,
            file: metadata.file,
            version: metadata.version,
            size: metadata.size,
            checksum: metadata.checksum,
            created_at: metadata.created_at,
            rights: slot.rights,
            file_type: metadata.file_type,
            link_count: metadata.link_count,
            mode: metadata.mode,
        })
    }

    pub fn delete(
        &mut self,
        process: ProcessId,
        capability: Capability,
    ) -> Result<(), DaemonError> {
        let index = self.file_index(process, capability, FileRights::DELETE)?;
        if self.open_files[index].read_only_mount {
            return Err(DaemonError::ReadOnly);
        }
        let path = self.open_files[index].path;
        self.filesystem.delete(path.as_str())?;
        Ok(())
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
    ) -> Result<(), DaemonError> {
        self.authorize_process(process, authority, FileRights::DELETE)?;
        let path = Name::from_str(path)?;
        if self.mount_is_read_only(path.as_str()) {
            return Err(DaemonError::ReadOnly);
        }
        self.filesystem.remove_directory(path.as_str())?;
        Ok(())
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
        let new_path = Name::from_str(new_path)?;
        if self.path_is_read_only(new_path.as_str())? {
            return Err(DaemonError::ReadOnly);
        }
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

    pub fn list_links(
        &self,
        process: ProcessId,
        authority: Capability,
        path: &str,
        output: &mut [u8],
    ) -> Result<(FileInfo, usize), DaemonError> {
        self.authorize_process(process, authority, FileRights::READ)?;
        let path = Name::from_str(path)?;
        let metadata = self.filesystem.lookup(path.as_str())?;
        let mut entries = [LinkEntry::EMPTY; MAX_DIRECTORY_ENTRIES];
        let count = self.filesystem.list_links(path.as_str(), &mut entries)?;
        let mut written = 0;
        for entry in entries.iter().take(count) {
            let suffix = if entry.version == 0 {
                0
            } else {
                1 + digits(entry.version)
            };
            let required = entry.path.as_bytes().len() + suffix + 1;
            if written + required > output.len() {
                return Err(DaemonError::BufferTooSmall {
                    required: written + required,
                });
            }
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
        if self.filesystem.lookup(new_path.as_str()).is_ok() {
            return Err(DaemonError::File(SynFsError::AlreadyExists));
        }
        let mut transaction = self.filesystem.transaction();
        transaction.rename(old_path.as_str(), new_path.as_str())?;
        transaction.commit()?;
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
                self.delete(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                )?;
                Ok(Response::success())
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
                let prefix = Name::from_bytes(&output[..prefix_end], true)?;
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
                self.remove_directory(
                    request.process,
                    request.capability.ok_or(DaemonError::InvalidCapability)?,
                    path.as_str(),
                )?;
                Ok(Response::success())
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
        snapshot: &synos_synfs::ReadOnlySnapshot<'_, BLOCKS>,
        prefix: &str,
        continuation: usize,
        output: &mut [u8],
    ) -> Result<(usize, usize), DaemonError> {
        let path = if prefix.is_empty() { "/" } else { prefix };
        let mut entries = [DirectoryEntry::EMPTY; MAX_DIRECTORY_ENTRIES];
        let count = snapshot.list_directory(path, &mut entries)?;
        let mut written = 0;
        let mut next = 0;
        let mut stopped_for_buffer = false;
        for (index, entry) in entries.iter().take(count).enumerate().skip(continuation) {
            let name = entry.name.as_bytes();
            let required = 22 + name.len();
            if written + required > output.len() {
                if written == 0 {
                    return Err(DaemonError::BufferTooSmall { required })
                }
                next = index;
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
        if stopped_for_buffer || next < count {
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
        Self::new(SynFs::new()).expect("daemon has a root mount")
    }
}

fn token(index: usize, generation: u32) -> Capability {
    Capability::from_raw(((generation as u64) << 32) | (index as u64 + 1))
        .expect("daemon token has a generation")
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
            | Operation::Rename
            | Operation::List
            | Operation::SnapshotList
            | Operation::Mount
            | Operation::MountList
            | Operation::Mkdir
            | Operation::Rmdir
            | Operation::Link
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
            | Operation::SnapshotList
            | Operation::MountList
        );
    if descriptor.writable != expected_writable {
        return Err(DaemonError::Protocol(ProtocolError::InvalidBuffer));
    }
    Ok(Some(buffer))
}

fn rms_capability() -> synos_synfs::RmsMapHandle {
    synos_synfs::RmsMapHandle::from_capability(INTERNAL_MAPPING_CAPABILITY)
        .expect("internal mapping capability has a generation")
}

fn mount_path(path: &str) -> &str {
    path.rsplit_once(';').map_or(path, |(path, _)| path)
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
