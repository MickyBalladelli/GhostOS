use super::*;
use crate::block::BlockStore;
use ghostos_durability::{CrashBoundary, CrashDomain, InterruptionInjector, NoInterruption};

const SUPERBLOCK_MAGIC: &[u8; 8] = b"SYNFSVOL";
const TYPE_MAP_MAGIC: &[u8; 8] = b"SYNFSMAP";
const TREE_MAGIC: &[u8; 4] = b"SYNT";
const TREE_FORMAT_VERSION: u8 = 1;
const SUPERBLOCK_CHECKSUM_OFFSET: usize = BLOCK_SIZE - 8;
const SUPERBLOCK_HEADER_BYTES: u16 = 512;
const TYPE_MAP_EMPTY: u8 = 0;
const TYPE_MAP_TREE: u8 = 1;
const TYPE_MAP_BRANCH: u8 = 2;
const TYPE_MAP_DATA: u8 = 3;

pub const VOLUME_FORMAT_VERSION: u16 = 4;
pub(crate) const PREVIOUS_VOLUME_FORMAT_VERSION: u16 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VolumeGeometry {
    pub block_size: usize,
    pub blocks_per_generation: usize,
    pub total_blocks: usize,
    pub total_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VolumeCommit {
    pub sequence: u64,
    pub generation: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct Superblock {
    pub(crate) format_version: u16,
    pub(crate) sequence: u64,
    pub(crate) generation: u64,
    pub(crate) root: BlockId,
    pub(crate) next_checkpoint: u64,
    pub(crate) checkpoints: [Option<Checkpoint>; MAX_CHECKPOINTS],
    pub(crate) type_map_checksum: u64,
    pub(crate) limits: VolumeLimits,
    pub(crate) next_object_id: u64,
}

struct Encoder<'a> {
    bytes: &'a mut [u8],
    position: usize,
}

impl<'a> Encoder<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn put_u8(&mut self, value: u8) -> Result<(), Error> {
        let slot = self.bytes.get_mut(self.position).ok_or(Error::Corrupt)?;
        *slot = value;
        self.position += 1;
        Ok(())
    }

    fn put_u16(&mut self, value: u16) -> Result<(), Error> {
        self.put_bytes(&value.to_le_bytes())
    }

    fn put_u32(&mut self, value: u32) -> Result<(), Error> {
        self.put_bytes(&value.to_le_bytes())
    }

    fn put_u64(&mut self, value: u64) -> Result<(), Error> {
        self.put_bytes(&value.to_le_bytes())
    }

    fn put_bytes(&mut self, value: &[u8]) -> Result<(), Error> {
        let end = self
            .position
            .checked_add(value.len())
            .ok_or(Error::Corrupt)?;
        let destination = self
            .bytes
            .get_mut(self.position..end)
            .ok_or(Error::Corrupt)?;
        destination.copy_from_slice(value);
        self.position = end;
        Ok(())
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn get_u8(&mut self) -> Result<u8, Error> {
        let value = *self.bytes.get(self.position).ok_or(Error::Corrupt)?;
        self.position += 1;
        Ok(value)
    }

    fn get_u16(&mut self) -> Result<u16, Error> {
        let mut bytes = [0; 2];
        bytes.copy_from_slice(self.get_bytes(2)?);
        Ok(u16::from_le_bytes(bytes))
    }

    fn get_u32(&mut self) -> Result<u32, Error> {
        let mut bytes = [0; 4];
        bytes.copy_from_slice(self.get_bytes(4)?);
        Ok(u32::from_le_bytes(bytes))
    }

    fn get_u64(&mut self) -> Result<u64, Error> {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(self.get_bytes(8)?);
        Ok(u64::from_le_bytes(bytes))
    }

    fn get_bytes(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let end = self.position.checked_add(length).ok_or(Error::Corrupt)?;
        let value = self.bytes.get(self.position..end).ok_or(Error::Corrupt)?;
        self.position = end;
        Ok(value)
    }
}

impl<const MAX_BLOCKS: usize> SynFs<MAX_BLOCKS> {
    pub const fn volume_bytes() -> usize {
        Self::volume_blocks().saturating_mul(BLOCK_SIZE)
    }

    pub const fn volume_blocks() -> usize {
        2usize.saturating_mul(MAX_BLOCKS.saturating_add(2))
    }

    pub const fn volume_geometry() -> VolumeGeometry {
        VolumeGeometry {
            block_size: BLOCK_SIZE,
            blocks_per_generation: MAX_BLOCKS.saturating_add(2),
            total_blocks: Self::volume_blocks(),
            total_bytes: Self::volume_bytes(),
        }
    }

    pub fn format(image: &mut [u8]) -> Result<(), Error> {
        require_image_size::<MAX_BLOCKS>(image)?;
        image.fill(0);
        let empty = Superblock {
            format_version: VOLUME_FORMAT_VERSION,
            sequence: 1,
            generation: 0,
            root: BlockId::NONE,
            next_checkpoint: 1,
            checkpoints: [None; MAX_CHECKPOINTS],
            type_map_checksum: checksum(&type_map_bytes::<MAX_BLOCKS>()),
            limits: VolumeLimits::UNLIMITED,
            next_object_id: 1,
        };
        let map = type_map_bytes::<MAX_BLOCKS>();
        write_type_map::<MAX_BLOCKS>(image, 0, &map)?;
        write_superblock::<MAX_BLOCKS>(image, 0, empty)
    }

