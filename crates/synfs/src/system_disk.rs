//! Mount support for the physical SynOS system-disk format.

use crate::{BLOCK_SIZE, BlockDevice, BlockIoError, BlockStore, SynFs};

pub const SYSTEM_DISK_MANIFEST_BYTES: usize = 64 * 1024;
pub const SYSTEM_VOLUME_BLOCKS: usize = 32;
const LOGICAL_SECTOR_BYTES: u64 = 512;
// The BIOS stage-2 loader occupies the first 16 sectors. Keep the two
// manifests in the reserved boot area instead of overlapping that loader.
const MANIFEST_A_LBA: u64 = 256;
const MANIFEST_B_LBA: u64 = MANIFEST_A_LBA + SYSTEM_DISK_MANIFEST_BYTES as u64 / LOGICAL_SECTOR_BYTES;
const MANIFEST_MAGIC: &[u8; 8] = b"SYNMANIF";
const MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemDiskError {
    Io,
    InvalidManifest,
    InvalidLayout,
    CorruptVolume,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemDiskManifest {
    pub generation: u64,
    pub disk_size: u64,
    pub system_volume_offset: u64,
    pub system_volume_size: u64,
    pub system_volume_checksum: u32,
}

impl SystemDiskManifest {
    fn decode(bytes: &[u8]) -> Result<Self, SystemDiskError> {
        if bytes.len() != SYSTEM_DISK_MANIFEST_BYTES
            || bytes.get(..8) != Some(MANIFEST_MAGIC)
            || get_u32(bytes, 8) != MANIFEST_VERSION
        {
            return Err(SystemDiskError::InvalidManifest)
        }
        let stored = get_u32(bytes, bytes.len() - 4);
        let mut crc = 0xffff_ffffu32;
        for (index, byte) in bytes.iter().copied().enumerate() {
            let byte = if index >= bytes.len() - 4 { 0 } else { byte };
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                }
            }
        }
        if !crc != stored {
            return Err(SystemDiskError::InvalidManifest)
        }
        let manifest = Self {
            generation: get_u64(bytes, 12),
            disk_size: get_u64(bytes, 20),
            system_volume_offset: get_u64(bytes, 32 + 3 * 16),
            system_volume_size: get_u64(bytes, 40 + 3 * 16),
            system_volume_checksum: get_u32(bytes, 136),
        };
        let end = manifest
            .system_volume_offset
            .checked_add(manifest.system_volume_size)
            .ok_or(SystemDiskError::InvalidLayout)?;
        if manifest.system_volume_offset % BLOCK_SIZE as u64 != 0
            || manifest.system_volume_size != SynFs::<SYSTEM_VOLUME_BLOCKS>::volume_bytes() as u64
            || end > manifest.disk_size
        {
            return Err(SystemDiskError::InvalidLayout)
        }
        Ok(manifest)
    }
}

pub struct SystemDiskVolume<'a, D> {
    device: &'a mut D,
    start_lba: u64,
}

impl<'a, D: BlockDevice> SystemDiskVolume<'a, D> {
    pub fn new(device: &'a mut D, manifest: SystemDiskManifest) -> Result<Self, SystemDiskError> {
        if manifest.system_volume_offset % LOGICAL_SECTOR_BYTES != 0 {
            return Err(SystemDiskError::InvalidLayout)
        }
        Ok(Self {
            device,
            start_lba: manifest.system_volume_offset / LOGICAL_SECTOR_BYTES,
        })
    }

    fn lba(&self, block: u64) -> Result<u64, BlockIoError> {
        self.start_lba
            .checked_add(block.saturating_mul(BLOCK_SIZE as u64 / LOGICAL_SECTOR_BYTES))
            .ok_or(BlockIoError::InvalidRequest)
    }
}

impl<D: BlockDevice> BlockStore for SystemDiskVolume<'_, D> {
    fn read_block(&mut self, block: u64, output: &mut [u8]) -> Result<(), BlockIoError> {
        if output.len() != BLOCK_SIZE {
            return Err(BlockIoError::InvalidBlockSize)
        }
        self.device
            .read_block(self.lba(block)?, output)
            .map_err(|_| BlockIoError::DeviceUnavailable)
    }

    fn write_block(&mut self, block: u64, input: &[u8]) -> Result<(), BlockIoError> {
        if input.len() != BLOCK_SIZE {
            return Err(BlockIoError::InvalidBlockSize)
        }
        self.device
            .write_block(self.lba(block)?, input)
            .map_err(|_| BlockIoError::DeviceUnavailable)
    }

    fn flush(&mut self) -> Result<(), BlockIoError> {
        self.device.flush().map_err(|_| BlockIoError::DeviceUnavailable)
    }

    fn discard_block(&mut self, block: u64) -> Result<(), BlockIoError> {
        let first = self.lba(block)?;
        for sector in 0..BLOCK_SIZE as u64 / LOGICAL_SECTOR_BYTES {
            self.device
                .discard_block(first + sector)
                .map_err(|_| BlockIoError::DeviceUnavailable)?
        }
        Ok(())
    }
}

pub struct MountedSystemVolume<'a, D> {
    pub manifest: SystemDiskManifest,
    pub filesystem: SynFs<SYSTEM_VOLUME_BLOCKS>,
    pub volume: SystemDiskVolume<'a, D>,
}

impl<'a, D: BlockDevice> MountedSystemVolume<'a, D> {
    pub fn mount(
        device: &'a mut D,
        manifest_scratch: &mut [u8; SYSTEM_DISK_MANIFEST_BYTES],
        volume_image: &mut [u8],
    ) -> Result<Self, SystemDiskError> {
        let first = read_manifest(device, MANIFEST_A_LBA, manifest_scratch).ok();
        let second = read_manifest(device, MANIFEST_B_LBA, manifest_scratch).ok();
        let manifest = match (first, second) {
            (Some(left), Some(right)) if right.generation > left.generation => right,
            (Some(left), _) => left,
            (_, Some(right)) => right,
            _ => return Err(SystemDiskError::InvalidManifest),
        };
        let mut volume = SystemDiskVolume::new(device, manifest)?;
        let filesystem = SynFs::<SYSTEM_VOLUME_BLOCKS>::load_from_device(volume_image, &mut volume)
            .map_err(|_| SystemDiskError::CorruptVolume)?;
        Ok(Self {
            manifest,
            filesystem,
            volume,
        })
    }

    pub fn fsync(&mut self) -> Result<(), SystemDiskError> {
        self.filesystem
            .fsync(&mut self.volume)
            .map(|_| ())
            .map_err(|_| SystemDiskError::Io)
    }

    pub fn sync(&mut self) -> Result<(), SystemDiskError> {
        self.fsync()
    }
}

fn read_manifest<D: BlockDevice>(
    device: &mut D,
    start_lba: u64,
    scratch: &mut [u8; SYSTEM_DISK_MANIFEST_BYTES],
) -> Result<SystemDiskManifest, SystemDiskError> {
    for (index, block) in scratch.chunks_exact_mut(BLOCK_SIZE).enumerate() {
        device
            .read_block(
                start_lba + index as u64 * (BLOCK_SIZE as u64 / LOGICAL_SECTOR_BYTES),
                block,
            )
            .map_err(|_| SystemDiskError::Io)?
    }
    SystemDiskManifest::decode(scratch)
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    bytes
        .get(offset..offset.saturating_add(4))
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    bytes
        .get(offset..offset.saturating_add(8))
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0)
}
