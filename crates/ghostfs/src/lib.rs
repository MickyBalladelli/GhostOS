#![no_std]
#![forbid(unsafe_code)]

use core::cmp::Ordering;
use core::fmt;
use ghostos_durability::{CrashBoundary, CrashDomain, InterruptionInjector, NoInterruption};
use ghostos_status::{IntoStatus, Severity, Status, facility};

pub use ghostos_path_pattern::{MAX_PATTERN_BYTES, Pattern, PatternError};

mod block;
mod capacity;
mod migration;
mod pool;
mod rms;
mod scrub;
mod service_manifest;
mod system_disk;
mod volume;

pub use block::{
    BlockCompletion, BlockDevice, BlockIoError, BlockIoQueue, BlockIoResult, BlockOperation,
    BlockRequest, BlockRequestToken, BlockStore, DEFAULT_BLOCK_IO_QUEUE, MAX_BLOCK_IO_BYTES,
    StoragePoolIo,
};
pub use capacity::{
    CapacityForecast, CapacityObservation, CapacityResource, FragmentationReport,
    SynFsPathDiagnostics, DEFAULT_ALLOCATION_RESERVE_BYTES, DEFAULT_FORECAST_HORIZON_US,
};
pub use migration::{
    BackgroundIoLimit, FormatMigration, FormatMigrationPhase, FormatMigrationProgress,
    MAX_MIGRATION_BLOCKS_PER_STEP,
};
pub use pool::{
    BlockPlacement, DeviceHealth, MAX_POOL_MEMBERS, MAX_POOL_NAME_BYTES, PoolHealth, PoolLayout,
    PoolName, StorageClass, StorageDevice, StorageDeviceId, StoragePool, StoragePoolAdmin,
    StoragePoolError, StoragePoolId,
};
pub use rms::{
    IndexDefinition, MappedRecordFile, MappedRecordInfo, RecordDescriptor, RecordFileInfo,
    RecordFormat, RecordImageBuilder, RecordIter, RecordOrganization, RecordRead, RecordSelector,
    RmsError, RmsMapHandle,
};
pub use scrub::{
    RepairAuthorization, RepairEvidence, RepairPreview, RepairReceipt, ScrubFinding,
    ScrubIssue, ScrubPlan, ScrubReport, ScrubScope,
};
pub use service_manifest::{
    ServiceManifest, ServiceManifestEntry, ServiceManifestError, SERVICE_MANIFEST_PATH,
    SERVICE_PACKAGE_CAPACITY, SERVICE_PACKAGE_PATH_BYTES,
};
pub use system_disk::{
    MountedSystemVolume, SystemDiskError, SystemDiskManifest, SystemDiskVolume,
    SYSTEM_DISK_MANIFEST_BYTES, SYSTEM_VOLUME_BLOCKS,
};
pub use volume::{VOLUME_FORMAT_VERSION, VolumeCommit, VolumeGeometry};

