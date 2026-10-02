//! Storage controllers and block backing stores.
//!
//! This module contains the guest-visible storage devices (AHCI SATA host
//! bus adapter and NVMe PCIe controller) plus the block-device image file
//! parser used to back their virtual disks.

pub mod ahci;
pub mod disk_image;
pub mod management;
mod native_io;
mod native_disk;
mod native_management;
pub mod nvme;
pub mod persistence;
pub mod system_disk;

pub use ahci::{
    Ahci, AHCI_ABAR_SIZE, AHCI_CLASS, AHCI_DEVICE_ID, AHCI_PROG_IF, AHCI_SUBCLASS, AHCI_VENDOR_ID,
};
pub use disk_image::{
    DiskFindingSeverity, DiskFormat, DiskImage, DiskInspectionFinding, DiskInspectionReport,
    DiskLockInfo, DiskRepairReport,
};
pub use management::{
    AttachedDisk, DiskController, DiskInfo, DiskManager, DiskMode, DiskPersistence, DiskRole,
    DiskSpec,
};
pub use nvme::{
    Nvme, NVME_BAR0_SIZE, NVME_CLASS, NVME_DEVICE_ID, NVME_PROG_IF, NVME_SUBCLASS, NVME_VENDOR_ID,
};
pub use persistence::SynosPersistencePort;
pub use system_disk::{
    SystemDiskBootArtifacts, SystemDiskCreateOptions, SystemDiskInstall, SystemDiskLayout,
    SystemDiskManifest, SystemDiskProvisioner, SystemDiskRepairReport, SystemDiskRollbackReport,
    SystemServicePackage,
    SystemSetting, SYSTEM_DISK_ALIGNMENT,
    SYSTEM_DISK_FORMAT_VERSION,
    SYSTEM_DISK_BOOT_RECORD_OFFSET, SYSTEM_DISK_BOOT_RECORD_SIZE, SYSTEM_DISK_MANIFEST_SIZE,
    SYSTEM_DISK_MIN_SIZE, SYSTEM_DISK_PAYLOAD_OFFSET,
    SYSTEM_DISK_SETTINGS_SIZE, GHOSTFS_SYSTEM_BLOCKS, GHOSTFS_SYSTEM_VOLUME_SIZE,
};

use crate::devices::DeviceError;

/// Legacy IDENTIFY serial written to AHCI/NVMe guests.
pub const VIRTUAL_DISK_SERIAL_LEGACY: &[u8] = b"SYNOSVM00001";
/// GhostOS IDENTIFY serial accepted alongside the legacy SynOS value.
pub const VIRTUAL_DISK_SERIAL_GHOSTOS: &[u8] = b"GHOSTVM00001";

pub fn virtual_disk_serial_accepted(serial: &[u8]) -> bool {
    serial == VIRTUAL_DISK_SERIAL_LEGACY || serial == VIRTUAL_DISK_SERIAL_GHOSTOS
}
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
                writeln!(f, "disk is already locked")?;
                writeln!(f, "  lock: {path}")?;
                let mut printed_owner = false;
                for line in owner.lines() {
                    let Some((key, value)) = line.split_once('=') else {
                        continue;
                    };
                    let label = match key {
                        "image_identity" => "image",
                        "owner_identity" => "owner",
                        "pid" => "pid",
                        "start_time" => "started",
                        "host_identity" => "host",
                        "format" => "format",
                        _ => continue,
                    };
                    writeln!(f, "  {label}: {}", value.trim())?;
                    printed_owner = true;
                }
                if !printed_owner {
                    writeln!(f, "  owner: {}", owner.trim().replace('\n', "; "))?;
                }
                let image_path = path
                    .strip_suffix(".ghostos.lock")
                    .or_else(|| path.strip_suffix(".synos.lock"))
                    .unwrap_or(path);
                writeln!(f)?;
                write!(
                    f,
                    "  action: ./target/release/ghostos-vm disk lock {image_path}",
                )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_disk_serial_accepts_ghostos_alias() {
        assert!(virtual_disk_serial_accepted(VIRTUAL_DISK_SERIAL_LEGACY));
        assert!(virtual_disk_serial_accepted(VIRTUAL_DISK_SERIAL_GHOSTOS));
        assert!(!virtual_disk_serial_accepted(b"XXXXXXXXXXXX"));
    }
}