    pub fn format_volume(image: &mut [u8]) -> Result<(), Error> {
        Self::format(image)
    }

    /// Format a volume image and persist it through a block provider.
    pub fn format_to_device<D: BlockStore>(image: &mut [u8], device: &mut D) -> Result<(), Error> {
        Self::format(image)?;
        device
            .reserve(Self::volume_blocks() as u64)
            .map_err(|_| Error::Io)?;
        let result = (|| {
            for block in 0..Self::volume_blocks() {
                let start = block * BLOCK_SIZE;
                device
                    .write_block(block as u64, &image[start..start + BLOCK_SIZE])
                    .map_err(|_| Error::Io)?;
            }
            device.flush().map_err(|_| Error::Io)
        })();
        if result.is_err() {
            let _ = device.release(Self::volume_blocks() as u64);
        }
        result
    }

    pub fn load(image: &[u8]) -> Result<Self, Error> {
        require_image_size::<MAX_BLOCKS>(image)?;
        load_committed_generation::<MAX_BLOCKS>(image)
    }

    pub fn load_volume(image: &[u8]) -> Result<Self, Error> {
        Self::load(image)
    }

    /// Read a complete persistent volume into the caller's staging image.
    pub fn load_from_device<D: BlockStore>(
        image: &mut [u8],
        device: &mut D,
    ) -> Result<Self, Error> {
        require_image_size::<MAX_BLOCKS>(image)?;
        device
            .reserve(Self::volume_blocks() as u64)
            .map_err(|_| Error::Io)?;
        let result = (|| {
            for block in 0..Self::volume_blocks() {
                let start = block * BLOCK_SIZE;
                device
                    .read_block(block as u64, &mut image[start..start + BLOCK_SIZE])
                    .map_err(|_| Error::Io)?;
            }
            Self::load(image)
        })();
        if result.is_err() {
            let _ = device.release(Self::volume_blocks() as u64);
        }
        result
    }

    pub fn recover(image: &[u8]) -> Result<Self, Error> {
        require_image_size::<MAX_BLOCKS>(image)?;
        load_committed_generation::<MAX_BLOCKS>(image)
    }

    pub fn recover_volume(image: &[u8]) -> Result<Self, Error> {
        Self::recover(image)
    }

    /// Serialize the next generation into a caller-owned image. The image is
    /// only published in memory; use `sync` or `flush_to_device` for a
    /// power-loss durability guarantee.
    pub fn flush(&mut self, image: &mut [u8]) -> Result<VolumeCommit, Error> {
        let mut no_interruption = NoInterruption;
        self.flush_with_interruption(image, &mut no_interruption)
    }

