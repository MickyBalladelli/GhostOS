//! VM-facing disk specifications and attachment bookkeeping.

use super::{DiskFormat, DiskImage, StorageError};
use super::native_management::{self, NativeManager, guest_id, clone_to_temporary};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiskRole {
    System,
    Data,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiskController {
    Ahci,
    Nvme,
    VirtioBlk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskPersistence {
    /// Writes go to the configured image and survive VM shutdown.
    Persistent,
    /// Work on a temporary clone and discard it when the VM closes.
    CopyOnWrite,
    /// Work on a temporary clone and discard it when the VM closes.
    Disposable,
}

/// Short name for callers that model the attachment as a disk mode.
pub type DiskMode = DiskPersistence;

impl Default for DiskPersistence {
    fn default() -> Self {
        Self::Persistent
    }
}

/// A stable host-side description of one guest disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskSpec {
    pub id: String,
    pub role: DiskRole,
    pub controller: DiskController,
    pub bus: u8,
    pub slot: u8,
    pub image_path: PathBuf,
    /// `None` means detect the image format from its header.
    pub format: Option<DiskFormat>,
    /// Exact logical capacity in bytes. `None` accepts the image capacity.
    pub capacity: Option<u64>,
    pub read_only: bool,
    pub persistence: DiskPersistence,
}

impl DiskSpec {
    pub fn new(id: impl Into<String>, image_path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            role: DiskRole::Data,
            controller: DiskController::VirtioBlk,
            bus: 0,
            slot: 0,
            image_path: image_path.into(),
            format: None,
            capacity: None,
            read_only: false,
            persistence: DiskPersistence::Persistent,
        }
    }

    pub fn system(id: impl Into<String>, image_path: impl Into<PathBuf>) -> Self {
        let mut spec = Self::new(id, image_path);
        spec.role = DiskRole::System;
        spec
    }

    pub fn with_controller(mut self, controller: DiskController) -> Self {
        self.controller = controller;
        self
    }

    pub fn with_location(mut self, bus: u8, slot: u8) -> Self {
        self.bus = bus;
        self.slot = slot;
        self
    }

    pub fn with_format(mut self, format: DiskFormat) -> Self {
        self.format = Some(format);
        self
    }

    pub fn with_capacity(mut self, capacity: u64) -> Self {
        self.capacity = Some(capacity);
        self
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn with_persistence(mut self, persistence: DiskPersistence) -> Self {
        self.persistence = persistence;
        self
    }

    pub fn with_mode(self, mode: DiskMode) -> Self {
        self.with_persistence(mode)
    }
}

/// Stable inspection information for an attached disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInfo {
    pub id: String,
    pub role: DiskRole,
    pub controller: DiskController,
    pub bus: u8,
    pub slot: u8,
    pub image_path: PathBuf,
    pub format: DiskFormat,
    pub capacity: u64,
    pub read_only: bool,
    pub persistence: DiskPersistence,
    pub guest_id: String,
}

#[derive(Debug)]
pub struct AttachedDisk {
    pub info: DiskInfo,
}

pub struct DiskManager {
    native: NativeManager,
}

impl DiskManager {
    pub fn new() -> Self {
        Self { native: NativeManager::new() }
    }

    pub fn list(&self) -> Vec<DiskInfo> {
        self.native.list()
    }

    pub fn get(&self, id: &str) -> Option<&DiskInfo> {
        self.native.get(id)
    }

    /// Inspect a configured disk without claiming its writable ownership.
    ///
    /// This is used by management commands so inspection never changes the
    /// runtime ownership state of a VM disk.
    pub fn inspect(spec: &DiskSpec) -> Result<DiskInfo, StorageError> {
        let source = fs::canonicalize(&spec.image_path).map_err(|error| {
            StorageError::InvalidImage(format!(
                "cannot resolve disk `{}`: {error}",
                spec.image_path.display()
            ))
        })?;
        let image = DiskImage::open_with_access(&source, false)?;
        native_management::validate_image(spec, image.format(), image.size())?;
        Ok(DiskInfo {
            id: spec.id.clone(),
            role: spec.role,
            controller: spec.controller,
            bus: spec.bus,
            slot: spec.slot,
            image_path: source,
            format: image.format(),
            capacity: image.size(),
            read_only: spec.read_only,
            persistence: spec.persistence,
            guest_id: guest_id(spec),
        })
    }

