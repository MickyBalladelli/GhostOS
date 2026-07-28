#![no_std]
#![forbid(unsafe_code)]

use core::cmp::Ordering;
use core::fmt;
use synos_status::{IntoStatus, Severity, Status, facility};

mod rms;

pub use rms::{
    IndexDefinition, RecordDescriptor, RecordFileInfo, RecordFormat, RecordOrganization,
    RecordRead, RecordSelector, RmsError,
};

pub const BLOCK_SIZE: usize = 4096;
pub const DATA_BYTES: usize = BLOCK_SIZE - 16;
pub const MAX_PATH_BYTES: usize = 192;
pub const MAX_KEYS: usize = 7;
pub const MAX_RETENTION_RULES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyExists,
    BufferTooSmall { required: usize },
    Corrupt,
    InvalidPath,
    InvalidVersion,
    NotFound,
    OutOfSpace,
    TooManyRetentionRules,
    VersionOverflow,
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::NotFound => Status::NOT_FOUND,
            Self::OutOfSpace | Self::TooManyRetentionRules => Status::NO_SPACE,
            Self::Corrupt => Status::CORRUPT,
            Self::InvalidPath | Self::InvalidVersion => Status::INVALID_ARGUMENT,
            Self::BufferTooSmall { .. } => {
                Status::new(Severity::Error, facility::FILESYSTEM, 1, 0)
                    .expect("valid filesystem status")
            }
            Self::AlreadyExists => {
                Status::new(Severity::Error, facility::FILESYSTEM, 2, 0)
                    .expect("valid filesystem status")
            }
            Self::VersionOverflow => {
                Status::new(Severity::Fatal, facility::FILESYSTEM, 3, 0)
                    .expect("valid filesystem status")
            }
        }
    }
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
            || value.ends_with(b"/")
            || value.windows(2).any(|pair| pair == b"//")
        {
            return Err(Error::InvalidPath)
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
        core::str::from_utf8(self.as_bytes()).expect("FileName invariant")
    }
}