    pub fn flush_with_interruption<I: InterruptionInjector>(
        &mut self,
        image: &mut [u8],
        injector: &mut I,
    ) -> Result<VolumeCommit, Error> {
        require_image_size::<MAX_BLOCKS>(image)?;
        if MAX_BLOCKS > (BLOCK_SIZE - 8) * 4 {
            return Err(Error::Corrupt);
        }
        self.check_consistency()?;
        let next_sequence = self
            .volume_sequence
            .checked_add(1)
            .ok_or(Error::VersionOverflow)?;
        let bank = 1 - self.volume_bank;
        let map = bank_type_map::<MAX_BLOCKS>(&self.arena);
        write_type_map::<MAX_BLOCKS>(image, bank, &map)?;
        for (index, slot) in self.arena.slots.iter().enumerate() {
            let destination = block_slice_mut::<MAX_BLOCKS>(image, bank, index)?;
            match slot.block {
                None => destination.fill(0),
                Some(Block::Tree(tree)) => encode_tree(destination, tree)?,
                Some(Block::Data(data)) => encode_data(destination, data),
            }
        }
        let superblock = Superblock {
            format_version: self.format_version,
            sequence: next_sequence,
            generation: self.generation,
            root: self.root,
            next_checkpoint: self.next_checkpoint,
            checkpoints: self.checkpoints,
            type_map_checksum: checksum(&map),
            limits: self.limits,
            next_object_id: self.next_object_id,
        };
        write_superblock::<MAX_BLOCKS>(image, bank, superblock)?;
        self.volume_bank = bank;
        self.volume_sequence = next_sequence;
        if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::Flush) {
            return Err(Error::Interrupted)
        }
        Ok(VolumeCommit {
            sequence: next_sequence,
            generation: self.generation,
        })
    }

    /// Persist the inactive generation directly to a block provider. The
    /// superblock is written last and the provider is flushed before the new
    /// generation becomes active, preserving the existing two-bank recovery
    /// protocol across power loss.
    pub fn flush_to_device<D: BlockStore>(
        &mut self,
        device: &mut D,
    ) -> Result<VolumeCommit, Error> {
        let mut no_interruption = NoInterruption;
        self.flush_to_device_with_interruption(device, &mut no_interruption)
    }

    /// End-to-end `fsync` for the persistent volume.
    ///
    /// This publishes the current CoW root, writes all data and type-map
    /// blocks before the superblock commit record, and waits for the block
    /// store's durability fence before returning.
    ///
    /// A successful return means the current generation is eligible for
    /// recovery after power loss. An error makes no durability promise.
    pub fn fsync<D: BlockStore>(&mut self, device: &mut D) -> Result<VolumeCommit, Error> {
        self.flush_to_device(device)
    }

    /// Compatibility name for callers that use the volume-level `sync` API.
    pub fn sync<D: BlockStore>(&mut self, device: &mut D) -> Result<VolumeCommit, Error> {
        self.fsync(device)
    }

    pub fn flush_to_device_with_interruption<D: BlockStore, I: InterruptionInjector>(
        &mut self,
        device: &mut D,
        injector: &mut I,
    ) -> Result<VolumeCommit, Error> {
        if MAX_BLOCKS > (BLOCK_SIZE - 8) * 4 {
            return Err(Error::Corrupt);
        }
        self.check_consistency()?;
        let next_sequence = self
            .volume_sequence
            .checked_add(1)
            .ok_or(Error::VersionOverflow)?;
        let bank = 1 - self.volume_bank;
        let base = generation_offset::<MAX_BLOCKS>(bank) / BLOCK_SIZE;
        let map = bank_type_map::<MAX_BLOCKS>(&self.arena);
        let mut scratch = [0; BLOCK_SIZE];
        device
            .write_block((base + 1) as u64, &map)
            .map_err(|_| Error::Io)?;
        for (index, slot) in self.arena.slots.iter().enumerate() {
            scratch.fill(0);
            match slot.block {
                None => {}
                Some(Block::Tree(tree)) => encode_tree(&mut scratch, tree)?,
                Some(Block::Data(data)) => encode_data(&mut scratch, data),
            }
            device
                .write_block((base + 2 + index) as u64, &scratch)
                .map_err(|_| Error::Io)?;
        }
        let superblock = Superblock {
            format_version: self.format_version,
            sequence: next_sequence,
            generation: self.generation,
            root: self.root,
            next_checkpoint: self.next_checkpoint,
            checkpoints: self.checkpoints,
            type_map_checksum: checksum(&map),
            limits: self.limits,
            next_object_id: self.next_object_id,
        };
        encode_superblock::<MAX_BLOCKS>(&mut scratch, superblock)?;
        device
            .write_block(base as u64, &scratch)
            .map_err(|_| Error::Io)?;
        device.flush().map_err(|_| Error::Io)?;
        self.volume_bank = bank;
        self.volume_sequence = next_sequence;
        if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::Flush) {
            return Err(Error::Interrupted)
        }
        Ok(VolumeCommit {
            sequence: next_sequence,
            generation: self.generation,
        })
    }

    /// Discard both on-device generation banks after the caller has removed
    /// the volume. This is explicit because discard is destructive.
    pub fn discard_from_device<D: BlockStore>(device: &mut D) -> Result<(), Error> {
        let mut no_interruption = NoInterruption;
        Self::discard_from_device_with_interruption(device, &mut no_interruption)
    }

    pub fn discard_from_device_with_interruption<D: BlockStore, I: InterruptionInjector>(
        device: &mut D,
        injector: &mut I,
    ) -> Result<(), Error> {
        for block in 0..Self::volume_blocks() {
            device.discard_block(block as u64).map_err(|_| Error::Io)?;
        }
        device.flush().map_err(|_| Error::Io)?;
        if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::Flush) {
            return Err(Error::Interrupted)
        }
        device
            .release(Self::volume_blocks() as u64)
            .map_err(|_| Error::Io)
    }

    pub fn check_consistency(&self) -> Result<(), Error> {
        if self.next_checkpoint == 0
            || self.next_object_id == 0
            || (self.limits.max_blocks != usize::MAX && self.limits.max_blocks > MAX_BLOCKS)
            || (self.root.is_some() && self.generation == 0)
        {
            return Err(Error::Corrupt);
        }
        let mut tree_seen = [false; MAX_BLOCKS];
        let mut data_owners = [None; MAX_BLOCKS];
        self.validate_root(self.root, &mut tree_seen, &mut data_owners)?;
        for (index, checkpoint) in self.checkpoints.iter().flatten().enumerate() {
            if checkpoint.info.id.raw() == 0
                || checkpoint.info.id.raw() >= self.next_checkpoint
                || checkpoint.info.generation > self.generation
                || self
                    .checkpoints
                    .iter()
                    .flatten()
                    .take(index)
                    .any(|previous| previous.info.id == checkpoint.info.id)
            {
                return Err(Error::Corrupt);
            }
            let mut tree_seen = [false; MAX_BLOCKS];
            let mut data_owners = [None; MAX_BLOCKS];
            self.validate_root(checkpoint.root, &mut tree_seen, &mut data_owners)?;
        }
        Ok(())
    }

    pub fn consistency_check(&self) -> Result<(), Error> {
        self.check_consistency()
    }

    fn validate_root(
        &self,
        root: BlockId,
        tree_seen: &mut [bool; MAX_BLOCKS],
        data_owners: &mut [Option<u64>; MAX_BLOCKS],
    ) -> Result<(), Error> {
        if !root.is_some() {
            return Ok(());
        }
        self.validate_tree(root, tree_seen, data_owners, None, None)
    }

    fn validate_tree(
        &self,
        id: BlockId,
        tree_seen: &mut [bool; MAX_BLOCKS],
        data_owners: &mut [Option<u64>; MAX_BLOCKS],
        lower: Option<FileKey>,
        upper: Option<FileKey>,
    ) -> Result<(), Error> {
        let index = block_index::<MAX_BLOCKS>(id)?;
        if tree_seen[index] {
            return Err(Error::Corrupt);
        }
        tree_seen[index] = true;
        let Block::Tree(tree) = self.arena.get(id)? else {
            return Err(Error::Corrupt);
        };
        match tree {
            TreeBlock::Leaf(leaf) => {
                let length = usize::from(leaf.len);
                if length == 0 || length > MAX_KEYS {
                    return Err(Error::Corrupt);
                }
                for pair in leaf.records[..length].windows(2) {
                    if pair[0].key >= pair[1].key {
                        return Err(Error::Corrupt);
                    }
                }
                for record in &leaf.records[..length] {
                    if record.key.version == 0
                        || record.key.file.len == 0
                        || usize::from(record.key.file.len) > MAX_PATH_BYTES
                        || record.created_at > self.generation
                        || lower.is_some_and(|bound| record.key < bound)
                        || upper.is_some_and(|bound| record.key >= bound)
                    {
                        return Err(Error::Corrupt);
                    }
                    if record.deleted {
                        if record.size != 0 || record.data.is_some() {
                            return Err(Error::Corrupt);
                        }
                    } else {
                        if record.link_count == 0 {
                            return Err(Error::Corrupt);
                        }
                        match record.file_type {
                            FileType::Directory => {
                                if record.object_id != 0
                                    || record.size != 0
                                    || record.data.is_some()
                                {
                                    return Err(Error::Corrupt);
                                }
                            }
                            FileType::Regular | FileType::Symlink => {
                                if record.object_id == 0 {
                                    return Err(Error::Corrupt);
                                }
                                self.validate_data(
                                    record.data,
                                    record.size,
                                    record.checksum,
                                    record.object_id,
                                    data_owners,
                                )?;
                            }
                        }
                    }
                }
            }
            TreeBlock::Branch(branch) => {
                let length = usize::from(branch.len);
                if length == 0 || length > MAX_KEYS {
                    return Err(Error::Corrupt);
                }
                for pair in branch.keys[..length].windows(2) {
                    if pair[0] >= pair[1] {
                        return Err(Error::Corrupt);
                    }
                }
                if branch.keys[..length]
                    .iter()
                    .any(|key| lower.is_some_and(|bound| *key < bound) || upper.is_some_and(|bound| *key >= bound))
                {
                    return Err(Error::Corrupt);
                }
                for index in 0..=length {
                    let child_lower = if index == 0 {
                        lower
                    } else {
                        Some(branch.keys[index - 1])
                    };
                    let child_upper = if index < length {
                        Some(branch.keys[index])
                    } else {
                        upper
                    };
                    self.validate_tree(
                        branch.children[index],
                        tree_seen,
                        data_owners,
                        child_lower,
                        child_upper,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn validate_data(
        &self,
        first: BlockId,
        size: u64,
        expected_checksum: u64,
        object_id: u64,
        data_owners: &mut [Option<u64>; MAX_BLOCKS],
    ) -> Result<(), Error> {
        let mut id = first;
        let mut total = 0u64;
        let mut data_checksum = 0xcbf29ce484222325_u64;
        let mut chain_seen = [false; MAX_BLOCKS];
        while id.is_some() {
            let index = block_index::<MAX_BLOCKS>(id)?;
            if chain_seen[index] {
                return Err(Error::Corrupt);
            }
            chain_seen[index] = true;
            if data_owners[index].is_some_and(|owner| owner != object_id) {
                return Err(Error::Corrupt);
            }
            data_owners[index] = Some(object_id);
            let Block::Data(data) = self.arena.get(id)? else {
                return Err(Error::Corrupt);
            };
            let length = usize::from(data.len);
            if length == 0 || length > DATA_BYTES || checksum(&data.bytes[..length]) != data.checksum {
                return Err(Error::Corrupt);
            }
            for byte in &data.bytes[..length] {
                data_checksum ^= *byte as u64;
                data_checksum = data_checksum.wrapping_mul(0x100000001b3);
            }
            total = total.checked_add(length as u64).ok_or(Error::Corrupt)?;
            id = data.next;
        }
        if total != size || data_checksum != expected_checksum {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
}

/// Select the newest generation whose header and every referenced block are
/// valid. A durable header alone is not a commit: a torn object or map makes
/// that bank ineligible, so recovery tries the other bank before reporting
/// unrecoverable corruption.
fn load_committed_generation<const MAX_BLOCKS: usize>(
    image: &[u8],
) -> Result<SynFs<MAX_BLOCKS>, Error> {
    let candidates = [
        read_superblock::<MAX_BLOCKS>(image, 0)?.map(|superblock| (0, superblock)),
        read_superblock::<MAX_BLOCKS>(image, 1)?.map(|superblock| (1, superblock)),
    ];
    let highest_sequence = candidates
        .iter()
        .flatten()
        .map(|(_, superblock)| superblock.sequence)
        .max()
        .ok_or(Error::Corrupt)?;
    if candidates
        .iter()
        .flatten()
        .filter(|(_, superblock)| superblock.sequence == highest_sequence)
        .count()
        > 1
    {
        return Err(Error::Corrupt);
    }
    let mut attempted = [false; 2];

    for _ in 0..candidates.len() {
        let mut selected: Option<(usize, Superblock)> = None;
        for (index, candidate) in candidates.iter().enumerate() {
            if attempted[index] {
                continue
            }
            if selected.is_none_or(|(_, current)| candidate.is_some_and(|(_, next)| next.sequence > current.sequence)) {
                selected = *candidate;
            }
        }
        let Some((index, superblock)) = selected else {
            break
        };
        attempted[index] = true;
        if let Ok(filesystem) = load_bank::<MAX_BLOCKS>(image, index, superblock) {
            return Ok(filesystem)
        }
    }
    Err(Error::Corrupt)
}

fn require_image_size<const MAX_BLOCKS: usize>(image: &[u8]) -> Result<(), Error> {
    if image.len() < SynFs::<MAX_BLOCKS>::volume_bytes() {
        return Err(Error::BufferTooSmall {
            required: SynFs::<MAX_BLOCKS>::volume_bytes(),
        });
    }
    Ok(())
}

fn block_index<const MAX_BLOCKS: usize>(id: BlockId) -> Result<usize, Error> {
    id.0.checked_sub(1)
        .map(|index| index as usize)
        .filter(|index| *index < MAX_BLOCKS)
        .ok_or(Error::Corrupt)
}

pub(crate) fn generation_offset<const MAX_BLOCKS: usize>(bank: usize) -> usize {
    bank * (MAX_BLOCKS + 2) * BLOCK_SIZE
}

fn block_slice<'a, const MAX_BLOCKS: usize>(
    image: &'a [u8],
    bank: usize,
    index: usize,
) -> Result<&'a [u8], Error> {
    if bank > 1 || index >= MAX_BLOCKS {
        return Err(Error::Corrupt);
    }
    let start = generation_offset::<MAX_BLOCKS>(bank) + (index + 2) * BLOCK_SIZE;
    image.get(start..start + BLOCK_SIZE).ok_or(Error::Corrupt)
}

fn block_slice_mut<'a, const MAX_BLOCKS: usize>(
    image: &'a mut [u8],
    bank: usize,
    index: usize,
) -> Result<&'a mut [u8], Error> {
    if bank > 1 || index >= MAX_BLOCKS {
        return Err(Error::Corrupt);
    }
    let start = generation_offset::<MAX_BLOCKS>(bank) + (index + 2) * BLOCK_SIZE;
    image
        .get_mut(start..start + BLOCK_SIZE)
        .ok_or(Error::Corrupt)
}

fn type_map_bytes<const MAX_BLOCKS: usize>() -> [u8; BLOCK_SIZE] {
    let mut map = [0; BLOCK_SIZE];
    map[..TYPE_MAP_MAGIC.len()].copy_from_slice(TYPE_MAP_MAGIC);
    map
}

fn bank_type_map<const MAX_BLOCKS: usize>(arena: &BlockArena<MAX_BLOCKS>) -> [u8; BLOCK_SIZE] {
    let mut map = type_map_bytes::<MAX_BLOCKS>();
    for (index, slot) in arena.slots.iter().enumerate() {
        let kind = match slot.block {
            None => TYPE_MAP_EMPTY,
            Some(Block::Tree(TreeBlock::Leaf(_))) => TYPE_MAP_TREE,
            Some(Block::Tree(TreeBlock::Branch(_))) => TYPE_MAP_BRANCH,
            Some(Block::Data(_)) => TYPE_MAP_DATA,
        };
        let byte = 8 + index / 4;
        let shift = (index % 4) * 2;
        map[byte] |= kind << shift;
    }
    map
}

fn map_kind(map: &[u8; BLOCK_SIZE], index: usize) -> u8 {
    let byte = 8 + index / 4;
    let shift = (index % 4) * 2;
    (map[byte] >> shift) & 0x03
}

fn write_type_map<const MAX_BLOCKS: usize>(
    image: &mut [u8],
    bank: usize,
    map: &[u8; BLOCK_SIZE],
) -> Result<(), Error> {
    if MAX_BLOCKS > (BLOCK_SIZE - 8) * 4 {
        return Err(Error::Corrupt);
    }
    let start = generation_offset::<MAX_BLOCKS>(bank) + BLOCK_SIZE;
    image
        .get_mut(start..start + BLOCK_SIZE)
        .ok_or(Error::Corrupt)?
        .copy_from_slice(map);
    Ok(())
}

fn read_type_map<const MAX_BLOCKS: usize>(
    image: &[u8],
    bank: usize,
    expected_checksum: u64,
) -> Result<[u8; BLOCK_SIZE], Error> {
    if MAX_BLOCKS > (BLOCK_SIZE - 8) * 4 {
        return Err(Error::Corrupt);
    }
    let start = generation_offset::<MAX_BLOCKS>(bank) + BLOCK_SIZE;
    let bytes = image.get(start..start + BLOCK_SIZE).ok_or(Error::Corrupt)?;
    if &bytes[..TYPE_MAP_MAGIC.len()] != TYPE_MAP_MAGIC || checksum(bytes) != expected_checksum {
        return Err(Error::Corrupt);
    }
    let used_bytes = 8 + MAX_BLOCKS.div_ceil(4);
    if bytes[used_bytes..].iter().any(|byte| *byte != 0) {
        return Err(Error::Corrupt);
    }
    let mut map = [0; BLOCK_SIZE];
    map.copy_from_slice(bytes);
    Ok(map)
}

pub(crate) fn write_superblock<const MAX_BLOCKS: usize>(
    image: &mut [u8],
    bank: usize,
    superblock: Superblock,
) -> Result<(), Error> {
    let start = generation_offset::<MAX_BLOCKS>(bank);
    let block = image
        .get_mut(start..start + BLOCK_SIZE)
        .ok_or(Error::Corrupt)?;
    encode_superblock::<MAX_BLOCKS>(block, superblock)
}

fn encode_superblock<const MAX_BLOCKS: usize>(
    block: &mut [u8],
    superblock: Superblock,
) -> Result<(), Error> {
    if block.len() < BLOCK_SIZE {
        return Err(Error::Corrupt);
    }
    block.fill(0);
    block[..8].copy_from_slice(SUPERBLOCK_MAGIC);
    put_u16(block, 8, superblock.format_version);
    put_u16(block, 10, SUPERBLOCK_HEADER_BYTES);
    put_u32(block, 12, BLOCK_SIZE as u32);
    put_u64(block, 16, MAX_BLOCKS as u64);
    put_u64(block, 24, superblock.sequence);
    put_u64(block, 32, superblock.generation);
    put_u32(block, 40, superblock.root.0);
    put_u64(block, 48, superblock.next_checkpoint);
    put_u32(
        block,
        56,
        superblock.checkpoints.iter().flatten().count() as u32,
    );
    put_u64(block, 64, superblock.type_map_checksum);
    put_u64(block, 72, superblock.next_object_id);
    let mut offset = 80;
    for checkpoint in superblock.checkpoints {
        if let Some(checkpoint) = checkpoint {
            put_u64(block, offset, checkpoint.info.id.raw());
            put_u64(block, offset + 8, checkpoint.info.generation);
            put_u32(block, offset + 16, checkpoint.root.0);
        }
        offset += 24;
    }
    put_u64(block, 464, superblock.limits.max_bytes);
    put_u64(block, 472, superblock.limits.max_files);
    put_u64(block, 480, superblock.limits.max_blocks as u64);
    let header_checksum = checksum(&block[..SUPERBLOCK_CHECKSUM_OFFSET]);
    put_u64(block, SUPERBLOCK_CHECKSUM_OFFSET, header_checksum);
    Ok(())
}

pub(crate) fn read_superblock<const MAX_BLOCKS: usize>(
    image: &[u8],
    bank: usize,
) -> Result<Option<Superblock>, Error> {
    let start = generation_offset::<MAX_BLOCKS>(bank);
    let block = image.get(start..start + BLOCK_SIZE).ok_or(Error::Corrupt)?;
    if &block[..SUPERBLOCK_MAGIC.len()] != SUPERBLOCK_MAGIC {
        return Ok(None);
    }
    if !supported_volume_format(u16_at(block, 8))
        || u16_at(block, 10) != SUPERBLOCK_HEADER_BYTES
        || u32_at(block, 12) != BLOCK_SIZE as u32
        || u64_at(block, 16) != MAX_BLOCKS as u64
        || u64_at(block, SUPERBLOCK_CHECKSUM_OFFSET)
            != checksum(&block[..SUPERBLOCK_CHECKSUM_OFFSET])
    {
        return Ok(None);
    }
    let checkpoint_count = u32_at(block, 56) as usize;
    if u64_at(block, 24) == 0
        || checkpoint_count > MAX_CHECKPOINTS
        || u64_at(block, 48) == 0
        || u64_at(block, 72) == 0
        || (u64_at(block, 480) != usize::MAX as u64
            && u64_at(block, 480) > MAX_BLOCKS as u64)
    {
        return Ok(None);
    }
    let root = BlockId(u32_at(block, 40));
    if root.is_some() && root.0 as usize > MAX_BLOCKS {
        return Ok(None);
    }
    let mut checkpoints = [None; MAX_CHECKPOINTS];
    let mut offset = 80;
    for slot in checkpoints.iter_mut().take(checkpoint_count) {
        let id = u64_at(block, offset);
        let generation = u64_at(block, offset + 8);
        let root = BlockId(u32_at(block, offset + 16));
        if id == 0
            || id >= u64_at(block, 48)
            || root.0 as usize > MAX_BLOCKS
            || generation > u64_at(block, 32)
        {
            return Ok(None);
        }
        *slot = Some(Checkpoint {
            info: CheckpointInfo {
                id: CheckpointId(id),
                generation,
            },
            root,
        });
        offset += 24;
    }
    if checkpoints.iter().flatten().enumerate().any(|(index, checkpoint)| {
        checkpoints
            .iter()
            .flatten()
            .take(index)
            .any(|previous| previous.info.id == checkpoint.info.id)
    }) {
        return Ok(None);
    }
    Ok(Some(Superblock {
        format_version: u16_at(block, 8),
        sequence: u64_at(block, 24),
        generation: u64_at(block, 32),
        root,
        next_checkpoint: u64_at(block, 48),
        checkpoints,
        type_map_checksum: u64_at(block, 64),
        limits: VolumeLimits {
            max_bytes: u64_at(block, 464),
            max_files: u64_at(block, 472),
            max_blocks: usize::try_from(u64_at(block, 480)).map_err(|_| Error::Corrupt)?,
        },
        next_object_id: u64_at(block, 72),
    }))
}

pub(crate) const fn supported_volume_format(version: u16) -> bool {
    version == PREVIOUS_VOLUME_FORMAT_VERSION || version == VOLUME_FORMAT_VERSION
}

pub(crate) fn load_bank<const MAX_BLOCKS: usize>(
    image: &[u8],
    bank: usize,
    superblock: Superblock,
) -> Result<SynFs<MAX_BLOCKS>, Error> {
    let map = read_type_map::<MAX_BLOCKS>(image, bank, superblock.type_map_checksum)?;
    let mut filesystem = SynFs {
        arena: BlockArena::new(),
        root: superblock.root,
        generation: superblock.generation,
        checkpoints: superblock.checkpoints,
        limits: superblock.limits,
        next_checkpoint: superblock.next_checkpoint,
        volume_bank: bank,
        volume_sequence: superblock.sequence,
        next_object_id: superblock.next_object_id,
        format_version: superblock.format_version,
    };
    for index in 0..MAX_BLOCKS {
        let block = block_slice::<MAX_BLOCKS>(image, bank, index)?;
        let decoded = match map_kind(&map, index) {
            TYPE_MAP_EMPTY => {
                if block.iter().any(|byte| *byte != 0) {
                    return Err(Error::Corrupt);
                }
                None
            }
            TYPE_MAP_TREE | TYPE_MAP_BRANCH => Some(decode_tree::<MAX_BLOCKS>(block)?),
            TYPE_MAP_DATA => Some(decode_data(block)?),
            _ => return Err(Error::Corrupt),
        };
        filesystem.arena.slots[index].block = decoded;
    }
    filesystem.check_consistency()?;
    Ok(filesystem)
}

fn encode_data(destination: &mut [u8], data: DataBlock) {
    destination.fill(0);
    put_u32(destination, 0, data.next.0);
    put_u16(destination, 4, data.len);
    put_u64(destination, 8, data.checksum);
    destination[16..].copy_from_slice(&data.bytes);
}

fn decode_data(source: &[u8]) -> Result<Block, Error> {
    let next = BlockId(u32_at(source, 0));
    let len = u16_at(source, 4);
    let mut bytes = [0; DATA_BYTES];
    bytes.copy_from_slice(source.get(16..16 + DATA_BYTES).ok_or(Error::Corrupt)?);
    if usize::from(len) > DATA_BYTES || checksum(&bytes[..usize::from(len)]) != u64_at(source, 8) {
        return Err(Error::Corrupt);
    }
    Ok(Block::Data(DataBlock {
        next,
        len,
        checksum: u64_at(source, 8),
        bytes,
    }))
}

fn encode_tree(destination: &mut [u8], tree: TreeBlock) -> Result<(), Error> {
    destination.fill(0);
    destination[..4].copy_from_slice(TREE_MAGIC);
    destination[5] = TREE_FORMAT_VERSION;
    let (kind, position) = match tree {
        TreeBlock::Leaf(leaf) => {
            let mut encoder = Encoder::new(&mut destination[16..]);
            encoder.put_u8(leaf.len)?;
            encoder.put_bytes(&[0; 3])?;
            for record in &leaf.records {
                encode_record(&mut encoder, *record)?;
            }
            (TYPE_MAP_TREE, encoder.position)
        }
        TreeBlock::Branch(branch) => {
            let mut encoder = Encoder::new(&mut destination[16..]);
            encoder.put_u8(branch.len)?;
            encoder.put_bytes(&[0; 3])?;
            for key in &branch.keys {
                encode_key(&mut encoder, *key)?;
            }
            for child in &branch.children {
                encoder.put_u32(child.0)?;
            }
            (TYPE_MAP_TREE + 1, encoder.position)
        }
    };
    destination[4] = kind;
    put_u16(destination, 6, position as u16);
    put_u64(destination, 8, checksum(&destination[16..16 + position]));
    Ok(())
}

fn decode_tree<const MAX_BLOCKS: usize>(source: &[u8]) -> Result<Block, Error> {
    if &source[..4] != TREE_MAGIC || source[5] != TREE_FORMAT_VERSION {
        return Err(Error::Corrupt);
    }
    let payload_len = usize::from(u16_at(source, 6));
    if payload_len > BLOCK_SIZE - 16 || checksum(&source[16..16 + payload_len]) != u64_at(source, 8)
    {
        return Err(Error::Corrupt);
    }
    let mut decoder = Decoder::new(&source[16..16 + payload_len]);
    match source[4] {
        TYPE_MAP_TREE => {
            let len = decoder.get_u8()?;
            decoder.get_bytes(3)?;
            if usize::from(len) > MAX_KEYS {
                return Err(Error::Corrupt);
            }
            let mut leaf = Leaf::EMPTY;
            leaf.len = len;
            for record in &mut leaf.records {
                *record = decode_record(&mut decoder)?;
            }
            Ok(Block::Tree(TreeBlock::Leaf(leaf)))
        }
        TYPE_MAP_BRANCH => {
            let len = decoder.get_u8()?;
            decoder.get_bytes(3)?;
            if usize::from(len) > MAX_KEYS {
                return Err(Error::Corrupt);
            }
            let mut branch = Branch::EMPTY;
            branch.len = len;
            for key in &mut branch.keys {
                *key = decode_key(&mut decoder)?;
            }
            for child in &mut branch.children {
                *child = BlockId(decoder.get_u32()?);
            }
            Ok(Block::Tree(TreeBlock::Branch(branch)))
        }
        _ => Err(Error::Corrupt),
    }
}

fn encode_key(encoder: &mut Encoder<'_>, key: FileKey) -> Result<(), Error> {
    encode_file_name(encoder, key.file)?;
    encoder.put_u32(key.version)
}

fn decode_key(decoder: &mut Decoder<'_>) -> Result<FileKey, Error> {
    Ok(FileKey {
        file: decode_file_name(decoder)?,
        version: decoder.get_u32()?,
    })
}

fn encode_record(encoder: &mut Encoder<'_>, record: FileRecord) -> Result<(), Error> {
    encode_key(encoder, record.key)?;
    encoder.put_u64(record.object_id)?;
    encoder.put_u64(record.size)?;
    encoder.put_u32(record.data.0)?;
    encoder.put_u64(record.checksum)?;
    encoder.put_u64(record.created_at)?;
    encoder.put_u8(record.deleted as u8)?;
    encoder.put_u8(record.file_type as u8)?;
    encoder.put_u32(record.link_count)?;
    encoder.put_u16(record.mode)
}

fn decode_record(decoder: &mut Decoder<'_>) -> Result<FileRecord, Error> {
    let key = decode_key(decoder)?;
    let object_id = decoder.get_u64()?;
    let size = decoder.get_u64()?;
    let data = BlockId(decoder.get_u32()?);
    let checksum = decoder.get_u64()?;
    let created_at = decoder.get_u64()?;
    let deleted = decoder.get_u8()?;
    let raw_file_type = decoder.get_u8()?;
    let file_type = if raw_file_type == 0 {
        FileType::Regular
    } else {
        FileType::from_raw(raw_file_type).ok_or(Error::Corrupt)?
    };
    let raw_link_count = decoder.get_u32()?;
    let mode = decoder.get_u16()?;
    if deleted > 1 {
        return Err(Error::Corrupt);
    }
    Ok(FileRecord {
        key,
        object_id,
        size,
        data,
        checksum,
        created_at,
        deleted: deleted != 0,
        file_type,
        link_count: if raw_link_count == 0 { 1 } else { raw_link_count },
        mode,
    })
}

fn encode_file_name(encoder: &mut Encoder<'_>, file: FileName) -> Result<(), Error> {
    encoder.put_u16(file.len)?;
    encoder.put_bytes(&file.bytes)
}

fn decode_file_name(decoder: &mut Decoder<'_>) -> Result<FileName, Error> {
    let len = decoder.get_u16()?;
    let mut bytes = [0; MAX_PATH_BYTES];
    let source = decoder.get_bytes(MAX_PATH_BYTES)?;
    if usize::from(len) > MAX_PATH_BYTES
        || core::str::from_utf8(&source[..usize::from(len)]).is_err()
    {
        return Err(Error::Corrupt);
    }
    bytes.copy_from_slice(source);
    Ok(FileName { bytes, len })
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes())
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes())
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes())
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}
