#![no_std]
#![forbid(unsafe_code)]

use synos_status::{IntoStatus, Severity, Status, facility};

mod ext4;
mod fat32;
mod ntfs;

pub use ext4::Ext4;
pub use fat32::Fat32;
pub use ntfs::Ntfs;

pub const MAX_PARTITIONS: usize = 32;

pub trait ReadAt {
    fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> Result<(), Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    BufferTooSmall,
    Corrupt,
    InvalidOffset,
    Io,
    NotFound,
    NotSupported,
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::NotFound => Status::NOT_FOUND,
            Self::Corrupt => Status::CORRUPT,
            Self::InvalidOffset | Self::BufferTooSmall => Status::INVALID_ARGUMENT,
            Self::Io => Status::new(Severity::Error, facility::FILESYSTEM, 10, 0)
                .expect("valid host filesystem status"),
            Self::NotSupported => {
                Status::new(Severity::Error, facility::FILESYSTEM, 11, 0)
                    .expect("valid host filesystem status")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartitionKind {
    EfiSystem,
    LinuxFilesystem,
    MicrosoftBasicData,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Partition {
    pub start: u64,
    pub length: u64,
    pub kind: PartitionKind,
}

impl Partition {
    pub const EMPTY: Self = Self {
        start: 0,
        length: 0,
        kind: PartitionKind::Other,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileSystemKind {
    Ext4,
    Fat32,
    Ntfs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct File {
    pub(crate) id: u64,
    pub(crate) auxiliary: u64,
    pub size: u64,
}

pub enum Volume {
    Ext4(Ext4),
    Fat32(Fat32),
    Ntfs(Ntfs),
}

impl Volume {
    pub fn mount<D: ReadAt>(
        device: &mut D,
        partition: Partition,
        scratch: &mut [u8],
    ) -> Result<Self, Error> {
        match detect(device, partition, scratch)? {
            FileSystemKind::Ext4 => Ext4::mount(device, partition, scratch).map(Self::Ext4),
            FileSystemKind::Fat32 => Fat32::mount(device, partition, scratch).map(Self::Fat32),
            FileSystemKind::Ntfs => Ntfs::mount(device, partition, scratch).map(Self::Ntfs),
        }
    }

    pub fn open<D: ReadAt>(
        &self,
        device: &mut D,
        path: &str,
        scratch: &mut [u8],
    ) -> Result<File, Error> {
        match self {
            Self::Ext4(volume) => volume.open(device, path, scratch),
            Self::Fat32(volume) => volume.open(device, path, scratch),
            Self::Ntfs(volume) => volume.open(device, path, scratch),
        }
    }

    pub fn read<D: ReadAt>(
        &self,
        device: &mut D,
        file: File,
        offset: u64,
        output: &mut [u8],
        scratch: &mut [u8],
    ) -> Result<usize, Error> {
        match self {
            Self::Ext4(volume) => volume.read(device, file, offset, output, scratch),
            Self::Fat32(volume) => volume.read(device, file, offset, output, scratch),
            Self::Ntfs(volume) => volume.read(device, file, offset, output, scratch),
        }
    }
}

pub fn scan_partitions<D: ReadAt>(
    device: &mut D,
    logical_block_size: usize,
    partitions: &mut [Partition],
    scratch: &mut [u8],
) -> Result<usize, Error> {
    if logical_block_size < 512 || scratch.len() < logical_block_size {
        return Err(Error::BufferTooSmall)
    }
    device.read_at(0, &mut scratch[..logical_block_size])?;
    if scratch[510..512] != [0x55, 0xaa] {
        return Err(Error::Corrupt)
    }

    let protective = scratch[450] == 0xee;
    if protective {
        return scan_gpt(device, logical_block_size, partitions, scratch)
    }

    let mut count = 0;
    for index in 0..4 {
        let entry = 446 + index * 16;
        let partition_type = scratch[entry + 4];
        let first_lba = le_u32(&scratch[entry + 8..]) as u64;
        let blocks = le_u32(&scratch[entry + 12..]) as u64;
        if partition_type == 0 || blocks == 0 {
            continue
        }
        if count == partitions.len() {
            break
        }
        partitions[count] = Partition {
            start: first_lba
                .checked_mul(logical_block_size as u64)
                .ok_or(Error::InvalidOffset)?,
            length: blocks
                .checked_mul(logical_block_size as u64)
                .ok_or(Error::InvalidOffset)?,
            kind: mbr_kind(partition_type),
        };
        count += 1;
    }
    Ok(count)
}

pub fn detect<D: ReadAt>(
    device: &mut D,
    partition: Partition,
    scratch: &mut [u8],
) -> Result<FileSystemKind, Error> {
    if scratch.len() < 2048 {
        return Err(Error::BufferTooSmall)
    }
    if partition.length < 2048
        || partition.start.checked_add(partition.length).is_none()
    {
        return Err(Error::Corrupt)
    }
    device.read_at(partition.start, &mut scratch[..2048])?;

    if scratch[3..11] == *b"NTFS    " {
        return Ok(FileSystemKind::Ntfs)
    }
    if scratch[82..90] == *b"FAT32   " {
        return Ok(FileSystemKind::Fat32)
    }
    if le_u16(&scratch[1080..]) == 0xef53 {
        return Ok(FileSystemKind::Ext4)
    }
    Err(Error::NotSupported)
}

fn scan_gpt<D: ReadAt>(
    device: &mut D,
    logical_block_size: usize,
    partitions: &mut [Partition],
    scratch: &mut [u8],
) -> Result<usize, Error> {
    device.read_at(logical_block_size as u64, &mut scratch[..logical_block_size])?;
    if scratch[..8] != *b"EFI PART" {
        return Err(Error::Corrupt)
    }

    let entries_lba = le_u64(&scratch[72..]);
    let entry_count = le_u32(&scratch[80..]) as usize;
    let entry_size = le_u32(&scratch[84..]) as usize;
    if entry_size < 128 || entry_size > scratch.len() {
        return Err(Error::NotSupported)
    }

    let mut count = 0;
    for index in 0..entry_count.min(MAX_PARTITIONS * 8) {
        let offset = entries_lba
            .checked_mul(logical_block_size as u64)
            .and_then(|base| base.checked_add((index * entry_size) as u64))
            .ok_or(Error::InvalidOffset)?;
        device.read_at(offset, &mut scratch[..entry_size])?;
        if scratch[..16].iter().all(|byte| *byte == 0) {
            continue
        }
        let first_lba = le_u64(&scratch[32..]);
        let last_lba = le_u64(&scratch[40..]);
        if last_lba < first_lba {
            return Err(Error::Corrupt)
        }
        if count == partitions.len() {
            break
        }
        partitions[count] = Partition {
            start: first_lba
                .checked_mul(logical_block_size as u64)
                .ok_or(Error::InvalidOffset)?,
            length: (last_lba - first_lba + 1)
                .checked_mul(logical_block_size as u64)
                .ok_or(Error::InvalidOffset)?,
            kind: gpt_kind(&scratch[..16]),
        };
        count += 1;
    }
    Ok(count)
}

fn mbr_kind(kind: u8) -> PartitionKind {
    match kind {
        0x0b | 0x0c | 0xef => PartitionKind::EfiSystem,
        0x07 => PartitionKind::MicrosoftBasicData,
        0x83 => PartitionKind::LinuxFilesystem,
        _ => PartitionKind::Other,
    }
}

fn gpt_kind(guid: &[u8]) -> PartitionKind {
    const EFI: [u8; 16] = [
        0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11,
        0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
    ];
    const LINUX: [u8; 16] = [
        0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47,
        0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d, 0xe4,
    ];
    const MICROSOFT_BASIC: [u8; 16] = [
        0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44,
        0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26, 0x99, 0xc7,
    ];

    if guid == EFI {
        PartitionKind::EfiSystem
    } else if guid == LINUX {
        PartitionKind::LinuxFilesystem
    } else if guid == MICROSOFT_BASIC {
        PartitionKind::MicrosoftBasicData
    } else {
        PartitionKind::Other
    }
}

pub(crate) fn path_components(path: &str) -> impl Iterator<Item = &str> {
    path.split(['/', '\\']).filter(|part| !part.is_empty())
}

pub(crate) fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

pub(crate) fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

pub(crate) fn le_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}
