//! Storage controllers and block backing stores.
//!
//! This module contains the guest-visible storage devices (AHCI SATA host
//! bus adapter and NVMe PCIe controller) plus the block-device image file
//! parser used to back their virtual disks.

pub mod ahci;
pub mod disk_image;
pub mod management;
pub mod nvme;
pub mod system_disk;

pub use ahci::{
    Ahci, AHCI_ABAR_SIZE, AHCI_CLASS, AHCI_DEVICE_ID, AHCI_PROG_IF, AHCI_SUBCLASS, AHCI_VENDOR_ID,
};
pub use disk_image::{DiskFormat, DiskImage};
pub use management::{
    AttachedDisk, DiskController, DiskInfo, DiskManager, DiskMode, DiskPersistence, DiskRole,
    DiskSpec,
};
pub use nvme::{
    Nvme, NVME_BAR0_SIZE, NVME_CLASS, NVME_DEVICE_ID, NVME_PROG_IF, NVME_SUBCLASS, NVME_VENDOR_ID,
};
pub use system_disk::{
    SystemDiskCreateOptions, SystemDiskInstall, SystemDiskLayout, SystemDiskManifest,
    SystemDiskProvisioner, SystemSetting, SYSTEM_DISK_ALIGNMENT, SYSTEM_DISK_FORMAT_VERSION,
    SYSTEM_DISK_MANIFEST_SIZE, SYSTEM_DISK_MIN_SIZE, SYSTEM_DISK_PAYLOAD_OFFSET,
    SYSTEM_DISK_SETTINGS_SIZE, SYNFS_SYSTEM_BLOCKS, SYNFS_SYSTEM_VOLUME_SIZE,
};

use crate::devices::DeviceError;
use std::fmt;

/// Errors produced by the storage stack.
#[derive(Debug)]
pub enum StorageError {
    Io(std::io::Error),
    InvalidImage(String),
    Locked { path: String, owner: String },
    ReadOnly,
    OutOfRange,
    Unsupported(String),
    Dma(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Io(e) => write!(f, "storage I/O error: {e}"),
            StorageError::InvalidImage(msg) => write!(f, "invalid disk image: {msg}"),
            StorageError::Locked { path, owner } => {
                write!(f, "disk is already locked by {path} ({owner})")
            }
            StorageError::ReadOnly => write!(f, "disk image opened read-only"),
            StorageError::OutOfRange => write!(f, "storage access out of range"),
            StorageError::Unsupported(msg) => write!(f, "unsupported storage feature: {msg}"),
            StorageError::Dma(msg) => write!(f, "storage DMA error: {msg}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        StorageError::Io(e)
    }
}

impl From<StorageError> for DeviceError {
    fn from(_: StorageError) -> Self {
        DeviceError::NotReady
    }
}