pub const BLOCK_SIZE: usize = 4096;
pub const DATA_BYTES: usize = BLOCK_SIZE - 16;
pub const MAX_PATH_BYTES: usize = 192;
pub const MAX_KEYS: usize = 7;
pub const MAX_RETENTION_RULES: usize = 16;
pub const MAX_CHECKPOINTS: usize = 16;
pub const MAX_WILDCARD_MATCHES: usize = 256;
pub const MAX_SYMLINK_DEPTH: usize = 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyExists,
    BufferTooSmall { required: usize },
    Corrupt,
    DirectoryNotEmpty,
    InvalidPath,
    SymlinkLoop,
    NotSymlink,
    InvalidPattern,
    InvalidVersion,
    NotFound,
    NotDirectory,
    OutOfSpace,
    QuotaExceeded,
    CheckpointNotFound,
    TransactionAborted,
    TooManyCheckpoints,
    TooManyRetentionRules,
    VersionOverflow,
    Io,
    Interrupted,
    RepairUnauthorized,
    StaleRepairPlan,
    DowngradeRefused,
    MigrationAlreadyCurrent,
    MigrationInProgress,
    MigrationIncomplete,
    MigrationConflict,
    UnsupportedMigration,
    InvalidMigrationLimit,
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::NotFound | Self::CheckpointNotFound => Status::NOT_FOUND,
            Self::OutOfSpace | Self::TooManyCheckpoints | Self::TooManyRetentionRules => {
                Status::NO_SPACE
            }
            Self::Corrupt => Status::CORRUPT,
            Self::InvalidPath | Self::InvalidVersion | Self::SymlinkLoop => Status::INVALID_PATH,
            Self::NotSymlink => Status::INVALID_ARGUMENT,
            Self::InvalidPattern => Status::INVALID_PATTERN,
            Self::NotDirectory => Status::NOT_DIRECTORY,
            Self::DirectoryNotEmpty => Status::DIRECTORY_NOT_EMPTY,
            Self::TransactionAborted => Status::BUSY,
            Self::BufferTooSmall { .. } => Status::new(Severity::Error, facility::FILESYSTEM, 1, 0)
                .unwrap_or(Status::INVALID_ARGUMENT),
            Self::AlreadyExists => Status::new(Severity::Error, facility::FILESYSTEM, 2, 0)
                .unwrap_or(Status::INVALID_ARGUMENT),
            Self::VersionOverflow => Status::new(Severity::Fatal, facility::FILESYSTEM, 3, 0)
                .unwrap_or(Status::CORRUPT),
            Self::Io => Status::new(Severity::Error, facility::FILESYSTEM, 4, 0)
                .unwrap_or(Status::INVALID_ARGUMENT),
            Self::Interrupted => Status::BUSY,
            Self::QuotaExceeded => Status::NO_SPACE,
            Self::RepairUnauthorized => Status::ACCESS_DENIED,
            Self::StaleRepairPlan => Status::BUSY,
            Self::DowngradeRefused | Self::UnsupportedMigration | Self::InvalidMigrationLimit => {
                Status::INVALID_ARGUMENT
            }
            Self::MigrationAlreadyCurrent => Status::ALREADY_EXISTS,
            Self::MigrationInProgress | Self::MigrationIncomplete | Self::MigrationConflict => {
                Status::BUSY
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FileType {
    Regular = 1,
    Directory = 2,
    Symlink = 3,
}

impl FileType {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Regular),
            2 => Some(Self::Directory),
            3 => Some(Self::Symlink),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VolumeLimits {
    pub max_bytes: u64,
    pub max_files: u64,
    pub max_blocks: usize,
}

impl VolumeLimits {
    pub const UNLIMITED: Self = Self {
        max_bytes: u64::MAX,
        max_files: u64::MAX,
        max_blocks: usize::MAX,
    };
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct FileName {
    bytes: [u8; MAX_PATH_BYTES],
    len: u16,
}

impl FileName {
    pub fn new(path: &str) -> Result<Self, Error> {
        let value = path.as_bytes();
        if value.is_empty()
            || value.len() > MAX_PATH_BYTES
            || value.contains(&0)
            || value.contains(&b';')
            || value[0] == b'/' && value.len() == 1
            || value.ends_with(b"/")
            || value.windows(2).any(|pair| pair == b"//")
            || value
                .split(|byte| *byte == b'/')
                .any(|component| component == b"." || component == b"..")
        {
            return Err(Error::InvalidPath);
        }

        let mut bytes = [0; MAX_PATH_BYTES];
        bytes[..value.len()].copy_from_slice(value);
        Ok(Self {
            bytes,
            len: value.len() as u16,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or("")
    }
}

impl fmt::Debug for FileName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("FileName")
            .field(&self.as_str())
            .finish()
    }
}

impl Ord for FileName {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl PartialOrd for FileName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionSelector {
    Latest,
    Exact(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionedPath {
    pub file: FileName,
    pub version: VersionSelector,
}

impl VersionedPath {
    pub fn parse(path: &str) -> Result<Self, Error> {
        let Some((file, suffix)) = path.rsplit_once(';') else {
            return Ok(Self {
                file: FileName::new(path)?,
                version: VersionSelector::Latest,
            });
        };

        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Error::InvalidVersion);
        }
        let version = suffix.parse::<u32>().map_err(|_| Error::InvalidVersion)?;
        Ok(Self {
            file: FileName::new(file)?,
            version: if version == 0 {
                VersionSelector::Latest
            } else {
                VersionSelector::Exact(version)
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FileKey {
    file: FileName,
    version: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
struct BlockId(u32);

impl BlockId {
    const NONE: Self = Self(0);

    const fn is_some(self) -> bool {
        self.0 != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileRecord {
    key: FileKey,
    object_id: u64,
    size: u64,
    data: BlockId,
    checksum: u64,
    created_at: u64,
    deleted: bool,
    file_type: FileType,
    link_count: u32,
    mode: u16,
}

impl FileRecord {
    const EMPTY: Self = Self {
        key: FileKey {
            file: FileName {
                bytes: [0; MAX_PATH_BYTES],
                len: 0,
            },
            version: 0,
        },
        object_id: 0,
        size: 0,
        data: BlockId::NONE,
        checksum: 0,
        created_at: 0,
        deleted: false,
        file_type: FileType::Regular,
        link_count: 0,
        mode: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Leaf {
    len: u8,
    records: [FileRecord; MAX_KEYS],
}

impl Leaf {
    const EMPTY: Self = Self {
        len: 0,
        records: [FileRecord::EMPTY; MAX_KEYS],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Branch {
    len: u8,
    keys: [FileKey; MAX_KEYS],
    children: [BlockId; MAX_KEYS + 1],
}

impl Branch {
    const EMPTY: Self = Self {
        len: 0,
        keys: [FileRecord::EMPTY.key; MAX_KEYS],
        children: [BlockId::NONE; MAX_KEYS + 1],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TreeBlock {
    Leaf(Leaf),
    Branch(Branch),
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct DataBlock {
    next: BlockId,
    len: u16,
    checksum: u64,
    bytes: [u8; DATA_BYTES],
}

impl fmt::Debug for DataBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DataBlock")
            .field("next", &self.next)
            .field("len", &self.len)
            .field("checksum", &self.checksum)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Block {
    Tree(TreeBlock),
    Data(DataBlock),
}

#[derive(Clone, Copy)]
struct Slot {
    block: Option<Block>,
}

impl Slot {
    const EMPTY: Self = Self { block: None };
}

struct BlockArena<const MAX_BLOCKS: usize> {
    slots: [Slot; MAX_BLOCKS],
}

impl<const MAX_BLOCKS: usize> BlockArena<MAX_BLOCKS> {
    const fn new() -> Self {
        Self {
            slots: [Slot::EMPTY; MAX_BLOCKS],
        }
    }

    fn get(&self, id: BlockId) -> Result<Block, Error> {
        Ok(*self.get_ref(id)?)
    }

    fn get_ref(&self, id: BlockId) -> Result<&Block, Error> {
        if !id.is_some() {
            return Err(Error::Corrupt);
        }
        self.slots
            .get(id.0 as usize - 1)
            .and_then(|slot| slot.block.as_ref())
            .ok_or(Error::Corrupt)
    }

    fn allocate(&mut self, block: Block) -> Result<BlockId, Error> {
        let index = self
            .slots
            .iter()
            .position(|slot| slot.block.is_none())
            .ok_or(Error::OutOfSpace)?;
        self.slots[index].block = Some(block);
        Ok(BlockId(index as u32 + 1))
    }

    fn used(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.block.is_some())
            .count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileVersion {
    pub file: FileName,
    pub version: u32,
    pub size: u64,
    pub checksum: u64,
    pub created_at: u64,
    pub file_type: FileType,
    pub link_count: u32,
    pub mode: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub name: FileName,
    pub file_type: FileType,
    pub size: u64,
    pub version: u32,
    pub link_count: u32,
    pub mode: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinkEntry {
    pub path: FileName,
    /// Zero means the current version. Non-zero means an explicit version
    /// selector is needed to name this link.
    pub version: u32,
}

impl LinkEntry {
    pub const EMPTY: Self = Self {
        path: DirectoryEntry::EMPTY.name,
        version: 0,
    };
}

impl DirectoryEntry {
    pub const EMPTY: Self = Self {
        name: FileName {
            bytes: [0; MAX_PATH_BYTES],
            len: 0,
        },
        file_type: FileType::Regular,
        size: 0,
        version: 0,
        link_count: 0,
        mode: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadResult {
    pub file: FileVersion,
    pub bytes_read: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionCommit {
    pub generation: u64,
    pub operations: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GcReport {
    pub live_blocks: usize,
    pub freed_blocks: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SynFsDiagnostics {
    pub generation: u64,
    pub capacity_blocks: usize,
    pub allocated_blocks: usize,
    pub live_blocks: usize,
    pub cow_snapshot_blocks: usize,
    pub retained_versions: u64,
    pub retained_bytes: u64,
    pub file_count: u64,
    pub checkpoints: usize,
    pub free_blocks: usize,
    pub free_bytes: u64,
    pub max_bytes: u64,
    pub max_files: u64,
    pub max_blocks: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CheckpointId(u64);

impl CheckpointId {
    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn from_valid_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointInfo {
    pub id: CheckpointId,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RollbackInfo {
    pub from_generation: u64,
    pub to_generation: u64,
}

#[derive(Clone, Copy)]
struct Checkpoint {
    info: CheckpointInfo,
    root: BlockId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappedFilePage<'a> {
    pub file_offset: u64,
    pub bytes: &'a [u8],
    pub checksum: u64,
}

/// Capability-tied view over one immutable CoW B-tree generation.
pub struct ReadOnlySnapshot<'a, const MAX_BLOCKS: usize> {
    capability: RmsMapHandle,
    generation: u64,
    root: BlockId,
    filesystem: &'a SynFs<MAX_BLOCKS>,
}

impl<'a, const MAX_BLOCKS: usize> ReadOnlySnapshot<'a, MAX_BLOCKS> {
    pub const fn capability(&self) -> RmsMapHandle {
        self.capability
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn lookup(&self, path: &str) -> Result<FileVersion, Error> {
        self.filesystem.lookup_at(self.root, path)
    }

    /// Return one live file version by its stable in-snapshot ordinal.
    pub fn file_at(&self, index: u32) -> Result<Option<FileVersion>, Error> {
        let Some(record) = self.filesystem.record_at(self.root, index)? else {
            return Ok(None)
        };
        let mut file: FileVersion = record.into();
        file.link_count = self
            .filesystem
            .link_count_at(self.root, record.object_id)?;
        Ok(Some(file))
    }

    pub fn list_directory(
        &self,
        path: &str,
        entries: &mut [DirectoryEntry],
    ) -> Result<usize, Error> {
        if path == "/" {
            return self.filesystem.list_directory_at(self.root, path, entries)
        }
        let directory = self
            .filesystem
            .lookup_following_buffer(self.root, path.as_bytes())?;
        if directory.file_type != FileType::Directory {
            return Err(Error::NotDirectory)
        }
        self.filesystem
            .list_directory_at(self.root, directory.file.as_str(), entries)
    }

    /// Expand a wildcard over names visible at the selected version.
    ///
    /// Without a selector, or with `;0`, only latest live names are returned.
    /// With `;N`, only live retained version `N` is returned for each matching
    /// name. The returned names do not carry `;N`; callers must preserve the
    /// selector when they perform the operation.
    pub fn expand_paths<const CAPACITY: usize>(
        &self,
        pattern: &str,
        output: &mut [Option<FileName>; CAPACITY],
    ) -> Result<usize, Error> {
        let (count, next) = self.expand_paths_page(pattern, 0, output)?;
        next.map_or(Ok(count), |next| {
            Err(Error::BufferTooSmall {
                required: next.saturating_add(1),
            })
        })
    }

    /// Return one bounded, sorted wildcard page. Continuation is the stable
    /// zero-based match ordinal, not an internal record position.
    pub fn expand_paths_page<const CAPACITY: usize>(
        &self,
        pattern: &str,
        continuation: usize,
        output: &mut [Option<FileName>; CAPACITY],
    ) -> Result<(usize, Option<usize>), Error> {
        let versioned = VersionedPath::parse(pattern)?;
        let pattern = Pattern::parse(versioned.file.as_str())
            .map_err(|_| Error::InvalidPattern)?;
        output.fill(None);
        let mut matches = [None; MAX_WILDCARD_MATCHES];
        let mut match_count = 0;
        let mut ordinal = 0;
        while let Some(record) = self.filesystem.record_at(self.root, ordinal)? {
            ordinal = ordinal.saturating_add(1);
            let selected = match versioned.version {
                VersionSelector::Latest => {
                    !record.deleted
                        && self.filesystem.latest_record_at(self.root, record.key.file)?
                            == Some(record)
                }
                VersionSelector::Exact(version) => {
                    !record.deleted && record.key.version == version
                }
            };
            if !selected || !pattern.matches(record.key.file.as_str()) {
                continue
            }
            if matches[..match_count]
                .iter()
                .flatten()
                .any(|path| *path == record.key.file)
            {
                continue
            }
            if match_count == matches.len() {
                return Err(Error::BufferTooSmall {
                    required: match_count.saturating_add(1),
                })
            }
            let insert_at = matches[..match_count]
                .iter()
                .position(|path| path.is_some_and(|path| path > record.key.file))
                .unwrap_or(match_count);
            for index in (insert_at..match_count).rev() {
                matches[index + 1] = matches[index];
            }
            matches[insert_at] = Some(record.key.file);
            match_count += 1;
        }
        if continuation >= match_count {
            return Ok((0, None))
        }
        let end = continuation.saturating_add(CAPACITY).min(match_count);
        for (slot, path) in matches[continuation..end].iter().flatten().copied().enumerate() {
            output[slot] = Some(path);
        }
        Ok((end - continuation, (end < match_count).then_some(end)))
    }

    /// Read a range from one file version without leaving the checkpoint tree.
    pub fn read_file_at(
        &self,
        index: u32,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, Error> {
        let record = self
            .filesystem
            .record_at(self.root, index)?
            .ok_or(Error::NotFound)?;
        self.filesystem
            .read_record_range(record, offset, destination)
    }

    /// Visit immutable file pages directly, without copying through a daemon.
    pub fn visit_file_pages(
        &self,
        path: &str,
        mut visitor: impl FnMut(MappedFilePage<'a>),
    ) -> Result<FileVersion, Error> {
        let file = self.filesystem.lookup_at(self.root, path)?;
        let record = self
            .filesystem
            .find_record_at(
                self.root,
                FileKey {
                    file: file.file,
                    version: file.version,
                },
            )?
            .ok_or(Error::Corrupt)?;
        let mut id = record.data;
        let mut file_offset = 0_u64;
        while id.is_some() {
            let Block::Data(block) = self.filesystem.arena.get_ref(id)? else {
                return Err(Error::Corrupt);
            };
            let length = block.len as usize;
            if length == 0 || length > DATA_BYTES || checksum(&block.bytes[..length]) != block.checksum {
                return Err(Error::Corrupt);
            }
            visitor(MappedFilePage {
                file_offset,
                bytes: &block.bytes[..length],
                checksum: block.checksum,
            });
            file_offset = file_offset.saturating_add(length as u64);
            id = block.next;
        }
        if file_offset != file.size {
            return Err(Error::Corrupt);
        }
        Ok(file)
    }
}

#[derive(Clone, Copy)]
struct Split {
    separator: FileKey,
    right: BlockId,
}

#[derive(Clone, Copy)]
struct Inserted {
    left: BlockId,
    split: Option<Split>,
}

/// Fixed-capacity GhostFS metadata and data block store.
///
/// Every tree update writes a new path from the changed leaf to the root.
/// Existing roots remain valid until commit, while unchanged data blocks are
/// shared by identity. This keeps file versions immutable without a kernel heap.
pub struct SynFs<const MAX_BLOCKS: usize> {
    arena: BlockArena<MAX_BLOCKS>,
    root: BlockId,
    generation: u64,
    limits: VolumeLimits,
    checkpoints: [Option<Checkpoint>; MAX_CHECKPOINTS],
    next_checkpoint: u64,
    volume_bank: usize,
    volume_sequence: u64,
    next_object_id: u64,
    format_version: u16,
}

/// An atomic group of GhostFS B+tree changes.
///
/// New data and tree paths remain unreachable from the committed root until
/// `commit` is called. Dropping an unfinished transaction restores the prior
/// root and reclaims its abandoned CoW blocks.
pub struct SynFsTransaction<'a, const MAX_BLOCKS: usize> {
    filesystem: &'a mut SynFs<MAX_BLOCKS>,
    original_root: BlockId,
    original_generation: u64,
    operations: u32,
    committed: bool,
    failed: bool,
}

impl<const MAX_BLOCKS: usize> SynFsTransaction<'_, MAX_BLOCKS> {
    /// Current private CoW generation, including staged changes.
    pub const fn generation(&self) -> u64 {
        self.filesystem.generation
    }

    pub const fn operations(&self) -> u32 {
        self.operations
    }

    pub fn lookup(&self, path: &str) -> Result<FileVersion, Error> {
        self.filesystem.lookup(path)
    }

    pub fn read(&self, path: &str, destination: &mut [u8]) -> Result<ReadResult, Error> {
        self.filesystem.read(path, destination)
    }

    pub fn write(&mut self, path: &str, contents: &[u8]) -> Result<FileVersion, Error> {
        if self.failed {
            return Err(Error::TransactionAborted);
        }
        let file = match self.filesystem.write_uncommitted(path, contents) {
            Ok(file) => file,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        self.operations = match self.operations.checked_add(1) {
            Some(operations) => operations,
            None => {
                self.failed = true;
                return Err(Error::VersionOverflow);
            }
        };
        Ok(file)
    }

    pub fn delete(&mut self, path: &str) -> Result<FileVersion, Error> {
        if self.failed {
            return Err(Error::TransactionAborted);
        }
        let file = match self.filesystem.delete_uncommitted(path) {
            Ok(file) => file,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        self.operations = match self.operations.checked_add(1) {
            Some(operations) => operations,
            None => {
                self.failed = true;
                return Err(Error::VersionOverflow);
            }
        };
        Ok(file)
    }

    pub fn create_directory(&mut self, path: &str, recursive: bool) -> Result<FileVersion, Error> {
        if self.failed {
            return Err(Error::TransactionAborted)
        }
        let result = self.filesystem.create_directory(path, recursive);
        match result {
            Ok(file) => {
                self.operations = self.operations.checked_add(1).ok_or_else(|| {
                    self.failed = true;
                    Error::VersionOverflow
                })?;
                Ok(file)
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    pub fn remove_directory(&mut self, path: &str) -> Result<FileVersion, Error> {
        if self.failed {
            return Err(Error::TransactionAborted)
        }
        let result = self.filesystem.remove_directory(path);
        match result {
            Ok(file) => {
                self.operations = self.operations.checked_add(1).ok_or_else(|| {
                    self.failed = true;
                    Error::VersionOverflow
                })?;
                Ok(file)
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    /// Stage a rename inside this transaction.
    ///
    /// The old root remains recoverable until the transaction commits. If the
    /// transaction is dropped or fails, all staged name changes disappear.
    pub fn rename(&mut self, old_path: &str, new_path: &str) -> Result<FileVersion, Error> {
        if self.failed {
            return Err(Error::TransactionAborted)
        }
        let result = self.filesystem.rename_uncommitted(old_path, new_path);
        match result {
            Ok(file) => {
                self.operations = self.operations.checked_add(1).ok_or_else(|| {
                    self.failed = true;
                    Error::VersionOverflow
                })?;
                Ok(file)
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    /// Publish all staged CoW changes with one root swap.
    ///
    /// Commit gives atomic in-memory visibility, not power-loss durability;
    /// the volume owner must call `SynFs::sync` afterward.
    pub fn commit(mut self) -> Result<TransactionCommit, Error> {
        if self.failed {
            return Err(Error::TransactionAborted);
        }
        self.committed = true;
        Ok(TransactionCommit {
            generation: self.filesystem.generation,
            operations: self.operations,
        })
    }
}

impl<const MAX_BLOCKS: usize> Drop for SynFsTransaction<'_, MAX_BLOCKS> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.filesystem.root = self.original_root;
        self.filesystem.generation = self.original_generation;
        self.filesystem.collect_garbage();
    }
}

impl<const MAX_BLOCKS: usize> SynFs<MAX_BLOCKS> {
    pub const fn new() -> Self {
        Self {
            arena: BlockArena::new(),
            root: BlockId::NONE,
            generation: 0,
            limits: VolumeLimits::UNLIMITED,
            checkpoints: [None; MAX_CHECKPOINTS],
            next_checkpoint: 1,
            volume_bank: 1,
            volume_sequence: 0,
            next_object_id: 1,
            format_version: volume::VOLUME_FORMAT_VERSION,
        }
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// TEMPORARY first-run diagnostics: summarize where a file's payload lives.
    /// Returns (size, slot index, kind, record checksum, first bytes as u64).
    /// kind: 0=no data block, 1=data block, 2=tree block, 3=missing slot.
    #[doc(hidden)]
    pub fn debug_data_probe(&self, path: &str) -> Result<(u64, u64, u8, u64, u64), Error> {
        let file = self.lookup_following(path)?;
        let record = self
            .find_record(FileKey {
                file: file.file,
                version: file.version,
            })?
            .ok_or(Error::Corrupt)?;
        if !record.data.is_some() {
            return Ok((record.size, u64::MAX, 0, record.checksum, 0));
        }
        let raw = record.data.0 as usize;
        if raw == 0 || raw > MAX_BLOCKS {
            return Err(Error::Corrupt);
        }
        match &self.arena.slots[raw - 1].block {
            Some(Block::Data(data)) => {
                let mut head = [0u8; 8];
                head.copy_from_slice(&data.bytes[..8]);
                Ok((
                    record.size,
                    raw as u64,
                    1,
                    record.checksum,
                    u64::from_le_bytes(head),
                ))
            }
            Some(Block::Tree(_)) => Ok((record.size, raw as u64, 2, record.checksum, 0)),
            None => Ok((record.size, raw as u64, 3, record.checksum, 0)),
        }
    }

    /// TEMPORARY first-run diagnostics: raw arena shape.
    /// Returns (packed slot kinds, root id, generation); kinds are 4-bit
    /// fields, 16 per word: 0=free, 1=data, 2=tree. Covers the first 80
    /// slots.
    #[doc(hidden)]
    pub fn debug_arena_dump(&self) -> ([u64; 5], u64, u64) {
        let mut words = [0u64; 5];
        for (index, slot) in self.arena.slots.iter().enumerate().take(80) {
            let kind = match &slot.block {
                None => 0u64,
                Some(Block::Data(_)) => 1,
                Some(Block::Tree(_)) => 2,
            };
            words[index / 16] |= kind << ((index % 16) * 4);
        }
        (words, self.root.0 as u64, self.generation)
    }

    pub const fn format_version(&self) -> u16 {
        self.format_version
    }

    /// Merge a cluster monotonic counter before starting a CoW update.
    ///
    /// A node may receive a larger counter from a peer after its last local
    /// write. Advancing the private generation first prevents a stale node
    /// from publishing a lower creation order after failover.
    pub fn synchronize_generation(&mut self, cluster_counter: u64) -> Result<(), Error> {
        if cluster_counter == 0 {
            return Err(Error::InvalidVersion)
        }
        if cluster_counter == u64::MAX {
            return Err(Error::VersionOverflow)
        }
        self.generation = self.generation.max(cluster_counter);
        Ok(())
    }

    pub fn used_blocks(&self) -> usize {
        self.arena.used()
    }

    pub const fn capacity(&self) -> usize {
        MAX_BLOCKS
    }

    pub const fn limits(&self) -> VolumeLimits {
        self.limits
    }

    pub fn set_limits(&mut self, limits: VolumeLimits) -> Result<(), Error> {
        if limits.max_blocks != usize::MAX && limits.max_blocks > MAX_BLOCKS {
            return Err(Error::QuotaExceeded);
        }
        let diagnostics = self.diagnostics()?;
        if diagnostics.retained_bytes > limits.max_bytes
            || diagnostics.file_count > limits.max_files
            || self.arena.used() > limits.max_blocks
        {
            return Err(Error::QuotaExceeded);
        }
        self.limits = limits;
        Ok(())
    }

    pub fn free_blocks(&self) -> usize {
        self.limits.max_blocks.min(MAX_BLOCKS).saturating_sub(self.arena.used())
    }

    pub fn free_bytes(&self) -> u64 {
        (self.free_blocks() as u64).saturating_mul(BLOCK_SIZE as u64)
    }

    /// Read-only accounting for `SHOW DISK`.
    ///
    /// `cow_snapshot_blocks` counts blocks reachable only through pinned
    /// checkpoint roots. Unreachable arena garbage is excluded from live and
    /// snapshot usage while remaining visible in `allocated_blocks`.
    pub fn diagnostics(&self) -> Result<SynFsDiagnostics, Error> {
        let mut current = [false; MAX_BLOCKS];
        self.mark_reachable(self.root, &mut current)?;

        let mut all_live = current;
        for checkpoint in self.checkpoints.iter().flatten() {
            self.mark_reachable(checkpoint.root, &mut all_live)?
        }

        let live_blocks = all_live.iter().filter(|marked| **marked).count();
        let cow_snapshot_blocks = all_live
            .iter()
            .zip(current.iter())
            .filter(|(live, current)| **live && !**current)
            .count();
        let mut retained_versions = 0u64;
        let mut retained_bytes = 0u64;
        let mut file_count = 0u64;
        let mut previous_file = None;
        let mut ordinal = 0u32;
        while let Some(record) = self.record_at(self.root, ordinal)? {
            retained_versions = retained_versions.saturating_add(1);
            retained_bytes = retained_bytes.saturating_add(record.size);
            if previous_file != Some(record.key.file) {
                file_count = file_count.saturating_add(1);
                previous_file = Some(record.key.file)
            }
            ordinal = ordinal.saturating_add(1)
        }

        Ok(SynFsDiagnostics {
            generation: self.generation,
            capacity_blocks: MAX_BLOCKS,
            allocated_blocks: self.arena.used(),
            live_blocks,
            cow_snapshot_blocks,
            retained_versions,
            retained_bytes,
            file_count,
            checkpoints: self.checkpoints.iter().flatten().count(),
            free_blocks: self.free_blocks(),
            free_bytes: self.free_bytes(),
            max_bytes: self.limits.max_bytes,
            max_files: self.limits.max_files,
            max_blocks: self.limits.max_blocks,
        })
    }

    pub fn transaction(&mut self) -> SynFsTransaction<'_, MAX_BLOCKS> {
        // Atomic groups need room for all CoW paths; reclaim abandoned blocks first.
        self.collect_garbage();
        SynFsTransaction {
            original_root: self.root,
            original_generation: self.generation,
            filesystem: self,
            operations: 0,
            committed: false,
            failed: false,
        }
    }

    /// Bind a validated kernel capability to the current immutable tree root.
    pub const fn mapped_snapshot(
        &self,
        capability: RmsMapHandle,
    ) -> ReadOnlySnapshot<'_, MAX_BLOCKS> {
        ReadOnlySnapshot {
            capability,
            generation: self.generation,
            root: self.root,
            filesystem: self,
        }
    }

    /// Pin the current CoW root so later writes and collection cannot change it.
    pub fn create_checkpoint(&mut self) -> Result<CheckpointInfo, Error> {
        let slot = self
            .checkpoints
            .iter_mut()
            .find(|checkpoint| checkpoint.is_none())
            .ok_or(Error::TooManyCheckpoints)?;
        let id = CheckpointId(self.next_checkpoint);
        self.next_checkpoint = self
            .next_checkpoint
            .checked_add(1)
            .ok_or(Error::VersionOverflow)?;
        let checkpoint = Checkpoint {
            info: CheckpointInfo {
                id,
                generation: self.generation,
            },
            root: self.root,
        };
        *slot = Some(checkpoint);
        Ok(checkpoint.info)
    }

    pub fn checkpoint_info(&self, id: CheckpointId) -> Result<CheckpointInfo, Error> {
        self.find_checkpoint(id).map(|checkpoint| checkpoint.info)
    }

    pub fn checkpoint_snapshot(
        &self,
        id: CheckpointId,
        capability: RmsMapHandle,
    ) -> Result<ReadOnlySnapshot<'_, MAX_BLOCKS>, Error> {
        let checkpoint = self.find_checkpoint(id)?;
        Ok(ReadOnlySnapshot {
            capability,
            generation: checkpoint.info.generation,
            root: checkpoint.root,
            filesystem: self,
        })
    }

    pub fn release_checkpoint(&mut self, id: CheckpointId) -> Result<(), Error> {
        let slot = self
            .checkpoints
            .iter_mut()
            .find(|checkpoint| checkpoint.is_some_and(|value| value.info.id == id))
            .ok_or(Error::CheckpointNotFound)?;
        *slot = None;
        Ok(())
    }

    /// Make a pinned checkpoint the live root in one pointer swap.
    ///
    /// The checkpoint stays pinned after this call so the caller can verify
    /// and release it only after related state has been restored.
    pub fn rollback_to_checkpoint(&mut self, id: CheckpointId) -> Result<RollbackInfo, Error> {
        let checkpoint = self.find_checkpoint(id)?;
        let info = RollbackInfo {
            from_generation: self.generation,
            to_generation: checkpoint.info.generation,
        };
        self.root = checkpoint.root;
        self.generation = checkpoint.info.generation;
        self.collect_garbage();
        Ok(info)
    }

    pub fn restore_checkpoint(&mut self, id: CheckpointId) -> Result<RollbackInfo, Error> {
        self.rollback_to_checkpoint(id)
    }

    pub fn write(&mut self, path: &str, contents: &[u8]) -> Result<FileVersion, Error> {
        let resolved = match self.lookup(path) {
            Ok(file) if file.file_type == FileType::Symlink => {
                Some(self.lookup_following(path)?.file)
            }
            Ok(_) | Err(Error::NotFound) => None,
            Err(error) => return Err(error),
        };
        let result = match resolved {
            Some(file) => self.write_uncommitted(file.as_str(), contents),
            None => self.write_uncommitted(path, contents),
        };
        if result == Err(Error::OutOfSpace) {
            self.collect_garbage();
            return match resolved {
                Some(file) => self.write_uncommitted(file.as_str(), contents),
                None => self.write_uncommitted(path, contents),
            };
        }
        if result.is_err() {
            self.collect_garbage();
        }
        result
    }

    /// Create a symbolic link. The target is stored as link data and is not
    /// required to exist yet.
    pub fn symlink(&mut self, target: &str, link_path: &str) -> Result<FileVersion, Error> {
        let target = target.as_bytes();
        if target.is_empty()
            || target.len() > MAX_PATH_BYTES
            || target.contains(&0)
            || target.contains(&b';')
            || core::str::from_utf8(target).is_err()
        {
            return Err(Error::InvalidPath)
        }
        let link = FileName::new(link_path)?;
        if self.latest_record_at(self.root, link)?.is_some() {
            return Err(Error::AlreadyExists)
        }
        self.require_parent_directory(link)?;
        self.enforce_limits(target.len() as u64, true)?;
        let data = self.store_data(target)?;
        let version = self.next_version(link)?;
        let object_id = self.allocate_object_id()?;
        self.insert_record(
            link,
            version,
            object_id,
            self.generation.saturating_add(1),
            target.len() as u64,
            data,
            checksum(target),
            FileType::Symlink,
            1,
            0o777,
        )
    }

    /// Change the permission bits on one live filesystem object.
    pub fn set_mode(&mut self, path: &str, mode: u16) -> Result<FileVersion, Error> {
        let selected = self.lookup_record(path)?;
        let mut updated = selected;
        updated.mode = mode & 0o7777;
        self.root = self.replace(self.root, selected.key, updated)?;
        self.generation = self.generation.saturating_add(1);
        Ok(updated.into())
    }

    fn write_uncommitted(&mut self, path: &str, contents: &[u8]) -> Result<FileVersion, Error> {
        let parsed = VersionedPath::parse(path)?;
        if parsed.version != VersionSelector::Latest || path.contains(';') {
            return Err(Error::InvalidVersion);
        }

        let previous = self.last_record(parsed.file)?;
        if previous.is_some_and(|record| record.file_type == FileType::Directory && !record.deleted) {
            return Err(Error::NotDirectory);
        }
        self.require_parent_directory(parsed.file)?;
        self.enforce_limits(contents.len() as u64, previous.is_none())?;
        let version = match previous {
            Some(record)
                if !record.deleted
                    && record.key.version == 1
                    && record.size == 0
                    && !record.data.is_some() =>
            {
                record.key.version
            }
            Some(record) => record
                .key
                .version
                .checked_add(1)
                .ok_or(Error::VersionOverflow)?,
            None => 1,
        };
        let created_at = self.generation.saturating_add(1);
        let object_id = previous
            .filter(|record| !record.deleted)
            .map_or_else(|| self.allocate_object_id(), |record| Ok(record.object_id))?;
        self.write_inner(parsed.file, version, object_id, created_at, contents)
    }

    fn write_inner(
        &mut self,
        file: FileName,
        version: u32,
        object_id: u64,
        created_at: u64,
        contents: &[u8],
    ) -> Result<FileVersion, Error> {
        let data = self.store_data(contents)?;
        self.insert_record(
            file,
            version,
            object_id,
            created_at,
            contents.len() as u64,
            data,
            checksum(contents),
            FileType::Regular,
            1,
            0o666,
        )
    }

    fn insert_record(
        &mut self,
        file: FileName,
        version: u32,
        object_id: u64,
        created_at: u64,
        size: u64,
        data: BlockId,
        record_checksum: u64,
        file_type: FileType,
        link_count: u32,
        mode: u16,
    ) -> Result<FileVersion, Error> {
        let record = FileRecord {
            key: FileKey { file, version },
            object_id,
            size,
            data,
            checksum: record_checksum,
            created_at,
            deleted: false,
            file_type,
            link_count: link_count.max(1),
            mode,
        };
        let root = if self.find_record(record.key)?.is_some() {
            self.replace(self.root, record.key, record)?
        } else {
            self.insert(self.root, record)?
        };
        self.root = root;
        self.generation = created_at;
        Ok(record.into())
    }

    pub fn create_directory(&mut self, path: &str, recursive: bool) -> Result<FileVersion, Error> {
        let file = FileName::new(path)?;
        if self.latest_record_at(self.root, file)?.is_some() {
            return Err(Error::AlreadyExists);
        }
        if recursive {
            let bytes = file.as_bytes();
            let mut end = 0;
            while let Some(relative) = bytes[end..].iter().position(|byte| *byte == b'/') {
                let boundary = end + relative;
                if boundary != 0 {
                    let prefix = core::str::from_utf8(&bytes[..boundary]).map_err(|_| Error::InvalidPath)?;
                    if self.latest_record_at(self.root, FileName::new(prefix)?)?.is_none() {
                        self.create_directory(prefix, false)?;
                    }
                }
                end = boundary + 1;
            }
        }
        self.require_parent_directory(file)?;
        self.enforce_limits(0, true)?;
        let version = self.next_version(file)?;
        self.insert_record(
            file,
            version,
            0,
            self.generation.saturating_add(1),
            0,
            BlockId::NONE,
            checksum(&[]),
            FileType::Directory,
            1,
            0o777,
        )
    }

    pub fn remove_directory(&mut self, path: &str) -> Result<FileVersion, Error> {
        let file = FileName::new(path)?;
        let directory = self.latest_record_at(self.root, file)?.ok_or(Error::NotFound)?;
        if directory.file_type != FileType::Directory {
            return Err(Error::NotDirectory);
        }
        let mut ordinal = 0;
        while let Some(record) = self.record_at(self.root, ordinal)? {
            ordinal = ordinal.saturating_add(1);
            let Some(rest) = record.key.file.as_str().strip_prefix(file.as_str())
                .and_then(|rest| rest.strip_prefix('/')) else {
                continue;
            };
            if !rest.contains('/')
                && self.latest_record_at(self.root, record.key.file)? == Some(record)
            {
                return Err(Error::DirectoryNotEmpty);
            }
        }
        self.tombstone_latest(directory)
    }

    pub fn link(&mut self, existing: &str, new_path: &str) -> Result<FileVersion, Error> {
        let source = self.lookup_record(existing)?;
        if source.file_type != FileType::Regular || source.object_id == 0 {
            return Err(Error::NotDirectory);
        }
        let parsed_target = VersionedPath::parse(new_path)?;
        if parsed_target.version != VersionSelector::Latest || new_path.contains(';') {
            return Err(Error::InvalidVersion);
        }
        let target = parsed_target.file;
        if self.latest_record_at(self.root, target)?.is_some() {
            return Err(Error::AlreadyExists);
        }
        self.require_parent_directory(target)?;
        self.enforce_limits(0, true)?;
        let version = self.next_version(target)?;
        self.insert_record(
            target,
            version,
            source.object_id,
            self.generation.saturating_add(1),
            source.size,
            source.data,
            source.checksum,
            source.file_type,
            source.link_count.saturating_add(1),
            source.mode,
        )?;
        self.lookup(target.as_str())
    }

    /// Atomically publish one rename in the in-memory filesystem generation.
    ///
    /// The rename becomes power-loss durable only after [`Self::fsync`]
    /// succeeds.
    pub fn rename(&mut self, old_path: &str, new_path: &str) -> Result<FileVersion, Error> {
        let mut no_interruption = NoInterruption;
        self.rename_with_interruption(old_path, new_path, &mut no_interruption)
    }

    pub fn rename_with_interruption<I: InterruptionInjector>(
        &mut self,
        old_path: &str,
        new_path: &str,
        injector: &mut I,
    ) -> Result<FileVersion, Error> {
        let mut retried = false;
        loop {
            let result = (|| {
                let mut transaction = self.transaction();
                transaction.rename(old_path, new_path)?;
                transaction.commit()
            })();
            match result {
                Ok(_) => {
                    if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::Rename) {
                        return Err(Error::Interrupted)
                    }
                    return self.lookup(new_path)
                }
                Err(Error::OutOfSpace) if !retried => {
                    retried = true;
                    self.collect_garbage();
                }
                Err(error) => {
                    self.collect_garbage();
                    return Err(error)
                }
            }
        }
    }

    fn rename_uncommitted(&mut self, old_path: &str, new_path: &str) -> Result<FileVersion, Error> {
        let old = FileName::new(old_path)?;
        let new = FileName::new(new_path)?;
        let source = self.lookup_record(old.as_str())?;
        if self.latest_record_at(self.root, new)?.is_some() {
            return Err(Error::AlreadyExists);
        }
        if new.as_str().starts_with(old.as_str())
            && new.as_bytes().get(old.as_bytes().len()) == Some(&b'/')
        {
            return Err(Error::InvalidPath);
        }
        self.require_parent_directory(new)?;
        let mut first = None;
        let mut moved = 0;
        loop {
            let mut ordinal = 0;
            let mut candidate = None;
            while let Some(record) = self.record_at(self.root, ordinal)? {
                ordinal = ordinal.saturating_add(1);
                let is_source = record.key.file == old
                    || (source.file_type == FileType::Directory
                        && record.key.file.as_str().starts_with(old.as_str())
                        && record.key.file.as_bytes().get(old.as_bytes().len()) == Some(&b'/'));
                if is_source && self.latest_record_at(self.root, record.key.file)? == Some(record) {
                    candidate = Some(record);
                    break;
                }
            }
            let Some(record) = candidate else {
                break
            };
            let target = Self::renamed_name(old, new, record.key.file)?;
            if self.latest_record_at(self.root, target)?.is_some() {
                return Err(Error::AlreadyExists);
            }
            let version = self.next_version(target)?;
            let moved_file = self.insert_record(
                target,
                version,
                record.object_id,
                self.generation.saturating_add(1),
                record.size,
                record.data,
                record.checksum,
                record.file_type,
                record.link_count,
                record.mode,
            )?;
            self.root = self.tombstone(self.root, record.key)?;
            first = Some(first.unwrap_or(moved_file));
            moved += 1;
        }
        if moved == 0 {
            return Err(Error::NotFound)
        }
        first.ok_or(Error::Corrupt)
    }

    fn renamed_name(old: FileName, new: FileName, current: FileName) -> Result<FileName, Error> {
        let suffix = current
            .as_str()
            .strip_prefix(old.as_str())
            .and_then(|value| value.strip_prefix('/'))
            .unwrap_or("");
        let mut bytes = [0; MAX_PATH_BYTES];
        let new_length = new.as_bytes().len();
        let separator = usize::from(!suffix.is_empty());
        let total = new_length
            .checked_add(separator)
            .and_then(|length| length.checked_add(suffix.len()))
            .ok_or(Error::InvalidPath)?;
        if total > MAX_PATH_BYTES {
            return Err(Error::InvalidPath)
        }
        bytes[..new_length].copy_from_slice(new.as_bytes());
        if separator != 0 {
            bytes[new_length] = b'/';
            bytes[new_length + 1..total].copy_from_slice(suffix.as_bytes());
        }
        FileName::new(core::str::from_utf8(&bytes[..total]).map_err(|_| Error::InvalidPath)?)
    }

    pub fn list_directory(&self, path: &str, entries: &mut [DirectoryEntry]) -> Result<usize, Error> {
        if path == "/" {
            return self.list_directory_at(self.root, path, entries)
        }
        let directory = self.lookup_following(path)?;
        if directory.file_type != FileType::Directory {
            return Err(Error::NotDirectory)
        }
        self.list_directory_at(self.root, directory.file.as_str(), entries)
    }

    /// Expand a bounded wildcard over names visible at the selected version.
    /// Without a selector, or with `;0`, results contain only latest live
    /// names. With `;N`, results contain only live retained version `N`.
    /// Results are sorted by canonical path and never include deleted records.
    pub fn expand_paths<const CAPACITY: usize>(
        &self,
        pattern: &str,
        output: &mut [Option<FileName>; CAPACITY],
    ) -> Result<usize, Error> {
        let (count, next) = self.expand_paths_page(pattern, 0, output)?;
        next.map_or(Ok(count), |next| {
            Err(Error::BufferTooSmall {
                required: next.saturating_add(1),
            })
        })
    }

    /// Return one bounded, sorted wildcard page. Continuation is the stable
    /// zero-based match ordinal, not an internal record position.
    pub fn expand_paths_page<const CAPACITY: usize>(
        &self,
        pattern: &str,
        continuation: usize,
        output: &mut [Option<FileName>; CAPACITY],
    ) -> Result<(usize, Option<usize>), Error> {
        let versioned = VersionedPath::parse(pattern)?;
        let pattern = Pattern::parse(versioned.file.as_str())
            .map_err(|_| Error::InvalidPattern)?;
        output.fill(None);
        let mut matches = [None; MAX_WILDCARD_MATCHES];
        let mut match_count = 0;
        let mut ordinal = 0;
        while let Some(record) = self.record_at(self.root, ordinal)? {
            ordinal = ordinal.saturating_add(1);
            let selected = match versioned.version {
                VersionSelector::Latest => {
                    !record.deleted
                        && self.latest_record_at(self.root, record.key.file)? == Some(record)
                }
                VersionSelector::Exact(version) => {
                    !record.deleted && record.key.version == version
                }
            };
            if !selected || !pattern.matches(record.key.file.as_str()) {
                continue
            }
            if matches[..match_count]
                .iter()
                .flatten()
                .any(|path| *path == record.key.file)
            {
                continue
            }
            if match_count == matches.len() {
                return Err(Error::BufferTooSmall {
                    required: match_count.saturating_add(1),
                })
            }
            let insert_at = matches[..match_count]
                .iter()
                .position(|path| path.is_some_and(|path| path > record.key.file))
                .unwrap_or(match_count);
            for index in (insert_at..match_count).rev() {
                matches[index + 1] = matches[index];
            }
            matches[insert_at] = Some(record.key.file);
            match_count += 1;
        }
        if continuation >= match_count {
            return Ok((0, None))
        }
        let end = continuation.saturating_add(CAPACITY).min(match_count);
        for (slot, path) in matches[continuation..end].iter().flatten().copied().enumerate() {
            output[slot] = Some(path);
        }
        Ok((end - continuation, (end < match_count).then_some(end)))
    }

    pub fn list_links(&self, path: &str, entries: &mut [LinkEntry]) -> Result<usize, Error> {
        let selected = self.lookup_record(path)?;
        let mut written = 0;
        let mut ordinal = 0;
        while let Some(record) = self.record_at(self.root, ordinal)? {
            ordinal = ordinal.saturating_add(1);
            if record.object_id != selected.object_id
                || record.deleted
                || self.latest_record_at(self.root, record.key.file)? != Some(record)
            {
                continue;
            }
            if entries.get(written).is_none() {
                return Err(Error::BufferTooSmall {
                    required: written.saturating_add(1),
                });
            }
            entries[written] = LinkEntry {
                path: record.key.file,
                version: if self.latest_record_at(self.root, record.key.file)? == Some(record) {
                    0
                } else {
                    record.key.version
                },
            };
            written += 1;
        }
        Ok(written)
    }

    fn list_directory_at(
        &self,
        root: BlockId,
        path: &str,
        entries: &mut [DirectoryEntry],
    ) -> Result<usize, Error> {
        if !self.is_directory_path_at(root, path)? {
            return Err(Error::NotDirectory);
        }
        let mut written = 0;
        let mut ordinal = 0;
        while let Some(record) = self.record_at(root, ordinal)? {
            ordinal = ordinal.saturating_add(1);
            if record.deleted {
                continue;
            }
            let Some(name) = (if path == "/" {
                Some(record.key.file.as_str().strip_prefix('/').unwrap_or(record.key.file.as_str()))
            } else {
                record
                    .key.file
                    .as_str()
                    .strip_prefix(path)
                    .and_then(|rest| rest.strip_prefix('/'))
            }) else {
                continue;
            };
            if name.contains('/') {
                continue;
            }
            if entries.get(written).is_none() {
                return Err(Error::BufferTooSmall {
                    required: written.saturating_add(1),
                });
            }
            entries[written] = DirectoryEntry {
                name: FileName::new(name)?,
                file_type: record.file_type,
                size: record.size,
                version: record.key.version,
                link_count: self.link_count_at(root, record.object_id)?,
                mode: record.mode,
            };
            written += 1;
        }
        Ok(written)
    }

    fn lookup_record(&self, path: &str) -> Result<FileRecord, Error> {
        let parsed = VersionedPath::parse(path)?;
        let record = match parsed.version {
            VersionSelector::Latest => self.latest_record_at(self.root, parsed.file),
            VersionSelector::Exact(version) => self.find_record(FileKey {
                file: parsed.file,
                version,
            }),
        }?
        .filter(|record| !record.deleted)
        .ok_or(Error::NotFound)?;
        Ok(record)
    }

    fn next_version(&self, file: FileName) -> Result<u32, Error> {
        self.latest_record_at(self.root, file)?.map_or(Ok(1), |record| {
            record
                .key
                .version
                .checked_add(1)
                .ok_or(Error::VersionOverflow)
        })
    }

    fn allocate_object_id(&mut self) -> Result<u64, Error> {
        let object_id = self.next_object_id;
        self.next_object_id = self
            .next_object_id
            .checked_add(1)
            .ok_or(Error::VersionOverflow)?;
        Ok(object_id)
    }

    fn link_count_at(&self, root: BlockId, object_id: u64) -> Result<u32, Error> {
        if object_id == 0 {
            return Ok(1);
        }
        let mut count = 0_u32;
        let mut ordinal = 0;
        while let Some(record) = self.record_at(root, ordinal)? {
            ordinal = ordinal.saturating_add(1);
            if record.object_id == object_id
                && !record.deleted
                && self.latest_record_at(root, record.key.file)? == Some(record)
            {
                count = count.saturating_add(1);
            }
        }
        Ok(count)
    }

    fn require_parent_directory(&self, file: FileName) -> Result<(), Error> {
        let Some(separator) = file.as_bytes().iter().rposition(|byte| *byte == b'/') else {
            return Ok(());
        };
        if separator == 0 {
            return Ok(())
        }
        let parent = FileName::new(core::str::from_utf8(&file.as_bytes()[..separator]).map_err(|_| Error::InvalidPath)?)?;
        let record = self.lookup_following(parent.as_str())?;
        if record.file_type != FileType::Directory {
            return Err(Error::NotDirectory);
        }
        Ok(())
    }

    fn is_directory_path_at(&self, root: BlockId, path: &str) -> Result<bool, Error> {
        if path == "/" {
            return Ok(true)
        }
        let parsed = VersionedPath::parse(path)?;
        Ok(self
            .latest_record_at(root, parsed.file)?
            .is_some_and(|record| record.file_type == FileType::Directory))
    }

    fn tombstone_latest(&mut self, previous: FileRecord) -> Result<FileVersion, Error> {
        self.root = self.tombstone(self.root, previous.key)?;
        self.generation = self.generation.saturating_add(1);
        let mut deleted: FileVersion = previous.into();
        deleted.link_count = self.link_count_at(self.root, previous.object_id)?;
        Ok(deleted)
    }

    fn enforce_limits(&self, additional_bytes: u64, new_file: bool) -> Result<(), Error> {
        if self.arena.used() >= self.limits.max_blocks
            || self.arena.used() >= MAX_BLOCKS
        {
            return Err(Error::QuotaExceeded);
        }
        let diagnostics = self.diagnostics()?;
        if diagnostics.retained_bytes.saturating_add(additional_bytes) > self.limits.max_bytes
            || (new_file && diagnostics.file_count >= self.limits.max_files)
        {
            return Err(Error::QuotaExceeded);
        }
        Ok(())
    }

    /// Delete one live version. Older snapshots keep seeing their data.
    pub fn delete(&mut self, path: &str) -> Result<FileVersion, Error> {
        let result = self.delete_uncommitted(path);
        if result == Err(Error::OutOfSpace) {
            self.collect_garbage();
            return self.delete_uncommitted(path);
        }
        if result.is_err() {
            self.collect_garbage();
        }
        result
    }

    fn delete_uncommitted(&mut self, path: &str) -> Result<FileVersion, Error> {
        let parsed = VersionedPath::parse(path)?;
        let selected = match parsed.version {
            VersionSelector::Latest => self.latest_record_at(self.root, parsed.file),
            VersionSelector::Exact(version) => self.find_record(FileKey {
                file: parsed.file,
                version,
            }),
        }?
        .filter(|record| !record.deleted)
        .ok_or(Error::NotFound)?;
        if selected.file_type == FileType::Directory {
            return Err(Error::NotDirectory);
        }
        self.root = self.tombstone(self.root, selected.key)?;
        self.generation = self.generation.saturating_add(1);
        let mut deleted: FileVersion = selected.into();
        deleted.link_count = self.link_count_at(self.root, selected.object_id)?;
        Ok(deleted)
    }

    pub fn lookup(&self, path: &str) -> Result<FileVersion, Error> {
        self.lookup_at(self.root, path)
    }

    /// Look up a path after following symbolic links in any path component.
    pub fn lookup_following(&self, path: &str) -> Result<FileVersion, Error> {
        let length = path.len();
        if length == 0 || length > MAX_PATH_BYTES {
            return Err(Error::InvalidPath)
        }
        let mut current = [0; MAX_PATH_BYTES];
        current[..length].copy_from_slice(path.as_bytes());
        self.lookup_following_buffer(self.root, &current[..length])
    }

    fn lookup_following_buffer(&self, root: BlockId, path: &[u8]) -> Result<FileVersion, Error> {
        let mut current = [0; MAX_PATH_BYTES];
        current[..path.len()].copy_from_slice(path);
        let mut length = path.len();
        let mut next = [0; MAX_PATH_BYTES];
        for _ in 0..MAX_SYMLINK_DEPTH {
            let current_path = core::str::from_utf8(&current[..length]).map_err(|_| Error::InvalidPath)?;
            let mut component_end = 1;
            let mut symlink = None;
            while component_end <= length {
                if component_end == length || current[component_end] == b'/' {
                    if component_end > 1 {
                        let component = core::str::from_utf8(&current[..component_end])
                            .map_err(|_| Error::InvalidPath)?;
                        match self.lookup_at(root, component) {
                            Ok(file) if file.file_type == FileType::Symlink => {
                                symlink = Some((component_end, file));
                                break
                            }
                            Ok(_) | Err(Error::NotFound) => {}
                            Err(error) => return Err(error),
                        }
                    }
                    component_end += 1;
                } else {
                    component_end += 1;
                }
            }
            let Some((link_end, link)) = symlink else {
                return self.lookup_at(root, current_path)
            };
            let mut target = [0; MAX_PATH_BYTES];
            let target_len = usize::try_from(link.size).map_err(|_| Error::Corrupt)?;
            if target_len > target.len() {
                return Err(Error::Corrupt)
            }
            let record = self
                .find_record_at(root, FileKey {
                    file: link.file,
                    version: link.version,
                })?
                .ok_or(Error::Corrupt)?;
            self.read_record_range(record, 0, &mut target[..target_len])?;
            let parent_end = current[..link_end]
                .iter()
                .rposition(|byte| *byte == b'/')
                .unwrap_or(0);
            let target = core::str::from_utf8(&target[..target_len]).map_err(|_| Error::Corrupt)?;
            length = join_symlink_path(
                &current[..parent_end.max(1)],
                target.as_bytes(),
                &current[link_end..length],
                &mut next,
            )?;
            current[..length].copy_from_slice(&next[..length]);
            next.fill(0);
        }
        Err(Error::SymlinkLoop)
    }

    pub fn read_link(&self, path: &str, destination: &mut [u8]) -> Result<ReadResult, Error> {
        let file = self.lookup(path)?;
        if file.file_type != FileType::Symlink {
            return Err(Error::NotSymlink)
        }
        let record = self
            .find_record(FileKey {
                file: file.file,
                version: file.version,
            })?
            .ok_or(Error::Corrupt)?;
        let bytes_read = self.read_record_range(record, 0, destination)?;
        Ok(ReadResult { file, bytes_read })
    }

    /// Count links that are current directory entries, excluding versions
    /// visible only because a newer version was deleted.
    pub fn current_link_count(&self, path: &str) -> Result<u32, Error> {
        let selected = self.lookup_record(path)?;
        let mut count = 0_u32;
        let mut ordinal = 0;
        while let Some(record) = self.record_at(self.root, ordinal)? {
            ordinal = ordinal.saturating_add(1);
            if record.object_id == selected.object_id
                && !record.deleted
                && self.latest_record_at(self.root, record.key.file)? == Some(record)
                && self.last_record_at(self.root, record.key.file)? == Some(record)
            {
                count = count.saturating_add(1);
            }
        }
        Ok(count)
    }

    fn lookup_at(&self, root: BlockId, path: &str) -> Result<FileVersion, Error> {
        let path = VersionedPath::parse(path)?;
        let record = match path.version {
            VersionSelector::Latest => self.latest_record_at(root, path.file)?,
            VersionSelector::Exact(version) => self
                .find_record_at(
                    root,
                    FileKey {
                        file: path.file,
                        version,
                    },
                )?
                .filter(|record| !record.deleted),
        }
        .ok_or(Error::NotFound)?;
        let link_count = self.link_count_at(root, record.object_id)?;
        let mut version: FileVersion = record.into();
        version.link_count = link_count;
        Ok(version)
    }

    pub fn read(&self, path: &str, destination: &mut [u8]) -> Result<ReadResult, Error> {
        let file = self.lookup_following(path)?;
        self.read_file_version(file, destination)
    }

    /// Read a bounded range without requiring the caller to allocate a full
    /// file-sized buffer.
    pub fn read_at(
        &self,
        path: &str,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<ReadResult, Error> {
        let file = self.lookup_following(path)?;
        let bytes_read = self.read_record_range(
            self.find_record(FileKey {
                file: file.file,
                version: file.version,
            })?
            .ok_or(Error::Corrupt)?,
            offset,
            destination,
        )?;
        Ok(ReadResult { file, bytes_read })
    }

    pub fn read_version(
        &self,
        path: &str,
        version: u32,
        destination: &mut [u8],
    ) -> Result<ReadResult, Error> {
        if version == 0 {
            return Err(Error::InvalidVersion);
        }
        let file_name = FileName::new(path)?;
        let record = self
            .find_record(FileKey {
                file: file_name,
                version,
            })?
            .filter(|record| !record.deleted)
            .ok_or(Error::NotFound)?;
        let link_count = self.link_count_at(self.root, record.object_id)?;
        let mut file: FileVersion = record.into();
        file.link_count = link_count;
        self.read_file_version(file, destination)
    }

    pub fn retained_version_span(&self, path: &str) -> Result<(u32, Option<u32>), Error> {
        self.version_span(FileName::new(path)?)
    }

    fn read_file_version(
        &self,
        file: FileVersion,
        destination: &mut [u8],
    ) -> Result<ReadResult, Error> {
        if file.file_type == FileType::Directory {
            return Err(Error::NotDirectory);
        }
        let required = usize::try_from(file.size).map_err(|_| Error::Corrupt)?;
        if destination.len() < required {
            return Err(Error::BufferTooSmall { required });
        }

        let record = self
            .find_record(FileKey {
                file: file.file,
                version: file.version,
            })?
            .ok_or(Error::Corrupt)?;
        let mut id = record.data;
        let mut offset = 0;
        while id.is_some() {
            let Block::Data(block) = self.arena.get(id)? else {
                return Err(Error::Corrupt);
            };
            let length = block.len as usize;
            if length > DATA_BYTES
                || offset + length > required
                || length == 0
                || checksum(&block.bytes[..length]) != block.checksum
            {
                return Err(Error::Corrupt);
            }
            destination[offset..offset + length].copy_from_slice(&block.bytes[..length]);
            offset += length;
            id = block.next;
        }
        if offset != required || checksum(&destination[..offset]) != record.checksum {
            return Err(Error::Corrupt);
        }

        Ok(ReadResult {
            file,
            bytes_read: offset,
        })
    }

    /// Tombstones at most `limit` old versions and returns the number changed.
    pub fn purge(&mut self, path: &str, keep_latest: u32, limit: usize) -> Result<usize, Error> {
        if keep_latest == 0 {
            return Err(Error::InvalidVersion);
        }
        let file = FileName::new(path)?;
        let mut purged = 0;
        while purged < limit {
            let (count, oldest) = self.version_span(file)?;
            if count <= keep_latest {
                break;
            }
            let key = FileKey {
                file,
                version: oldest.ok_or(Error::Corrupt)?,
            };
            let root = self.tombstone(self.root, key)?;
            self.root = root;
            self.generation = self.generation.saturating_add(1);
            purged += 1;
        }
        Ok(purged)
    }

    /// Mark from the committed root, then reclaim abandoned CoW and data blocks.
    pub fn collect_garbage(&mut self) -> GcReport {
        let mut marked = [false; MAX_BLOCKS];
        let mut pending = [BlockId::NONE; MAX_BLOCKS];
        let mut pending_len = 0;
        if self.root.is_some() && MAX_BLOCKS != 0 {
            pending[0] = self.root;
            marked[self.root.0 as usize - 1] = true;
            pending_len = 1;
        }
        for checkpoint in self.checkpoints.iter().flatten() {
            if checkpoint.root.is_some() {
                mark_pending(checkpoint.root, &mut marked, &mut pending, &mut pending_len)
            }
        }

        while pending_len != 0 {
            pending_len -= 1;
            let id = pending[pending_len];
            let Ok(block) = self.arena.get(id) else {
                continue;
            };
            match block {
                Block::Data(data) if data.next.is_some() => {
                    mark_pending(data.next, &mut marked, &mut pending, &mut pending_len);
                }
                Block::Tree(TreeBlock::Leaf(leaf)) => {
                    for record in &leaf.records[..leaf.len as usize] {
                        if !record.deleted && record.data.is_some() {
                            mark_pending(record.data, &mut marked, &mut pending, &mut pending_len);
                        }
                    }
                }
                Block::Tree(TreeBlock::Branch(branch)) => {
                    for child in &branch.children[..=branch.len as usize] {
                        mark_pending(*child, &mut marked, &mut pending, &mut pending_len);
                    }
                }
                Block::Data(_) => {}
            }
        }

        let mut live_blocks = 0;
        let mut freed_blocks = 0;
        for (index, slot) in self.arena.slots.iter_mut().enumerate() {
            if slot.block.is_none() {
                continue;
            }
            if marked[index] {
                live_blocks += 1
            } else {
                slot.block = None;
                freed_blocks += 1
            }
        }
        GcReport {
            live_blocks,
            freed_blocks,
        }
    }

    fn mark_reachable(&self, root: BlockId, marked: &mut [bool; MAX_BLOCKS]) -> Result<(), Error> {
        if !root.is_some() {
            return Ok(());
        }
        let mut pending = [BlockId::NONE; MAX_BLOCKS];
        let mut pending_len = 0;
        mark_pending(root, marked, &mut pending, &mut pending_len);
        while pending_len != 0 {
            pending_len -= 1;
            let id = pending[pending_len];
            match self.arena.get(id)? {
                Block::Data(data) if data.next.is_some() => {
                    mark_pending(data.next, marked, &mut pending, &mut pending_len)
                }
                Block::Tree(TreeBlock::Leaf(leaf)) => {
                    for record in &leaf.records[..leaf.len as usize] {
                        if !record.deleted && record.data.is_some() {
                            mark_pending(record.data, marked, &mut pending, &mut pending_len)
                        }
                    }
                }
                Block::Tree(TreeBlock::Branch(branch)) => {
                    for child in &branch.children[..=branch.len as usize] {
                        mark_pending(*child, marked, &mut pending, &mut pending_len)
                    }
                }
                Block::Data(_) => {}
            }
        }
        Ok(())
    }

    fn store_data(&mut self, contents: &[u8]) -> Result<BlockId, Error> {
        let mut next = BlockId::NONE;
        for chunk in contents.rchunks(DATA_BYTES) {
            let mut bytes = [0; DATA_BYTES];
            bytes[..chunk.len()].copy_from_slice(chunk);
            next = self.arena.allocate(Block::Data(DataBlock {
                next,
                len: chunk.len() as u16,
                checksum: checksum(chunk),
                bytes,
            }))?;
        }
        Ok(next)
    }

    fn insert(&mut self, root: BlockId, record: FileRecord) -> Result<BlockId, Error> {
        if !root.is_some() {
            let mut leaf = Leaf::EMPTY;
            leaf.len = 1;
            leaf.records[0] = record;
            return self.arena.allocate(Block::Tree(TreeBlock::Leaf(leaf)));
        }

        let inserted = self.insert_node(root, record)?;
        let Some(split) = inserted.split else {
            return Ok(inserted.left);
        };
        let mut branch = Branch::EMPTY;
        branch.len = 1;
        branch.keys[0] = split.separator;
        branch.children[0] = inserted.left;
        branch.children[1] = split.right;
        self.arena.allocate(Block::Tree(TreeBlock::Branch(branch)))
    }

    fn replace(&mut self, id: BlockId, key: FileKey, record: FileRecord) -> Result<BlockId, Error> {
        match self.arena.get(id)? {
            Block::Tree(TreeBlock::Leaf(mut leaf)) => {
                let index = leaf.records[..leaf.len as usize]
                    .binary_search_by_key(&key, |item| item.key)
                    .map_err(|_| Error::NotFound)?;
                leaf.records[index] = record;
                self.arena.allocate(Block::Tree(TreeBlock::Leaf(leaf)))
            }
            Block::Tree(TreeBlock::Branch(mut branch)) => {
                let index = child_index(&branch, key);
                branch.children[index] = self.replace(branch.children[index], key, record)?;
                self.arena.allocate(Block::Tree(TreeBlock::Branch(branch)))
            }
            Block::Data(_) => Err(Error::Corrupt),
        }
    }

    fn insert_node(&mut self, id: BlockId, record: FileRecord) -> Result<Inserted, Error> {
        match self.arena.get(id)? {
            Block::Tree(TreeBlock::Leaf(leaf)) => self.insert_leaf(leaf, record),
            Block::Tree(TreeBlock::Branch(branch)) => self.insert_branch(branch, record),
            Block::Data(_) => Err(Error::Corrupt),
        }
    }

    fn insert_leaf(&mut self, leaf: Leaf, record: FileRecord) -> Result<Inserted, Error> {
        let len = leaf.len as usize;
        let position = leaf.records[..len]
            .binary_search_by_key(&record.key, |item| item.key)
            .unwrap_or_else(|position| position);
        if position < len && leaf.records[position].key == record.key {
            return Err(Error::AlreadyExists);
        }

        let mut records = [FileRecord::EMPTY; MAX_KEYS + 1];
        records[..position].copy_from_slice(&leaf.records[..position]);
        records[position] = record;
        records[position + 1..=len].copy_from_slice(&leaf.records[position..len]);
        if len < MAX_KEYS {
            let mut updated = Leaf::EMPTY;
            updated.len = (len + 1) as u8;
            updated.records[..=len].copy_from_slice(&records[..=len]);
            let left = self.arena.allocate(Block::Tree(TreeBlock::Leaf(updated)))?;
            return Ok(Inserted { left, split: None });
        }

        let middle = records.len() / 2;
        let mut left_leaf = Leaf::EMPTY;
        left_leaf.len = middle as u8;
        left_leaf.records[..middle].copy_from_slice(&records[..middle]);
        let mut right_leaf = Leaf::EMPTY;
        right_leaf.len = (records.len() - middle) as u8;
        right_leaf.records[..records.len() - middle].copy_from_slice(&records[middle..]);
        let left = self
            .arena
            .allocate(Block::Tree(TreeBlock::Leaf(left_leaf)))?;
        let right = self
            .arena
            .allocate(Block::Tree(TreeBlock::Leaf(right_leaf)))?;
        Ok(Inserted {
            left,
            split: Some(Split {
                separator: right_leaf.records[0].key,
                right,
            }),
        })
    }

    fn insert_branch(&mut self, branch: Branch, record: FileRecord) -> Result<Inserted, Error> {
        let len = branch.len as usize;
        let child_index = child_index(&branch, record.key);
        let inserted = self.insert_node(branch.children[child_index], record)?;
        let mut keys = [FileRecord::EMPTY.key; MAX_KEYS + 1];
        let mut children = [BlockId::NONE; MAX_KEYS + 2];

        if let Some(split) = inserted.split {
            keys[..child_index].copy_from_slice(&branch.keys[..child_index]);
            keys[child_index] = split.separator;
            keys[child_index + 1..=len].copy_from_slice(&branch.keys[child_index..len]);
            children[..child_index].copy_from_slice(&branch.children[..child_index]);
            children[child_index] = inserted.left;
            children[child_index + 1] = split.right;
            children[child_index + 2..=len + 1]
                .copy_from_slice(&branch.children[child_index + 1..=len]);
        } else {
            keys[..len].copy_from_slice(&branch.keys[..len]);
            children[..=len].copy_from_slice(&branch.children[..=len]);
            children[child_index] = inserted.left;
        }

        let new_len = len + usize::from(inserted.split.is_some());
        if new_len <= MAX_KEYS {
            let mut updated = Branch::EMPTY;
            updated.len = new_len as u8;
            updated.keys[..new_len].copy_from_slice(&keys[..new_len]);
            updated.children[..=new_len].copy_from_slice(&children[..=new_len]);
            let left = self
                .arena
                .allocate(Block::Tree(TreeBlock::Branch(updated)))?;
            return Ok(Inserted { left, split: None });
        }

        let middle = new_len / 2;
        let separator = keys[middle];
        let mut left_branch = Branch::EMPTY;
        left_branch.len = middle as u8;
        left_branch.keys[..middle].copy_from_slice(&keys[..middle]);
        left_branch.children[..=middle].copy_from_slice(&children[..=middle]);
        let right_len = new_len - middle - 1;
        let mut right_branch = Branch::EMPTY;
        right_branch.len = right_len as u8;
        right_branch.keys[..right_len].copy_from_slice(&keys[middle + 1..new_len]);
        right_branch.children[..=right_len].copy_from_slice(&children[middle + 1..=new_len]);
        let left = self
            .arena
            .allocate(Block::Tree(TreeBlock::Branch(left_branch)))?;
        let right = self
            .arena
            .allocate(Block::Tree(TreeBlock::Branch(right_branch)))?;
        Ok(Inserted {
            left,
            split: Some(Split { separator, right }),
        })
    }

    fn find_record(&self, key: FileKey) -> Result<Option<FileRecord>, Error> {
        self.find_record_at(self.root, key)
    }

    fn find_record_at(&self, root: BlockId, key: FileKey) -> Result<Option<FileRecord>, Error> {
        if !root.is_some() {
            return Ok(None);
        }
        let mut id = root;
        loop {
            match self.arena.get(id)? {
                Block::Tree(TreeBlock::Leaf(leaf)) => {
                    return Ok(leaf.records[..leaf.len as usize]
                        .binary_search_by_key(&key, |record| record.key)
                        .ok()
                        .map(|index| leaf.records[index]));
                }
                Block::Tree(TreeBlock::Branch(branch)) => {
                    id = branch.children[child_index(&branch, key)]
                }
                Block::Data(_) => return Err(Error::Corrupt),
            }
        }
    }

    fn last_record(&self, file: FileName) -> Result<Option<FileRecord>, Error> {
        self.last_record_at(self.root, file)
    }

    fn latest_record_at(&self, root: BlockId, file: FileName) -> Result<Option<FileRecord>, Error> {
        let mut ordinal = 0;
        let mut latest = None;
        while let Some(record) = self.record_at(root, ordinal)? {
            ordinal = ordinal.saturating_add(1);
            if record.key.file == file
                && !record.deleted
                && latest.map_or(true, |current: FileRecord| {
                    current.key.version < record.key.version
                })
            {
                latest = Some(record);
            }
        }
        Ok(latest)
    }

    fn last_record_at(&self, root: BlockId, file: FileName) -> Result<Option<FileRecord>, Error> {
        if !root.is_some() {
            return Ok(None);
        }
        let key = FileKey {
            file,
            version: u32::MAX,
        };
        let mut id = root;
        loop {
            match self.arena.get(id)? {
                Block::Tree(TreeBlock::Leaf(leaf)) => {
                    return Ok(leaf.records[..leaf.len as usize]
                        .iter()
                        .rev()
                        .find(|record| record.key.file == file)
                        .copied());
                }
                Block::Tree(TreeBlock::Branch(branch)) => {
                    id = branch.children[child_index(&branch, key)]
                }
                Block::Data(_) => return Err(Error::Corrupt),
            }
        }
    }

    fn version_span(&self, file: FileName) -> Result<(u32, Option<u32>), Error> {
        fn visit<const MAX_BLOCKS: usize>(
            fs: &SynFs<MAX_BLOCKS>,
            id: BlockId,
            file: FileName,
            count: &mut u32,
            oldest: &mut Option<u32>,
        ) -> Result<(), Error> {
            match fs.arena.get(id)? {
                Block::Tree(TreeBlock::Leaf(leaf)) => {
                    for record in &leaf.records[..leaf.len as usize] {
                        if record.key.file == file && !record.deleted {
                            *count = count.saturating_add(1);
                            *oldest = Some(oldest.map_or(record.key.version, |current| {
                                current.min(record.key.version)
                            }));
                        }
                    }
                }
                Block::Tree(TreeBlock::Branch(branch)) => {
                    for child in &branch.children[..=branch.len as usize] {
                        visit(fs, *child, file, count, oldest)?;
                    }
                }
                Block::Data(_) => return Err(Error::Corrupt),
            }
            Ok(())
        }

        if !self.root.is_some() {
            return Ok((0, None));
        }
        let mut count = 0;
        let mut oldest = None;
        visit(self, self.root, file, &mut count, &mut oldest)?;
        Ok((count, oldest))
    }

    fn find_checkpoint(&self, id: CheckpointId) -> Result<Checkpoint, Error> {
        self.checkpoints
            .iter()
            .flatten()
            .find(|checkpoint| checkpoint.info.id == id)
            .copied()
            .ok_or(Error::CheckpointNotFound)
    }

    fn record_at(&self, root: BlockId, wanted: u32) -> Result<Option<FileRecord>, Error> {
        fn visit<const MAX_BLOCKS: usize>(
            fs: &SynFs<MAX_BLOCKS>,
            id: BlockId,
            wanted: u32,
            ordinal: &mut u32,
        ) -> Result<Option<FileRecord>, Error> {
            match fs.arena.get(id)? {
                Block::Tree(TreeBlock::Leaf(leaf)) => {
                    for record in &leaf.records[..leaf.len as usize] {
                        if record.deleted {
                            continue;
                        }
                        if *ordinal == wanted {
                            return Ok(Some(*record));
                        }
                        *ordinal = ordinal.saturating_add(1)
                    }
                    Ok(None)
                }
                Block::Tree(TreeBlock::Branch(branch)) => {
                    for child in &branch.children[..=branch.len as usize] {
                        if let Some(record) = visit(fs, *child, wanted, ordinal)? {
                            return Ok(Some(record));
                        }
                    }
                    Ok(None)
                }
                Block::Data(_) => Err(Error::Corrupt),
            }
        }

        if !root.is_some() {
            return Ok(None);
        }
        visit(self, root, wanted, &mut 0)
    }

    fn read_record_range(
        &self,
        record: FileRecord,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, Error> {
        if record.file_type == FileType::Directory {
            return Err(Error::NotDirectory);
        }
        if offset > record.size {
            return Err(Error::InvalidVersion);
        }
        let available = record.size - offset;
        let wanted = destination
            .len()
            .min(
                usize::try_from(available).map_err(|_| Error::BufferTooSmall {
                    required: usize::MAX,
                })?,
            );
        let mut id = record.data;
        let mut block_start = 0_u64;
        let mut copied = 0;
        while id.is_some() && copied < wanted {
            let Block::Data(block) = self.arena.get(id)? else {
                return Err(Error::Corrupt);
            };
            let length = block.len as usize;
            if length == 0 || length > DATA_BYTES || checksum(&block.bytes[..length]) != block.checksum {
                return Err(Error::Corrupt);
            }
            let block_end = block_start.saturating_add(length as u64);
            if offset < block_end {
                let start = usize::try_from(offset.saturating_sub(block_start))
                    .map_err(|_| Error::Corrupt)?;
                let amount = (length - start).min(wanted - copied);
                destination[copied..copied + amount]
                    .copy_from_slice(&block.bytes[start..start + amount]);
                copied += amount
            }
            block_start = block_end;
            id = block.next
        }
        if copied != wanted {
            return Err(Error::Corrupt);
        }
        Ok(copied)
    }

    fn tombstone(&mut self, id: BlockId, key: FileKey) -> Result<BlockId, Error> {
        match self.arena.get(id)? {
            Block::Tree(TreeBlock::Leaf(mut leaf)) => {
                let index = leaf.records[..leaf.len as usize]
                    .binary_search_by_key(&key, |record| record.key)
                    .map_err(|_| Error::NotFound)?;
                leaf.records[index].deleted = true;
                leaf.records[index].size = 0;
                leaf.records[index].data = BlockId::NONE;
                leaf.records[index].checksum = checksum(&[]);
                self.arena.allocate(Block::Tree(TreeBlock::Leaf(leaf)))
            }
            Block::Tree(TreeBlock::Branch(mut branch)) => {
                let index = child_index(&branch, key);
                branch.children[index] = self.tombstone(branch.children[index], key)?;
                self.arena.allocate(Block::Tree(TreeBlock::Branch(branch)))
            }
            Block::Data(_) => Err(Error::Corrupt),
        }
    }
}

impl<const MAX_BLOCKS: usize> Default for SynFs<MAX_BLOCKS> {
    fn default() -> Self {
        Self::new()
    }
}

impl From<FileRecord> for FileVersion {
    fn from(record: FileRecord) -> Self {
        Self {
            file: record.key.file,
            version: record.key.version,
            size: record.size,
            checksum: record.checksum,
            created_at: record.created_at,
            file_type: record.file_type,
            link_count: record.link_count,
            mode: record.mode,
        }
    }
}

fn child_index(branch: &Branch, key: FileKey) -> usize {
    branch.keys[..branch.len as usize].partition_point(|separator| key >= *separator)
}

fn join_symlink_path(
    parent: &[u8],
    target: &[u8],
    suffix: &[u8],
    output: &mut [u8; MAX_PATH_BYTES],
) -> Result<usize, Error> {
    let mut length = 1;
    output[0] = b'/';
    if !target.starts_with(b"/") {
        append_path_components(parent, output, &mut length)?;
    }
    append_path_components(target, output, &mut length)?;
    append_path_components(suffix, output, &mut length)?;
    Ok(length)
}

fn append_path_components(
    source: &[u8],
    output: &mut [u8; MAX_PATH_BYTES],
    length: &mut usize,
) -> Result<(), Error> {
    for component in source.split(|byte| *byte == b'/') {
        if component.is_empty() || component == b"." {
            continue
        }
        if component == b".." {
            if *length > 1 {
                *length = output[..*length - 1]
                    .iter()
                    .rposition(|byte| *byte == b'/')
                    .map_or(1, |position| position + 1);
            }
            continue
        }
        let required = component
            .len()
            .checked_add(usize::from(*length > 1))
            .and_then(|size| (*length).checked_add(size))
            .ok_or(Error::InvalidPath)?;
        if required > output.len() || core::str::from_utf8(component).is_err() {
            return Err(Error::InvalidPath)
        }
        if *length > 1 {
            output[*length] = b'/';
            *length += 1;
        }
        output[*length..*length + component.len()].copy_from_slice(component);
        *length += component.len();
    }
    Ok(())
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn mark_pending<const MAX_BLOCKS: usize>(
    id: BlockId,
    marked: &mut [bool; MAX_BLOCKS],
    pending: &mut [BlockId; MAX_BLOCKS],
    pending_len: &mut usize,
) {
    let Some(index) = id.0.checked_sub(1).map(|index| index as usize) else {
        return;
    };
    let Some(is_marked) = marked.get_mut(index) else {
        return;
    };
    if *is_marked {
        return;
    }
    *is_marked = true;
    pending[*pending_len] = id;
    *pending_len += 1;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionRule {
    pub file: FileName,
    pub keep_latest: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PurgeReport {
    pub versions_purged: usize,
    pub garbage: GcReport,
}

/// Cooperative background retention worker intended for the `ghostfs_purged`
/// user-space service. Each poll performs bounded version work.
pub struct SynfsPurged {
    rules: [Option<RetentionRule>; MAX_RETENTION_RULES],
    cursor: usize,
}

impl SynfsPurged {
    pub const fn new() -> Self {
        Self {
            rules: [None; MAX_RETENTION_RULES],
            cursor: 0,
        }
    }

    pub fn add_rule(&mut self, path: &str, keep_latest: u32) -> Result<(), Error> {
        if keep_latest == 0 {
            return Err(Error::InvalidVersion);
        }
        let slot = self
            .rules
            .iter_mut()
            .find(|rule| rule.is_none())
            .ok_or(Error::TooManyRetentionRules)?;
        *slot = Some(RetentionRule {
            file: FileName::new(path)?,
            keep_latest,
        });
        Ok(())
    }

    pub fn poll<const MAX_BLOCKS: usize>(
        &mut self,
        fs: &mut SynFs<MAX_BLOCKS>,
        version_budget: usize,
    ) -> Result<PurgeReport, Error> {
        let mut versions_purged = 0;
        let mut visited = 0;
        while versions_purged < version_budget && visited < MAX_RETENTION_RULES {
            let index = self.cursor;
            self.cursor = (self.cursor + 1) % MAX_RETENTION_RULES;
            visited += 1;
            let Some(rule) = self.rules[index] else {
                continue;
            };
            versions_purged += fs.purge(rule.file.as_str(), rule.keep_latest, 1)?;
        }
        Ok(PurgeReport {
            versions_purged,
            garbage: fs.collect_garbage(),
        })
    }
}

impl Default for SynfsPurged {
    fn default() -> Self {
        Self::new()
    }
}