impl fmt::Debug for FileName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("FileName").field(&self.as_str()).finish()
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
            })
        };

        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Error::InvalidVersion)
        }
        let version = suffix
            .parse::<u32>()
            .map_err(|_| Error::InvalidVersion)?;
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
    size: u64,
    data: BlockId,
    checksum: u64,
    created_at: u64,
    deleted: bool,
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
        size: 0,
        data: BlockId::NONE,
        checksum: 0,
        created_at: 0,
        deleted: false,
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
        if !id.is_some() {
            return Err(Error::Corrupt)
        }
        self.slots
            .get(id.0 as usize - 1)
            .and_then(|slot| slot.block)
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

    fn allocate_shared_data(&mut self, block: DataBlock) -> Result<BlockId, Error> {
        if let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.block == Some(Block::Data(block)))
        {
            return Ok(BlockId(index as u32 + 1))
        }
        self.allocate(Block::Data(block))
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadResult {
    pub file: FileVersion,
    pub bytes_read: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GcReport {
    pub live_blocks: usize,
    pub freed_blocks: usize,
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

/// Fixed-capacity SynFS metadata and data block store.
///
/// Every tree update writes a new path from the changed leaf to the root.
/// Existing roots remain valid until commit, while unchanged data blocks are
/// shared by identity. This keeps file versions immutable without a kernel heap.
pub struct SynFs<const MAX_BLOCKS: usize> {
    arena: BlockArena<MAX_BLOCKS>,
    root: BlockId,
    generation: u64,
}

impl<const MAX_BLOCKS: usize> SynFs<MAX_BLOCKS> {
    pub const fn new() -> Self {
        Self {
            arena: BlockArena::new(),
            root: BlockId::NONE,
            generation: 0,
        }
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn used_blocks(&self) -> usize {
        self.arena.used()
    }

    pub const fn capacity(&self) -> usize {
        MAX_BLOCKS
    }

    pub fn write(&mut self, path: &str, contents: &[u8]) -> Result<FileVersion, Error> {
        let parsed = VersionedPath::parse(path)?;
        if parsed.version != VersionSelector::Latest || path.contains(';') {
            return Err(Error::InvalidVersion)
        }

        let version = self
            .latest_record(parsed.file)?
            .map_or(Ok(1), |record| record.key.version.checked_add(1).ok_or(Error::VersionOverflow))?;
        let created_at = self.generation.saturating_add(1);
        let mut result = self.write_inner(parsed.file, version, created_at, contents);
        if result == Err(Error::OutOfSpace) {
            self.collect_garbage();
            result = self.write_inner(parsed.file, version, created_at, contents);
        }
        if result.is_err() {
            self.collect_garbage();
        }
        result
    }

    fn write_inner(
        &mut self,
        file: FileName,
        version: u32,
        created_at: u64,
        contents: &[u8],
    ) -> Result<FileVersion, Error> {
        let data = self.store_data(contents)?;
        let record = FileRecord {
            key: FileKey { file, version },
            size: contents.len() as u64,
            data,
            checksum: checksum(contents),
            created_at,
            deleted: false,
        };
        let root = self.insert(self.root, record)?;
        self.root = root;
        self.generation = created_at;
        Ok(record.into())
    }

    pub fn lookup(&self, path: &str) -> Result<FileVersion, Error> {
        let path = VersionedPath::parse(path)?;
        let record = match path.version {
            VersionSelector::Latest => self.latest_record(path.file)?,
            VersionSelector::Exact(version) => {
                self.find_record(FileKey {
                    file: path.file,
                    version,
                })?
                .filter(|record| !record.deleted)
            }
        }
        .ok_or(Error::NotFound)?;
        Ok(record.into())
    }

    pub fn read(&self, path: &str, destination: &mut [u8]) -> Result<ReadResult, Error> {
        let file = self.lookup(path)?;
        let required = usize::try_from(file.size).map_err(|_| Error::Corrupt)?;
        if destination.len() < required {
            return Err(Error::BufferTooSmall { required })
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
                return Err(Error::Corrupt)
            };
            let length = block.len as usize;
            if length > DATA_BYTES
                || offset + length > required
                || checksum(&block.bytes[..length]) != block.checksum
            {
                return Err(Error::Corrupt)
            }
            destination[offset..offset + length].copy_from_slice(&block.bytes[..length]);
            offset += length;
            id = block.next;
        }
        if offset != required || checksum(&destination[..offset]) != record.checksum {
            return Err(Error::Corrupt)
        }

        Ok(ReadResult {
            file,
            bytes_read: offset,
        })
    }

    /// Tombstones at most `limit` old versions and returns the number changed.
    pub fn purge(&mut self, path: &str, keep_latest: u32, limit: usize) -> Result<usize, Error> {
        if keep_latest == 0 {
            return Err(Error::InvalidVersion)
        }
        let file = FileName::new(path)?;
        let mut purged = 0;
        while purged < limit {
            let (count, oldest) = self.version_span(file)?;
            if count <= keep_latest {
                break
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

        while pending_len != 0 {
            pending_len -= 1;
            let id = pending[pending_len];
            let Ok(block) = self.arena.get(id) else {
                continue
            };
            match block {
                Block::Data(data) if data.next.is_some() => {
                    mark_pending(data.next, &mut marked, &mut pending, &mut pending_len);
                }
                Block::Tree(TreeBlock::Leaf(leaf)) => {
                    for record in &leaf.records[..leaf.len as usize] {
                        if !record.deleted && record.data.is_some() {
                            mark_pending(
                                record.data,
                                &mut marked,
                                &mut pending,
                                &mut pending_len,
                            );
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
                continue
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

    fn store_data(&mut self, contents: &[u8]) -> Result<BlockId, Error> {
        let mut next = BlockId::NONE;
        for chunk in contents.rchunks(DATA_BYTES) {
            let mut bytes = [0; DATA_BYTES];
            bytes[..chunk.len()].copy_from_slice(chunk);
            next = self.arena.allocate_shared_data(DataBlock {
                next,
                len: chunk.len() as u16,
                checksum: checksum(chunk),
                bytes,
            })?;
        }
        Ok(next)
    }

    fn insert(&mut self, root: BlockId, record: FileRecord) -> Result<BlockId, Error> {
        if !root.is_some() {
            let mut leaf = Leaf::EMPTY;
            leaf.len = 1;
            leaf.records[0] = record;
            return self.arena.allocate(Block::Tree(TreeBlock::Leaf(leaf)))
        }

        let inserted = self.insert_node(root, record)?;
        let Some(split) = inserted.split else {
            return Ok(inserted.left)
        };
        let mut branch = Branch::EMPTY;
        branch.len = 1;
        branch.keys[0] = split.separator;
        branch.children[0] = inserted.left;
        branch.children[1] = split.right;
        self.arena
            .allocate(Block::Tree(TreeBlock::Branch(branch)))
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
            return Err(Error::AlreadyExists)
        }

        let mut records = [FileRecord::EMPTY; MAX_KEYS + 1];
        records[..position].copy_from_slice(&leaf.records[..position]);
        records[position] = record;
        records[position + 1..=len].copy_from_slice(&leaf.records[position..len]);
        if len < MAX_KEYS {
            let mut updated = Leaf::EMPTY;
            updated.len = (len + 1) as u8;
            updated.records[..=len].copy_from_slice(&records[..=len]);
            let left = self
                .arena
                .allocate(Block::Tree(TreeBlock::Leaf(updated)))?;
            return Ok(Inserted { left, split: None })
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
            return Ok(Inserted { left, split: None })
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
        right_branch.children[..=right_len]
            .copy_from_slice(&children[middle + 1..=new_len]);
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
        if !self.root.is_some() {
            return Ok(None)
        }
        let mut id = self.root;
        loop {
            match self.arena.get(id)? {
                Block::Tree(TreeBlock::Leaf(leaf)) => {
                    return Ok(leaf.records[..leaf.len as usize]
                        .binary_search_by_key(&key, |record| record.key)
                        .ok()
                        .map(|index| leaf.records[index]))
                }
                Block::Tree(TreeBlock::Branch(branch)) => {
                    id = branch.children[child_index(&branch, key)]
                }
                Block::Data(_) => return Err(Error::Corrupt),
            }
        }
    }

    fn latest_record(&self, file: FileName) -> Result<Option<FileRecord>, Error> {
        if !self.root.is_some() {
            return Ok(None)
        }
        let key = FileKey {
            file,
            version: u32::MAX,
        };
        let mut id = self.root;
        loop {
            match self.arena.get(id)? {
                Block::Tree(TreeBlock::Leaf(leaf)) => {
                    return Ok(leaf.records[..leaf.len as usize]
                        .iter()
                        .rev()
                        .find(|record| record.key.file == file && !record.deleted)
                        .copied())
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
            return Ok((0, None))
        }
        let mut count = 0;
        let mut oldest = None;
        visit(self, self.root, file, &mut count, &mut oldest)?;
        Ok((count, oldest))
    }

    fn tombstone(&mut self, id: BlockId, key: FileKey) -> Result<BlockId, Error> {
        match self.arena.get(id)? {
            Block::Tree(TreeBlock::Leaf(mut leaf)) => {
                let index = leaf.records[..leaf.len as usize]
                    .binary_search_by_key(&key, |record| record.key)
                    .map_err(|_| Error::NotFound)?;
                leaf.records[index].deleted = true;
                self.arena.allocate(Block::Tree(TreeBlock::Leaf(leaf)))
            }
            Block::Tree(TreeBlock::Branch(mut branch)) => {
                let index = child_index(&branch, key);
                branch.children[index] = self.tombstone(branch.children[index], key)?;
                self.arena
                    .allocate(Block::Tree(TreeBlock::Branch(branch)))
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
        }
    }
}

fn child_index(branch: &Branch, key: FileKey) -> usize {
    branch.keys[..branch.len as usize].partition_point(|separator| key >= *separator)
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
        return
    };
    let Some(is_marked) = marked.get_mut(index) else {
        return
    };
    if *is_marked {
        return
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

/// Cooperative background retention worker intended for the `synfs_purged`
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
            return Err(Error::InvalidVersion)
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
                continue
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