    pub(crate) fn validate_specs(specs: &[DiskSpec]) -> Result<(), StorageError> {
        native_management::validate_specs(specs)
    }

    pub(crate) fn open(spec: &DiskSpec) -> Result<(DiskImage, DiskInfo), StorageError> {
        let source = fs::canonicalize(&spec.image_path).map_err(|error| {
            StorageError::InvalidImage(format!(
                "cannot resolve disk `{}`: {error}",
                spec.image_path.display()
            ))
        })?;
        let (path, cleanup_path) = if !native_management::needs_clone(spec) {
            (source.clone(), None)
        } else {
            let overlay = clone_to_temporary(&source)?;
            (overlay.clone(), Some(overlay))
        };

        let mut image = match DiskImage::open_for_vm(&path, !spec.read_only) {
            Ok(image) => image,
            Err(error) => {
                if let Some(path) = cleanup_path {
                    let _ = fs::remove_file(path);
                }
                return Err(error);
            }
        };
        if let Some(path) = cleanup_path {
            image.set_cleanup_path(path);
        }

        native_management::validate_image(spec, image.format(), image.size())?;
        let info = DiskInfo {
            id: spec.id.clone(),
            role: spec.role,
            controller: spec.controller,
            bus: spec.bus,
            slot: spec.slot,
            image_path: source,
            format: image.format(),
            capacity: image.size(),
            read_only: spec.read_only,
            persistence: spec.persistence,
            guest_id: guest_id(spec),
        };
        Ok((image, info))
    }

    pub(crate) fn insert(&mut self, info: DiskInfo) {
        self.native.insert(info);
    }

    pub(crate) fn remove(&mut self, id: &str) -> Option<DiskInfo> {
        self.native.remove(id)
    }
}

impl Default for DiskManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_path(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ghostos-disk-management-{label}-{}-{stamp}.raw",
            std::process::id()
        ))
    }

    fn create_raw(path: &Path) {
        File::create(path).unwrap().set_len(4096).unwrap();
    }

    fn read_sector(path: &Path) -> [u8; 512] {
        let mut image = DiskImage::open_with_access(path, false).unwrap();
        let mut sector = [0u8; 512];
        image.read_sector(0, &mut sector).unwrap();
        sector
    }

    #[test]
    fn copy_on_write_and_disposable_discard_guest_writes() {
        for persistence in [DiskPersistence::CopyOnWrite, DiskPersistence::Disposable] {
            let path = test_path("discard");
            create_raw(&path);
            let spec = DiskSpec::new("data", &path).with_persistence(persistence);
            let (mut overlay, info) = DiskManager::open(&spec).unwrap();
            assert_eq!(info.persistence, persistence);
            let overlay_path = PathBuf::from(overlay.filename());
            let sector = [0xD7u8; 512];
            overlay.write_sector(0, &sector).unwrap();
            overlay.sync().unwrap();
            drop(overlay);

            assert!(!overlay_path.exists());
            assert_eq!(read_sector(&path), [0u8; 512]);
            let _ = fs::remove_file(DiskImage::lock_path(&path));
            let _ = fs::remove_file(&path);
        }
    }

    #[test]
    fn read_only_attachment_isolated_from_writes() {
        let path = test_path("read-only");
        create_raw(&path);
        let spec = DiskSpec::new("data", &path).read_only(true);
        let (mut image, info) = DiskManager::open(&spec).unwrap();
        assert!(info.read_only);
        assert!(matches!(
            image.write_sector(0, &[0xE1u8; 512]),
            Err(StorageError::ReadOnly)
        ));
        drop(image);

        assert_eq!(read_sector(&path), [0u8; 512]);
        let _ = fs::remove_file(DiskImage::lock_path(&path));
        let _ = fs::remove_file(&path);
    }
}
