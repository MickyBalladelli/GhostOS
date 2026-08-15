//! Provisioning and validation for an installed SynOS system disk.
//!
//! The disk keeps a small boot area followed by two manifest slots. Payloads
//! are written first and the new manifest is written last. A torn write can
//! therefore fall back to the previous valid generation.

use super::{DiskFormat, DiskImage, StorageError};
use crate::devices::storage::disk_image::{sync_parent_directory, SECTOR_SIZE};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use synos_synfs::{ServiceManifest, ServiceManifestEntry, SynFs};

pub const SYSTEM_DISK_FORMAT_VERSION: u32 = 1;
pub const SYSTEM_DISK_ALIGNMENT: u64 = 1024 * 1024;
pub const SYSTEM_DISK_MIN_SIZE: u64 = 16 * 1024 * 1024;
pub const SYSTEM_DISK_MANIFEST_SIZE: u64 = 64 * 1024;
pub const SYSTEM_DISK_PAYLOAD_OFFSET: u64 = SYSTEM_DISK_ALIGNMENT;
pub const SYSTEM_DISK_BOOT_RECORD_OFFSET: u64 = 64 * 1024;
pub const SYSTEM_DISK_BOOT_RECORD_SIZE: u64 = SECTOR_SIZE;
pub const SYNFS_SYSTEM_BLOCKS: usize = 32;
pub const SYNFS_SYSTEM_VOLUME_SIZE: u64 = SynFs::<SYNFS_SYSTEM_BLOCKS>::volume_bytes() as u64;
pub const SYSTEM_DISK_SETTINGS_SIZE: u64 = 64 * 1024;

const HEADER_SIZE: usize = SECTOR_SIZE as usize;
const HEADER_METADATA_OFFSET: usize = 0x1c0;
const MANIFEST_MAGIC: &[u8; 8] = b"SYNMANIF";
const HEADER_MAGIC: &[u8; 8] = b"SYNOSDSK";
const SETTINGS_MAGIC: &[u8; 8] = b"SYNSET01";
// Keep the first 64 KiB available for BIOS stage 2. The manifest slots live
// in the boot metadata area, before the payloads, and are still redundant.
const MANIFEST_A_OFFSET: u64 = 128 * 1024;
const MANIFEST_B_OFFSET: u64 = MANIFEST_A_OFFSET + SYSTEM_DISK_MANIFEST_SIZE;
const PAYLOAD_ALIGNMENT: u64 = SYSTEM_DISK_ALIGNMENT;
const MAX_MANIFEST_STRING: usize = 16 * 1024;
const MAX_CAPABILITIES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDiskLayout {
    pub boot_metadata_offset: u64,
    pub boot_metadata_size: u64,
    pub kernel_offset: u64,
    pub kernel_size: u64,
    pub initrd_offset: u64,
    pub initrd_size: u64,
    pub system_volume_offset: u64,
    pub system_volume_size: u64,
    pub settings_offset: u64,
    pub settings_size: u64,
    pub reserved_offset: u64,
    pub reserved_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDiskManifest {
    pub generation: u64,
    pub disk_size: u64,
    pub format: DiskFormat,
    pub layout: SystemDiskLayout,
    pub boot_args: String,
    pub machine_identity: String,
    pub network_identity: String,
    pub capabilities: Vec<String>,
    pub kernel_checksum: u32,
    pub initrd_checksum: u32,
    pub system_volume_checksum: u32,
    pub settings_checksum: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDiskRepairReport {
    pub path: PathBuf,
    pub repaired_manifest_slot: Option<u64>,
    pub generation: u64,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDiskRollbackReport {
    pub path: PathBuf,
    pub restored_path: PathBuf,
    pub generation: u64,
}

/// Validated boot data loaded from an attached SynOS system disk.
#[derive(Debug, Clone)]
pub struct SystemDiskBootArtifacts {
    pub manifest: SystemDiskManifest,
    pub kernel: Vec<u8>,
    pub initrd: Option<Vec<u8>>,
    pub settings: Vec<SystemSetting>,
    pub system_volume: Vec<u8>,
}

impl SystemDiskBootArtifacts {
    pub fn setting(&self, key: &str) -> Option<&str> {
        self.settings
            .iter()
            .find(|setting| setting.key == key)
            .map(|setting| setting.value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemSetting {
    pub key: String,
    pub value: String,
}

impl SystemSetting {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SystemDiskInstall {
    pub kernel_path: PathBuf,
    pub initrd_path: Option<PathBuf>,
    pub boot_args: String,
    pub machine_identity: String,
    pub network_identity: String,
    pub capabilities: Vec<String>,
    pub settings: Vec<SystemSetting>,
    pub service_packages: Vec<SystemServicePackage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemServicePackage {
    pub role: u8,
    pub path: PathBuf,
}

impl SystemDiskInstall {
    pub fn new(kernel_path: impl Into<PathBuf>) -> Self {
        Self {
            kernel_path: kernel_path.into(),
            initrd_path: None,
            boot_args: String::new(),
            machine_identity: String::new(),
            network_identity: String::new(),
            capabilities: Vec::new(),
            settings: default_settings(),
            service_packages: Vec::new(),
        }
    }

    pub fn with_initrd(mut self, path: impl Into<PathBuf>) -> Self {
        self.initrd_path = Some(path.into());
        self
    }

    pub fn with_boot_args(mut self, value: impl Into<String>) -> Self {
        self.boot_args = value.into();
        self
    }

    pub fn with_machine_identity(mut self, value: impl Into<String>) -> Self {
        self.machine_identity = value.into();
        self
    }

    pub fn with_network_identity(mut self, value: impl Into<String>) -> Self {
        self.network_identity = value.into();
        self
    }

    pub fn with_capabilities(
        mut self,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.capabilities = values.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_setting(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.settings.push(SystemSetting::new(key, value));
        self
    }

    pub fn with_service_package(mut self, role: u8, path: impl Into<PathBuf>) -> Self {
        self.service_packages.push(SystemServicePackage {
            role,
            path: path.into(),
        });
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SystemDiskCreateOptions {
    pub size_bytes: u64,
    pub format: DiskFormat,
    pub reserved_bytes: u64,
    pub replace: bool,
}

impl SystemDiskCreateOptions {
    pub fn new(size_bytes: u64) -> Self {
        Self {
            size_bytes,
            format: DiskFormat::Raw,
            reserved_bytes: SYSTEM_DISK_ALIGNMENT * 8,
            replace: false,
        }
    }

    pub fn with_format(mut self, format: DiskFormat) -> Self {
        self.format = format;
        self
    }

    pub fn with_reserved_bytes(mut self, bytes: u64) -> Self {
        self.reserved_bytes = bytes;
        self
    }

    pub fn replace_existing(mut self, replace: bool) -> Self {
        self.replace = replace;
        self
    }
}

pub struct SystemDiskProvisioner;

impl SystemDiskProvisioner {
    /// Create an empty, aligned image. Existing images are never overwritten
    /// unless `replace` is explicitly set.
    pub fn create<P: AsRef<Path>>(
        path: P,
        options: SystemDiskCreateOptions,
    ) -> Result<(), StorageError> {
        let path = path.as_ref();
        validate_create_options(&options)?;
        if path.exists() && !options.replace {
            return Err(StorageError::InvalidImage(format!(
                "system disk `{}` already exists; request replacement explicitly",
                path.display()
            )));
        }
        if DiskImage::lock_path(path).exists() {
            return Err(StorageError::Locked {
                path: DiskImage::lock_path(path).display().to_string(),
                owner: "ownership marker exists".to_string(),
            });
        }

        let temporary = temporary_path(path)?;
        if let Err(error) = create_image_file(&temporary, options.size_bytes, options.format)
            .and_then(|_| install_header(&temporary, options.size_bytes))
        {
            let _ = fs::remove_file(&temporary);
            let _ = sync_parent_directory(&temporary);
            return Err(error);
        }
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            let _ = sync_parent_directory(&temporary);
            return Err(StorageError::Io(error));
        }
        sync_parent_directory(path.as_ref()).map_err(StorageError::Io)?;
        Ok(())
    }

    /// Provision a new image, or return the existing manifest when the
    /// requested installation is already present.
    pub fn provision<P: AsRef<Path>>(
        path: P,
        install: &SystemDiskInstall,
    ) -> Result<SystemDiskManifest, StorageError> {
        let path = path.as_ref();
        if path.exists() {
            match Self::validate(path) {
                Ok(manifest) => {
                    if install_matches(&manifest, install)? {
                        return Ok(manifest);
                    }
                    return Err(StorageError::InvalidImage(format!(
                        "system disk `{}` is already provisioned; use provision_with_options with replacement",
                        path.display()
                    )));
                }
                Err(_error) if is_valid_header(path) => return Self::install(path, install),
                Err(error) => return Err(error),
            }
        }
        let kernel_size = file_size(&install.kernel_path)?;
        let initrd_size = install
            .initrd_path
            .as_ref()
            .map(|path| file_size(path))
            .transpose()?
            .unwrap_or(0);
        let size = required_disk_size(kernel_size, initrd_size, 8 * SYSTEM_DISK_ALIGNMENT)?;
        Self::create(path, SystemDiskCreateOptions::new(size))?;
        Self::install(path, install)
    }

    /// Provision after creating the image with explicit format, size, and
    /// replacement policy.
    pub fn provision_with_options<P: AsRef<Path>>(
        path: P,
        options: SystemDiskCreateOptions,
        install: &SystemDiskInstall,
    ) -> Result<SystemDiskManifest, StorageError> {
        let path = path.as_ref();
        let kernel_size = file_size(&install.kernel_path)?;
        let initrd_size = install
            .initrd_path
            .as_ref()
            .map(|path| file_size(path))
            .transpose()?
            .unwrap_or(0);
        let required = required_disk_size(kernel_size, initrd_size, options.reserved_bytes)?;
        if options.size_bytes < required {
            return Err(StorageError::InvalidImage(format!(
                "system disk size {} is too small; need at least {}",
                options.size_bytes, required
            )));
        }
        Self::create(path, options)?;
        Self::install(path, install)
    }

    /// Install onto an empty image. This writes all payloads before publishing
    /// the generation manifest.
    pub fn install<P: AsRef<Path>>(
        path: P,
        install: &SystemDiskInstall,
    ) -> Result<SystemDiskManifest, StorageError> {
        validate_install(install)?;
        let kernel = fs::read(&install.kernel_path)?;
        let initrd = install
            .initrd_path
            .as_ref()
            .map(fs::read)
            .transpose()?
            .unwrap_or_default();
        let settings = encode_settings(install)?;
        let volume = create_system_volume(install, &settings)?;
        let mut image = DiskImage::open_for_vm(path.as_ref(), true)?;
        validate_header(&mut image)?;
        if has_manifest_marker(&mut image)? {
            return Err(StorageError::InvalidImage(
                "system disk already contains an installation manifest; request replacement explicitly"
                    .to_string(),
            ));
        }
        let disk_size = image.size();
        let layout = make_layout(disk_size, kernel.len() as u64, initrd.len() as u64)?;
        let generation = next_generation(&mut image)?;

        write_extent(
            &mut image,
            &layout.kernel_offset,
            layout.kernel_size,
            &kernel,
        )?;
        write_extent(
            &mut image,
            &layout.initrd_offset,
            layout.initrd_size,
            &initrd,
        )?;
        write_extent(
            &mut image,
            &layout.system_volume_offset,
            layout.system_volume_size,
            &volume,
        )?;
        write_extent(
            &mut image,
            &layout.settings_offset,
            layout.settings_size,
            &settings,
        )?;
        image.sync()?;

        let manifest = SystemDiskManifest {
            generation,
            disk_size,
            format: image.format(),
            layout,
            boot_args: install.boot_args.clone(),
            machine_identity: install.machine_identity.clone(),
            network_identity: install.network_identity.clone(),
            capabilities: install.capabilities.clone(),
            kernel_checksum: checksum(&kernel),
            initrd_checksum: checksum(&initrd),
            system_volume_checksum: checksum(&volume),
            settings_checksum: checksum(&settings),
        };
        let slot = if generation % 2 == 1 {
            MANIFEST_A_OFFSET
        } else {
            MANIFEST_B_OFFSET
        };
        write_manifest(&mut image, slot, &manifest)?;
        image.sync()?;
        write_boot_record(&mut image, &manifest)?;
        image.sync()?;
        sync_parent_directory(path.as_ref()).map_err(StorageError::Io)?;
        Ok(manifest)
    }

    /// Read and validate the newest complete installation generation.
    pub fn validate<P: AsRef<Path>>(path: P) -> Result<SystemDiskManifest, StorageError> {
        let mut image = DiskImage::open_with_access(path.as_ref(), false)?;
        read_best_manifest(&mut image)
    }

    /// Rebuild one damaged redundant manifest slot from the other valid slot.
    /// Payloads are never guessed at or rewritten. An explicit caller choice
    /// is required because this method opens the image for writing.
    pub fn repair<P: AsRef<Path>>(path: P) -> Result<SystemDiskRepairReport, StorageError> {
        let path = fs::canonicalize(path)?;
        let mut image = DiskImage::open_for_vm(&path, true)?;
        validate_header(&mut image)?;

        let mut valid = Vec::new();
        for offset in [MANIFEST_A_OFFSET, MANIFEST_B_OFFSET] {
            let bytes = read_extent(&mut image, offset, SYSTEM_DISK_MANIFEST_SIZE)?;
            if let Ok(manifest) = decode_manifest(&bytes) {
                if validate_manifest(&mut image, &manifest).is_ok() {
                    valid.push((offset, bytes, manifest.generation));
                }
            }
        }

        if valid.is_empty() {
            return Err(StorageError::InvalidImage(
                "no valid system-disk manifest is available for repair".to_string(),
            ));
        }
        if valid.len() == 2 {
            let generation = valid
                .iter()
                .map(|(_, _, generation)| *generation)
                .max()
                .unwrap_or(0);
            return Ok(SystemDiskRepairReport {
                path,
                repaired_manifest_slot: None,
                generation,
                actions: Vec::new(),
            });
        }

        let Some((source_offset, bytes, generation)) = valid.pop() else {
            return Err(StorageError::InvalidImage(
                "no valid system-disk manifest is available for repair".to_string(),
            ))
        };
        let target_offset = if source_offset == MANIFEST_A_OFFSET {
            MANIFEST_B_OFFSET
        } else {
            MANIFEST_A_OFFSET
        };
        write_extent(&mut image, &target_offset, SYSTEM_DISK_MANIFEST_SIZE, &bytes)?;
        image.sync()?;
        sync_parent_directory(&path)?;
        Ok(SystemDiskRepairReport {
            path,
            repaired_manifest_slot: Some(target_offset),
            generation,
            actions: vec![format!(
                "rebuilt system-disk manifest slot at byte offset {target_offset} from slot at byte offset {source_offset}"
            )],
        })
    }

    /// Replace an installed image atomically, retaining the previous image as
    /// a rollback candidate beside it.
    pub fn upgrade<P: AsRef<Path>>(
        path: P,
        install: &SystemDiskInstall,
    ) -> Result<SystemDiskManifest, StorageError> {
        let path = path.as_ref();
        let old = Self::validate(path)?;
        let temporary = temporary_path(path)?;
        let options = SystemDiskCreateOptions::new(old.disk_size).with_format(old.format);
        let result = Self::provision_with_options(&temporary, options, install);
        let manifest = match result {
            Ok(manifest) => manifest,
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error)
            }
        };
        let rollback = rollback_path(path);
        if rollback.exists() {
            return Err(StorageError::InvalidImage(format!(
                "rollback image `{}` already exists; recover or remove it first",
                rollback.display()
            )))
        }
        fs::rename(path, &rollback)?;
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::rename(&rollback, path);
            return Err(StorageError::Io(error))
        }
        sync_parent_directory(path).map_err(StorageError::Io)?;
        Ok(manifest)
    }

    /// Restore the image retained by [`Self::upgrade`]. The current image is
    /// retained as the next rollback candidate.
    pub fn rollback<P: AsRef<Path>>(path: P) -> Result<SystemDiskRollbackReport, StorageError> {
        let path = path.as_ref();
        let current = Self::validate(path)?;
        let rollback = rollback_path(path);
        let restored = Self::validate(&rollback)?;
        let temporary = temporary_path(path)?;
        fs::rename(path, &temporary)?;
        if let Err(error) = fs::rename(&rollback, path) {
            let _ = fs::rename(&temporary, path);
            return Err(StorageError::Io(error))
        }
        fs::rename(&temporary, &rollback)?;
        sync_parent_directory(path).map_err(StorageError::Io)?;
        Ok(SystemDiskRollbackReport {
            path: path.to_path_buf(),
            restored_path: rollback,
            generation: restored.generation.max(current.generation),
        })
    }

    pub fn inspect<P: AsRef<Path>>(path: P) -> Result<SystemDiskManifest, StorageError> {
        Self::validate(path)
    }

    /// Load the kernel, initrd, settings, and validated SynFS root volume.
    ///
    /// The image is opened read-only, so this is safe while the same image is
    /// attached to a writable VM controller.
    pub fn load_boot_artifacts<P: AsRef<Path>>(
        path: P,
    ) -> Result<SystemDiskBootArtifacts, StorageError> {
        let mut image = DiskImage::open_with_access(path.as_ref(), false)?;
        let manifest = read_best_manifest(&mut image)?;
        let kernel = read_extent(
            &mut image,
            manifest.layout.kernel_offset,
            manifest.layout.kernel_size,
        )?;
        let initrd = if manifest.layout.initrd_size == 0 {
            None
        } else {
            Some(read_extent(
                &mut image,
                manifest.layout.initrd_offset,
                manifest.layout.initrd_size,
            )?)
        };
        let settings_bytes = read_extent(
            &mut image,
            manifest.layout.settings_offset,
            manifest.layout.settings_size,
        )?;
        let settings = decode_settings(&settings_bytes)?;
        let system_volume = read_extent(
            &mut image,
            manifest.layout.system_volume_offset,
            manifest.layout.system_volume_size,
        )?;
        Ok(SystemDiskBootArtifacts {
            manifest,
            kernel,
            initrd,
            settings,
            system_volume,
        })
    }

    /// Commit a new validated SynFS system volume and publish its manifest.
    ///
    /// The volume is written before the redundant manifest slot. A reboot can
    /// therefore recover either the old complete generation or the new one.
    pub fn update_system_volume<P: AsRef<Path>>(
        path: P,
        volume: &[u8],
    ) -> Result<SystemDiskManifest, StorageError> {
        if volume.len() != SYNFS_SYSTEM_VOLUME_SIZE as usize {
            return Err(StorageError::InvalidImage(
                "system volume has the wrong size".to_string(),
            ));
        }
        let filesystem = SynFs::<SYNFS_SYSTEM_BLOCKS>::recover(volume)
            .map_err(|_| StorageError::InvalidImage("SynFS system volume is corrupt".to_string()))?;
        filesystem
            .check_consistency()
            .map_err(|_| StorageError::InvalidImage("SynFS system volume is inconsistent".to_string()))?;

        let path = path.as_ref();
        let mut image = DiskImage::open_for_vm(path, true)?;
        let mut manifest = read_best_manifest(&mut image)?;
        if manifest.layout.system_volume_size != volume.len() as u64 {
            return Err(StorageError::InvalidImage(
                "system volume does not match the installed disk layout".to_string(),
            ));
        }
        write_extent(
            &mut image,
            &manifest.layout.system_volume_offset,
            manifest.layout.system_volume_size,
            volume,
        )?;
        image.sync()?;

        manifest.generation = manifest
            .generation
            .checked_add(1)
            .ok_or_else(|| StorageError::InvalidImage("system-disk generation overflow".to_string()))?;
        manifest.system_volume_checksum = checksum(volume);
        let slot = if manifest.generation % 2 == 1 {
            MANIFEST_A_OFFSET
        } else {
            MANIFEST_B_OFFSET
        };
        write_manifest(&mut image, slot, &manifest)?;
        image.sync()?;
        write_boot_record(&mut image, &manifest)?;
        image.sync()?;
        sync_parent_directory(path).map_err(StorageError::Io)?;
        Ok(manifest)
    }
}

fn default_settings() -> Vec<SystemSetting> {
    vec![
        SystemSetting::new("hostname", "synos"),
        SystemSetting::new("timezone", "UTC"),
        SystemSetting::new("updates.channel", "stable"),
    ]
}

fn validate_create_options(options: &SystemDiskCreateOptions) -> Result<(), StorageError> {
    if options.size_bytes < SYSTEM_DISK_MIN_SIZE
        || options.size_bytes % SYSTEM_DISK_ALIGNMENT != 0
        || options.size_bytes % SECTOR_SIZE != 0
    {
        return Err(StorageError::InvalidImage(format!(
            "system disk size must be at least {} and aligned to {} bytes",
            SYSTEM_DISK_MIN_SIZE, SYSTEM_DISK_ALIGNMENT
        )));
    }
    if options.reserved_bytes < SYSTEM_DISK_ALIGNMENT
        || options.reserved_bytes % SYSTEM_DISK_ALIGNMENT != 0
    {
        return Err(StorageError::InvalidImage(
            "reserved system-disk space must be aligned to 1 MiB".to_string(),
        ));
    }
    if options.format == DiskFormat::Vhd && options.size_bytes > u32::MAX as u64 {
        return Err(StorageError::Unsupported(
            "fixed VHD images are limited to 4 GiB".to_string(),
        ));
    }
    Ok(())
}

fn validate_install(install: &SystemDiskInstall) -> Result<(), StorageError> {
    if install.boot_args.len() > MAX_MANIFEST_STRING
        || install.machine_identity.len() > MAX_MANIFEST_STRING
        || install.network_identity.len() > MAX_MANIFEST_STRING
        || install.capabilities.len() > MAX_CAPABILITIES
    {
        return Err(StorageError::InvalidImage(
            "system-disk identity or capability metadata is too large".to_string(),
        ));
    }
    if install.boot_args.contains('\0')
        || install.machine_identity.contains('\0')
        || install.network_identity.contains('\0')
    {
        return Err(StorageError::InvalidImage(
            "system-disk metadata contains a NUL byte".to_string(),
        ));
    }
    for capability in &install.capabilities {
        if capability.is_empty()
            || capability.len() > MAX_MANIFEST_STRING
            || capability.contains('\0')
        {
            return Err(StorageError::InvalidImage(
                "invalid system capability metadata".to_string(),
            ));
        }
    }
    for setting in &install.settings {
        if setting.key.is_empty()
            || setting.key.contains(['\0', '\n', '='])
            || setting.value.contains(['\0', '\n'])
        {
            return Err(StorageError::InvalidImage(
                "settings keys and values must be single-line text".to_string(),
            ));
        }
    }
    for (index, package) in install.service_packages.iter().enumerate() {
        if !(1..=14).contains(&package.role)
            || install.service_packages[..index]
                .iter()
                .any(|existing| existing.role == package.role)
        {
            return Err(StorageError::InvalidImage(
                "service package roles must be unique values from 1 through 14".to_string(),
            ));
        }
        fs::metadata(&package.path)?;
    }
    Ok(())
}

fn make_layout(
    disk_size: u64,
    kernel_size: u64,
    initrd_size: u64,
) -> Result<SystemDiskLayout, StorageError> {
    let kernel_offset = align_up(SYSTEM_DISK_PAYLOAD_OFFSET, PAYLOAD_ALIGNMENT)?;
    let initrd_offset = align_up(checked_add(kernel_offset, kernel_size)?, PAYLOAD_ALIGNMENT)?;
    let system_volume_offset =
        align_up(checked_add(initrd_offset, initrd_size)?, PAYLOAD_ALIGNMENT)?;
    let settings_offset = align_up(
        checked_add(system_volume_offset, SYNFS_SYSTEM_VOLUME_SIZE)?,
        PAYLOAD_ALIGNMENT,
    )?;
    let reserved_offset = align_up(
        checked_add(settings_offset, SYSTEM_DISK_SETTINGS_SIZE)?,
        PAYLOAD_ALIGNMENT,
    )?;
    if reserved_offset >= disk_size {
        return Err(StorageError::InvalidImage(
            "system disk has no reserved space after its system files".to_string(),
        ));
    }
    Ok(SystemDiskLayout {
        boot_metadata_offset: 0,
        boot_metadata_size: SYSTEM_DISK_PAYLOAD_OFFSET,
        kernel_offset,
        kernel_size,
        initrd_offset,
        initrd_size,
        system_volume_offset,
        system_volume_size: SYNFS_SYSTEM_VOLUME_SIZE,
        settings_offset,
        settings_size: SYSTEM_DISK_SETTINGS_SIZE,
        reserved_offset,
        reserved_size: disk_size - reserved_offset,
    })
}

fn required_disk_size(kernel: u64, initrd: u64, reserved: u64) -> Result<u64, StorageError> {
    let end = align_up(
        checked_add(
            align_up(
                checked_add(
                    align_up(
                        checked_add(SYSTEM_DISK_PAYLOAD_OFFSET, kernel)?,
                        PAYLOAD_ALIGNMENT,
                    )?,
                    initrd,
                )?,
                PAYLOAD_ALIGNMENT,
            )?,
            SYNFS_SYSTEM_VOLUME_SIZE + SYSTEM_DISK_SETTINGS_SIZE,
        )?,
        SYSTEM_DISK_ALIGNMENT,
    )?;
    let size = checked_add(end, reserved)?;
    Ok(size.max(SYSTEM_DISK_MIN_SIZE))
}

fn create_system_volume(
    install: &SystemDiskInstall,
    settings: &[u8],
) -> Result<Vec<u8>, StorageError> {
    let mut image = vec![0u8; SYNFS_SYSTEM_VOLUME_SIZE as usize];
    SynFs::<SYNFS_SYSTEM_BLOCKS>::format(&mut image)
        .map_err(|_| StorageError::InvalidImage("cannot format SynFS system volume".to_string()))?;
    let mut volume = SynFs::<SYNFS_SYSTEM_BLOCKS>::load(&image)
        .map_err(|_| StorageError::InvalidImage("cannot load SynFS system volume".to_string()))?;
    let settings_end = settings
        .iter()
        .rposition(|byte| *byte != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    for directory in [
        "/etc",
        "/etc/synos",
        "/system",
        "/system/services",
        "/var",
        "/var/log",
        "/home",
        "/tmp",
    ] {
        volume.create_directory(directory, true).map_err(|_| {
            StorageError::InvalidImage("cannot create system directories".to_string())
        })?;
    }
    volume
        .write("/etc/synos/settings", &settings[..settings_end])
        .map_err(|_| StorageError::InvalidImage("cannot write system settings".to_string()))?;
    volume
        .write("/etc/synos/machine-id", install.machine_identity.as_bytes())
        .map_err(|_| StorageError::InvalidImage("cannot write machine identity".to_string()))?;
    volume
        .write("/etc/synos/network-id", install.network_identity.as_bytes())
        .map_err(|_| StorageError::InvalidImage("cannot write network identity".to_string()))?;
    volume
        .write(
            "/etc/synos/capabilities",
            install.capabilities.join("\n").as_bytes(),
        )
        .map_err(|_| StorageError::InvalidImage("cannot write capabilities".to_string()))?;
    volume
        .write(
            "/etc/synos/disk-format",
            SYSTEM_DISK_FORMAT_VERSION.to_string().as_bytes(),
        )
        .map_err(|_| StorageError::InvalidImage("cannot write system version".to_string()))?;
    let mut service_manifest = ServiceManifest::new();
    for package in &install.service_packages {
        let image = fs::read(&package.path)?;
        let path = format!("/system/services/{}.pkg", package.role);
        volume
            .write(&path, &image)
            .map_err(|_| StorageError::InvalidImage("cannot write service package".to_string()))?;
        service_manifest
            .push(
                ServiceManifestEntry::new(package.role, &path, &image)
                    .map_err(|_| StorageError::InvalidImage("invalid service package".to_string()))?,
            )
            .map_err(|_| StorageError::InvalidImage("invalid service manifest".to_string()))?;
    }
    let mut manifest_bytes = [0; 2048];
    let manifest_length = service_manifest
        .encode(&mut manifest_bytes)
        .map_err(|_| StorageError::InvalidImage("cannot encode service manifest".to_string()))?;
    volume
        .write(
            synos_synfs::SERVICE_MANIFEST_PATH,
            &manifest_bytes[..manifest_length],
        )
        .map_err(|_| StorageError::InvalidImage("cannot write service manifest".to_string()))?;
    volume
        .flush(&mut image)
        .map_err(|_| StorageError::InvalidImage("cannot commit SynFS system volume".to_string()))?;
    volume.check_consistency().map_err(|_| {
        StorageError::InvalidImage("SynFS system volume is inconsistent".to_string())
    })?;
    Ok(image)
}

fn encode_settings(install: &SystemDiskInstall) -> Result<Vec<u8>, StorageError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(SETTINGS_MAGIC);
    push_setting(&mut bytes, "boot_args", &install.boot_args)?;
    push_setting(&mut bytes, "machine_identity", &install.machine_identity)?;
    push_setting(&mut bytes, "network_identity", &install.network_identity)?;
    push_setting(&mut bytes, "capabilities", &install.capabilities.join(","))?;
    for setting in &install.settings {
        push_setting(&mut bytes, &setting.key, &setting.value)?;
    }
    bytes.extend_from_slice(&checksum(&bytes).to_le_bytes());
    if bytes.len() as u64 > SYSTEM_DISK_SETTINGS_SIZE {
        return Err(StorageError::InvalidImage(
            "system settings exceed the settings extent".to_string(),
        ));
    }
    bytes.resize(SYSTEM_DISK_SETTINGS_SIZE as usize, 0);
    Ok(bytes)
}

fn decode_settings(bytes: &[u8]) -> Result<Vec<SystemSetting>, StorageError> {
    if bytes.len() < SETTINGS_MAGIC.len() + 4 || &bytes[..8] != SETTINGS_MAGIC {
        return Err(StorageError::InvalidImage(
            "invalid system settings header".to_string(),
        ));
    }

    let mut cursor = SETTINGS_MAGIC.len();
    let checksum_offset = loop {
        if cursor.checked_add(4).is_some_and(|end| end <= bytes.len())
            && checksum(&bytes[..cursor]) == get_u32(bytes, cursor)
            && bytes[cursor + 4..].iter().all(|byte| *byte == 0)
        {
            break cursor;
        }
        let header_end = cursor.checked_add(6).ok_or_else(|| {
            StorageError::InvalidImage("system settings record overflow".to_string())
        })?;
        if header_end > bytes.len() {
            return Err(StorageError::InvalidImage(
                "truncated system settings".to_string(),
            ));
        }
        let key_len = get_u16(bytes, cursor) as usize;
        let value_len = get_u32(bytes, cursor + 2) as usize;
        if key_len == 0 {
            return Err(StorageError::InvalidImage(
                "system setting key is empty".to_string(),
            ));
        }
        let key_start = header_end;
        let value_start = key_start.checked_add(key_len).ok_or_else(|| {
            StorageError::InvalidImage("system setting key overflow".to_string())
        })?;
        let end = value_start.checked_add(value_len).ok_or_else(|| {
            StorageError::InvalidImage("system setting value overflow".to_string())
        })?;
        if end > bytes.len() {
            return Err(StorageError::InvalidImage(
                "truncated system setting".to_string(),
            ));
        }
        let key = std::str::from_utf8(&bytes[key_start..value_start]).map_err(|_| {
            StorageError::InvalidImage("system setting key is not UTF-8".to_string())
        })?;
        let value = std::str::from_utf8(&bytes[value_start..end]).map_err(|_| {
            StorageError::InvalidImage("system setting value is not UTF-8".to_string())
        })?;
        if key.contains(['\0', '\n', '=']) || value.contains(['\0', '\n']) {
            return Err(StorageError::InvalidImage(
                "system setting contains an invalid character".to_string(),
            ));
        }
        cursor = end;
    };

    if checksum_offset + 4 > bytes.len() {
        return Err(StorageError::InvalidImage(
            "truncated system settings checksum".to_string(),
        ));
    }

    let mut settings = Vec::new();
    cursor = SETTINGS_MAGIC.len();
    while cursor < checksum_offset {
        let key_len = get_u16(bytes, cursor) as usize;
        let value_len = get_u32(bytes, cursor + 2) as usize;
        let key_start = cursor + 6;
        let value_start = key_start + key_len;
        let end = value_start + value_len;
        let key = std::str::from_utf8(&bytes[key_start..value_start])
            .map_err(|_| {
                StorageError::InvalidImage("system setting key is not UTF-8".to_string())
            })?
            .to_string();
        let value = std::str::from_utf8(&bytes[value_start..end])
            .map_err(|_| {
                StorageError::InvalidImage("system setting value is not UTF-8".to_string())
            })?
            .to_string();
        settings.push(SystemSetting { key, value });
        cursor = end;
    }
    Ok(settings)
}

fn push_setting(bytes: &mut Vec<u8>, key: &str, value: &str) -> Result<(), StorageError> {
    if key.len() > u16::MAX as usize || value.len() > u32::MAX as usize {
        return Err(StorageError::InvalidImage(
            "system setting is too large".to_string(),
        ));
    }
    bytes.extend_from_slice(&(key.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.extend_from_slice(key.as_bytes());
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn install_matches(
    manifest: &SystemDiskManifest,
    install: &SystemDiskInstall,
) -> Result<bool, StorageError> {
    validate_install(install)?;
    let kernel = fs::read(&install.kernel_path)?;
    let initrd = install
        .initrd_path
        .as_ref()
        .map(fs::read)
        .transpose()?
        .unwrap_or_default();
    if manifest.kernel_checksum != checksum(&kernel)
        || manifest.initrd_checksum != checksum(&initrd)
        || manifest.boot_args != install.boot_args
        || manifest.machine_identity != install.machine_identity
        || manifest.network_identity != install.network_identity
        || manifest.capabilities != install.capabilities
    {
        return Ok(false);
    }
    let settings = encode_settings(install)?;
    let volume = create_system_volume(install, &settings)?;
    Ok(checksum(&settings) == manifest.settings_checksum
        && checksum(&volume) == manifest.system_volume_checksum)
}

fn read_best_manifest(image: &mut DiskImage) -> Result<SystemDiskManifest, StorageError> {
    let mut candidates = Vec::new();
    let mut first_error = None;
    for offset in [MANIFEST_A_OFFSET, MANIFEST_B_OFFSET] {
        let bytes = read_extent(image, offset, SYSTEM_DISK_MANIFEST_SIZE)?;
        match decode_manifest(&bytes) {
            Ok(manifest) => match validate_manifest(image, &manifest) {
                Ok(()) => candidates.push(manifest),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            },
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    candidates
        .into_iter()
        .max_by_key(|manifest| manifest.generation)
        .ok_or_else(|| match first_error {
            Some(error) => StorageError::InvalidImage(format!(
                "no valid SynOS system-disk installation: {error}"
            )),
            None => StorageError::InvalidImage(
                "no valid SynOS system-disk installation".to_string(),
            ),
        })
}

fn validate_manifest(
    image: &mut DiskImage,
    manifest: &SystemDiskManifest,
) -> Result<(), StorageError> {
    validate_header(image)?;
    if manifest.generation == 0
        || manifest.disk_size != image.size()
        || manifest.format != image.format()
        || manifest.layout.boot_metadata_size != SYSTEM_DISK_PAYLOAD_OFFSET
        || manifest.layout.system_volume_size != SYNFS_SYSTEM_VOLUME_SIZE
        || manifest.layout.settings_size != SYSTEM_DISK_SETTINGS_SIZE
    {
        return Err(StorageError::InvalidImage(
            "invalid system-disk layout".to_string(),
        ));
    }
    let expected = make_layout(
        image.size(),
        manifest.layout.kernel_size,
        manifest.layout.initrd_size,
    )?;
    if manifest.layout != expected {
        return Err(StorageError::InvalidImage(
            "invalid system-disk extent layout".to_string(),
        ));
    }
    for (offset, size) in [
        (manifest.layout.kernel_offset, manifest.layout.kernel_size),
        (manifest.layout.initrd_offset, manifest.layout.initrd_size),
        (
            manifest.layout.system_volume_offset,
            manifest.layout.system_volume_size,
        ),
        (
            manifest.layout.settings_offset,
            manifest.layout.settings_size,
        ),
        (
            manifest.layout.reserved_offset,
            manifest.layout.reserved_size,
        ),
    ] {
        if offset % SECTOR_SIZE != 0
            || offset.checked_add(size).is_none()
            || offset + size > image.size()
        {
            return Err(StorageError::InvalidImage(
                "system-disk extent is out of range".to_string(),
            ));
        }
    }
    if checksum_extent(
        image,
        manifest.layout.kernel_offset,
        manifest.layout.kernel_size,
    )? != manifest.kernel_checksum
        || checksum_extent(
            image,
            manifest.layout.initrd_offset,
            manifest.layout.initrd_size,
        )? != manifest.initrd_checksum
        || checksum_extent(
            image,
            manifest.layout.system_volume_offset,
            manifest.layout.system_volume_size,
        )? != manifest.system_volume_checksum
        || checksum_extent(
            image,
            manifest.layout.settings_offset,
            manifest.layout.settings_size,
        )? != manifest.settings_checksum
    {
        return Err(StorageError::InvalidImage(
            "system-disk checksum mismatch".to_string(),
        ));
    }
    validate_boot_record(image, manifest)?;
    let settings = read_extent(
        image,
        manifest.layout.settings_offset,
        manifest.layout.settings_size,
    )?;
    decode_settings(&settings)?;
    let volume = read_extent(
        image,
        manifest.layout.system_volume_offset,
        manifest.layout.system_volume_size,
    )?;
    let filesystem = SynFs::<SYNFS_SYSTEM_BLOCKS>::recover(&volume)
        .map_err(|_| StorageError::InvalidImage("SynFS system volume is corrupt".to_string()))?;
    filesystem
        .check_consistency()
        .map_err(|_| StorageError::InvalidImage("SynFS system volume is inconsistent".to_string()))
}

fn validate_boot_record(
    image: &mut DiskImage,
    manifest: &SystemDiskManifest,
) -> Result<(), StorageError> {
    let record = read_extent(
        image,
        SYSTEM_DISK_BOOT_RECORD_OFFSET,
        SYSTEM_DISK_BOOT_RECORD_SIZE,
    )?;
    if &record[..8] != b"SYNBOOT1"
        || get_u32(&record, 8) != SYSTEM_DISK_FORMAT_VERSION
        || get_u64(&record, 12) != manifest.generation
        || get_u64(&record, 20) != manifest.layout.kernel_offset
        || get_u64(&record, 28) != manifest.layout.kernel_size
        || get_u64(&record, 36) != manifest.layout.initrd_offset
        || get_u64(&record, 44) != manifest.layout.initrd_size
        || get_u32(&record, 52) != manifest.kernel_checksum
        || get_u32(&record, 56) != manifest.initrd_checksum
        || get_u32(&record, 508) != boot_record_checksum(&record)
    {
        return Err(StorageError::InvalidImage(
            "invalid installed boot record".to_string(),
        ));
    }
    Ok(())
}

fn next_generation(image: &mut DiskImage) -> Result<u64, StorageError> {
    let mut generation = 0;
    for offset in [MANIFEST_A_OFFSET, MANIFEST_B_OFFSET] {
        let bytes = read_extent(image, offset, SYSTEM_DISK_MANIFEST_SIZE)?;
        if let Ok(manifest) = decode_manifest(&bytes) {
            generation = generation.max(manifest.generation);
        }
    }
    generation
        .checked_add(1)
        .ok_or_else(|| StorageError::InvalidImage("system-disk generation overflow".to_string()))
}

fn has_manifest_marker(image: &mut DiskImage) -> Result<bool, StorageError> {
    for offset in [MANIFEST_A_OFFSET, MANIFEST_B_OFFSET] {
        let bytes = read_extent(image, offset, SYSTEM_DISK_MANIFEST_SIZE)?;
        if &bytes[0..8] == MANIFEST_MAGIC {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_valid_header(path: &Path) -> bool {
    let Ok(mut image) = DiskImage::open_with_access(path, false) else {
        return false;
    };
    validate_header(&mut image).is_ok()
}

fn write_manifest(
    image: &mut DiskImage,
    offset: u64,
    manifest: &SystemDiskManifest,
) -> Result<(), StorageError> {
    let bytes = encode_manifest(manifest)?;
    write_extent(image, &offset, SYSTEM_DISK_MANIFEST_SIZE, &bytes)
}

fn encode_manifest(manifest: &SystemDiskManifest) -> Result<Vec<u8>, StorageError> {
    let mut bytes = vec![0u8; SYSTEM_DISK_MANIFEST_SIZE as usize];
    bytes[0..8].copy_from_slice(MANIFEST_MAGIC);
    put_u32(&mut bytes, 8, SYSTEM_DISK_FORMAT_VERSION);
    put_u64(&mut bytes, 12, manifest.generation);
    put_u64(&mut bytes, 20, manifest.disk_size);
    bytes[28] = format_code(manifest.format);
    let extents = [
        (
            manifest.layout.boot_metadata_offset,
            manifest.layout.boot_metadata_size,
        ),
        (manifest.layout.kernel_offset, manifest.layout.kernel_size),
        (manifest.layout.initrd_offset, manifest.layout.initrd_size),
        (
            manifest.layout.system_volume_offset,
            manifest.layout.system_volume_size,
        ),
        (
            manifest.layout.settings_offset,
            manifest.layout.settings_size,
        ),
        (
            manifest.layout.reserved_offset,
            manifest.layout.reserved_size,
        ),
    ];
    for (index, (offset, size)) in extents.into_iter().enumerate() {
        put_u64(&mut bytes, 32 + index * 16, offset);
        put_u64(&mut bytes, 40 + index * 16, size);
    }
    for (index, checksum) in [
        manifest.kernel_checksum,
        manifest.initrd_checksum,
        manifest.system_volume_checksum,
        manifest.settings_checksum,
    ]
    .into_iter()
    .enumerate()
    {
        put_u32(&mut bytes, 128 + index * 4, checksum);
    }
    let mut cursor = 160;
    push_manifest_string(&mut bytes, &mut cursor, &manifest.boot_args)?;
    push_manifest_string(&mut bytes, &mut cursor, &manifest.machine_identity)?;
    push_manifest_string(&mut bytes, &mut cursor, &manifest.network_identity)?;
    if manifest.capabilities.len() > MAX_CAPABILITIES {
        return Err(StorageError::InvalidImage(
            "too many system capabilities".to_string(),
        ));
    }
    put_u16(&mut bytes, cursor, manifest.capabilities.len() as u16);
    cursor += 2;
    for capability in &manifest.capabilities {
        push_manifest_string(&mut bytes, &mut cursor, capability)?;
    }
    let checksum_offset = bytes.len() - 4;
    let digest = checksum(&bytes);
    put_u32(&mut bytes, checksum_offset, digest);
    Ok(bytes)
}

fn decode_manifest(bytes: &[u8]) -> Result<SystemDiskManifest, StorageError> {
    if bytes.len() != SYSTEM_DISK_MANIFEST_SIZE as usize
        || &bytes[0..8] != MANIFEST_MAGIC
        || get_u32(bytes, 8) != SYSTEM_DISK_FORMAT_VERSION
    {
        return Err(StorageError::InvalidImage(
            "invalid system-disk manifest".to_string(),
        ));
    }
    let stored_checksum = get_u32(bytes, bytes.len() - 4);
    let mut checksum_bytes = bytes.to_vec();
    let checksum_offset = checksum_bytes.len() - 4;
    checksum_bytes[checksum_offset..].fill(0);
    if checksum(&checksum_bytes) != stored_checksum {
        return Err(StorageError::InvalidImage(
            "invalid system-disk manifest checksum".to_string(),
        ));
    }
    let format = decode_format(bytes[28])?;
    let extent = |index: usize| {
        (
            get_u64(bytes, 32 + index * 16),
            get_u64(bytes, 40 + index * 16),
        )
    };
    let (boot_metadata_offset, boot_metadata_size) = extent(0);
    let (kernel_offset, kernel_size) = extent(1);
    let (initrd_offset, initrd_size) = extent(2);
    let (system_volume_offset, system_volume_size) = extent(3);
    let (settings_offset, settings_size) = extent(4);
    let (reserved_offset, reserved_size) = extent(5);
    let mut cursor = 160;
    let boot_args = take_manifest_string(bytes, &mut cursor)?;
    let machine_identity = take_manifest_string(bytes, &mut cursor)?;
    let network_identity = take_manifest_string(bytes, &mut cursor)?;
    let count = get_u16(bytes, cursor) as usize;
    cursor += 2;
    if count > MAX_CAPABILITIES {
        return Err(StorageError::InvalidImage(
            "too many system capabilities".to_string(),
        ));
    }
    let mut capabilities = Vec::with_capacity(count);
    for _ in 0..count {
        capabilities.push(take_manifest_string(bytes, &mut cursor)?);
    }
    Ok(SystemDiskManifest {
        generation: get_u64(bytes, 12),
        disk_size: get_u64(bytes, 20),
        format,
        layout: SystemDiskLayout {
            boot_metadata_offset,
            boot_metadata_size,
            kernel_offset,
            kernel_size,
            initrd_offset,
            initrd_size,
            system_volume_offset,
            system_volume_size,
            settings_offset,
            settings_size,
            reserved_offset,
            reserved_size,
        },
        boot_args,
        machine_identity,
        network_identity,
        capabilities,
        kernel_checksum: get_u32(bytes, 128),
        initrd_checksum: get_u32(bytes, 132),
        system_volume_checksum: get_u32(bytes, 136),
        settings_checksum: get_u32(bytes, 140),
    })
}

fn push_manifest_string(
    bytes: &mut [u8],
    cursor: &mut usize,
    value: &str,
) -> Result<(), StorageError> {
    if value.len() > u16::MAX as usize || *cursor + 2 + value.len() > bytes.len() - 4 {
        return Err(StorageError::InvalidImage(
            "system-disk manifest is too large".to_string(),
        ));
    }
    put_u16(bytes, *cursor, value.len() as u16);
    *cursor += 2;
    bytes[*cursor..*cursor + value.len()].copy_from_slice(value.as_bytes());
    *cursor += value.len();
    Ok(())
}

fn take_manifest_string(bytes: &[u8], cursor: &mut usize) -> Result<String, StorageError> {
    if *cursor + 2 > bytes.len() - 4 {
        return Err(StorageError::InvalidImage(
            "truncated system-disk manifest".to_string(),
        ));
    }
    let length = get_u16(bytes, *cursor) as usize;
    *cursor += 2;
    if *cursor + length > bytes.len() - 4 {
        return Err(StorageError::InvalidImage(
            "truncated system-disk manifest".to_string(),
        ));
    }
    let value = std::str::from_utf8(&bytes[*cursor..*cursor + length])
        .map_err(|_| StorageError::InvalidImage("system-disk metadata is not UTF-8".to_string()))?
        .to_string();
    *cursor += length;
    Ok(value)
}

fn write_extent(
    image: &mut DiskImage,
    offset: &u64,
    size: u64,
    data: &[u8],
) -> Result<(), StorageError> {
    if *offset % SECTOR_SIZE != 0 || data.len() as u64 > size {
        return Err(StorageError::InvalidImage(
            "unaligned system-disk write".to_string(),
        ));
    }
    let mut sector = [0u8; HEADER_SIZE];
    let sectors = size.div_ceil(SECTOR_SIZE);
    for index in 0..sectors {
        sector.fill(0);
        let start = index as usize * HEADER_SIZE;
        if start < data.len() {
            let end = (start + HEADER_SIZE).min(data.len());
            sector[..end - start].copy_from_slice(&data[start..end]);
        }
        image.write_sector(*offset / SECTOR_SIZE + index, &sector)?;
    }
    Ok(())
}

fn read_extent(image: &mut DiskImage, offset: u64, size: u64) -> Result<Vec<u8>, StorageError> {
    if offset % SECTOR_SIZE != 0 {
        return Err(StorageError::InvalidImage(
            "unaligned system-disk read".to_string(),
        ));
    }
    let sectors = size.div_ceil(SECTOR_SIZE);
    let storage_size = sectors.checked_mul(SECTOR_SIZE).ok_or_else(|| {
        StorageError::InvalidImage("system-disk extent size overflows".to_string())
    })?;
    if offset
        .checked_add(storage_size)
        .map_or(true, |end| end > image.size())
    {
        return Err(StorageError::InvalidImage(
            "system-disk read is outside the image".to_string(),
        ));
    }
    let length = usize::try_from(size).map_err(|_| {
        StorageError::InvalidImage("system-disk extent is too large for this host".to_string())
    })?;
    let mut bytes = vec![0u8; length];
    for index in 0..sectors {
        let mut value = [0u8; HEADER_SIZE];
        image.read_sector(offset / SECTOR_SIZE + index, &mut value)?;
        let start = usize::try_from(index * SECTOR_SIZE).map_err(|_| {
            StorageError::InvalidImage("system-disk extent is too large for this host".to_string())
        })?;
        if start < bytes.len() {
            let end = (start + HEADER_SIZE).min(bytes.len());
            bytes[start..end].copy_from_slice(&value[..end - start]);
        }
    }
    Ok(bytes)
}

fn checksum_extent(image: &mut DiskImage, offset: u64, size: u64) -> Result<u32, StorageError> {
    let bytes = read_extent(image, offset, size)?;
    Ok(checksum(&bytes))
}

fn install_header(path: &Path, size: u64) -> Result<(), StorageError> {
    let mut header = [0u8; HEADER_SIZE];
    header[HEADER_METADATA_OFFSET..HEADER_METADATA_OFFSET + 8].copy_from_slice(HEADER_MAGIC);
    header[HEADER_METADATA_OFFSET + 8..HEADER_METADATA_OFFSET + 12]
        .copy_from_slice(&SYSTEM_DISK_FORMAT_VERSION.to_le_bytes());
    header[HEADER_METADATA_OFFSET + 12..HEADER_METADATA_OFFSET + 20]
        .copy_from_slice(&size.to_le_bytes());
    header[HEADER_METADATA_OFFSET + 20..HEADER_METADATA_OFFSET + 28]
        .copy_from_slice(&MANIFEST_A_OFFSET.to_le_bytes());
    header[HEADER_METADATA_OFFSET + 28..HEADER_METADATA_OFFSET + 36]
        .copy_from_slice(&MANIFEST_B_OFFSET.to_le_bytes());
    let mut image = DiskImage::open_with_access(path, true)?;
    image.write_sector(0, &header)?;
    image.sync()?;
    sync_parent_directory(path)?;
    Ok(())
}

fn write_boot_record(
    image: &mut DiskImage,
    manifest: &SystemDiskManifest,
) -> Result<(), StorageError> {
    let mut record = [0u8; SECTOR_SIZE as usize];
    record[0..8].copy_from_slice(b"SYNBOOT1");
    put_u32(&mut record, 8, SYSTEM_DISK_FORMAT_VERSION);
    put_u64(&mut record, 12, manifest.generation);
    put_u64(&mut record, 20, manifest.layout.kernel_offset);
    put_u64(&mut record, 28, manifest.layout.kernel_size);
    put_u64(&mut record, 36, manifest.layout.initrd_offset);
    put_u64(&mut record, 44, manifest.layout.initrd_size);
    put_u32(&mut record, 52, manifest.kernel_checksum);
    put_u32(&mut record, 56, manifest.initrd_checksum);
    let checksum_offset = record.len() - 4;
    put_u32(&mut record, checksum_offset, 0);
    let boot_checksum = boot_record_checksum(&record);
    put_u32(&mut record, checksum_offset, boot_checksum);
    write_extent(
        image,
        &SYSTEM_DISK_BOOT_RECORD_OFFSET,
        SYSTEM_DISK_BOOT_RECORD_SIZE,
        &record,
    )
}

fn validate_header(image: &mut DiskImage) -> Result<(), StorageError> {
    let mut header = [0u8; HEADER_SIZE];
    image.read_sector(0, &mut header)?;
    if &header[HEADER_METADATA_OFFSET..HEADER_METADATA_OFFSET + 8] != HEADER_MAGIC
        || get_u32(&header, HEADER_METADATA_OFFSET + 8) != SYSTEM_DISK_FORMAT_VERSION
        || get_u64(&header, HEADER_METADATA_OFFSET + 12) != image.size()
        || get_u64(&header, HEADER_METADATA_OFFSET + 20) != MANIFEST_A_OFFSET
        || get_u64(&header, HEADER_METADATA_OFFSET + 28) != MANIFEST_B_OFFSET
    {
        return Err(StorageError::InvalidImage(
            "invalid system-disk header".to_string(),
        ));
    }
    Ok(())
}

fn create_image_file(path: &Path, size: u64, format: DiskFormat) -> Result<(), StorageError> {
    let mut file = File::create(path)?;
    match format {
        DiskFormat::Raw => file.set_len(size)?,
        DiskFormat::Vhd => {
            file.set_len(size + 512)?;
            file.seek(SeekFrom::Start(size))?;
            file.write_all(&vhd_footer(size))?;
        }
        DiskFormat::Qcow2 => {
            let cluster_size = 64 * 1024u64;
            let coverage = cluster_size * (cluster_size / 8);
            let l1_size = size.div_ceil(coverage);
            if l1_size > u32::MAX as u64 {
                return Err(StorageError::Unsupported(
                    "QCOW2 image is too large".to_string(),
                ));
            }
            file.set_len(cluster_size * 2)?;
            let mut header = [0u8; 104];
            header[0..4].copy_from_slice(b"QFI\xfb");
            header[4..8].copy_from_slice(&2u32.to_be_bytes());
            header[20..24].copy_from_slice(&16u32.to_be_bytes());
            header[24..32].copy_from_slice(&size.to_be_bytes());
            header[36..40].copy_from_slice(&(l1_size as u32).to_be_bytes());
            header[40..48].copy_from_slice(&cluster_size.to_be_bytes());
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&header)?;
        }
    }
    file.sync_all()?;
    sync_parent_directory(path)?;
    Ok(())
}

fn vhd_footer(size: u64) -> [u8; 512] {
    let mut footer = [0u8; 512];
    footer[0..8].copy_from_slice(b"conectix");
    footer[12..16].copy_from_slice(&1u32.to_be_bytes());
    footer[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    footer[36..40].copy_from_slice(&(size as u32).to_be_bytes());
    footer[40..44].copy_from_slice(&(size as u32).to_be_bytes());
    footer[48..52].copy_from_slice(&2u32.to_be_bytes());
    let sum: u32 = footer
        .iter()
        .enumerate()
        .filter(|(index, _)| !(52..56).contains(index))
        .map(|(_, value)| *value as u32)
        .sum();
    footer[52..56].copy_from_slice(&(!sum).to_be_bytes());
    footer
}

fn temporary_path(path: &Path) -> Result<PathBuf, StorageError> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StorageError::InvalidImage("system clock is before Unix epoch".to_string()))?
        .as_nanos();
    for attempt in 0..32u32 {
        let candidate = path.with_extension(format!(
            "synos-provision-{}-{attempt}-{stamp}",
            std::process::id()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(StorageError::InvalidImage(
        "could not allocate a temporary system-disk path".to_string(),
    ))
}

fn rollback_path(path: &Path) -> PathBuf {
    path.with_extension("rollback")
}

fn boot_record_checksum(bytes: &[u8]) -> u32 {
    bytes[..bytes.len() - 4]
        .iter()
        .fold(0u32, |sum, byte| sum.wrapping_add(u32::from(*byte)))
}

fn file_size(path: &Path) -> Result<u64, StorageError> {
    Ok(fs::metadata(path)?.len())
}

fn align_up(value: u64, alignment: u64) -> Result<u64, StorageError> {
    if alignment == 0 {
        return Err(StorageError::InvalidImage("invalid alignment".to_string()));
    }
    let remainder = value % alignment;
    if remainder == 0 {
        Ok(value)
    } else {
        checked_add(value, alignment - remainder)
    }
}

fn checked_add(left: u64, right: u64) -> Result<u64, StorageError> {
    left.checked_add(right)
        .ok_or_else(|| StorageError::InvalidImage("system-disk size overflow".to_string()))
}

fn format_code(format: DiskFormat) -> u8 {
    match format {
        DiskFormat::Raw => 1,
        DiskFormat::Vhd => 2,
        DiskFormat::Qcow2 => 3,
    }
}

fn decode_format(value: u8) -> Result<DiskFormat, StorageError> {
    match value {
        1 => Ok(DiskFormat::Raw),
        2 => Ok(DiskFormat::Vhd),
        3 => Ok(DiskFormat::Qcow2),
        _ => Err(StorageError::InvalidImage(
            "unknown system-disk format".to_string(),
        )),
    }
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(bytes: &[u8], offset: usize) -> u16 {
    bytes
        .get(offset..offset.saturating_add(2))
        .and_then(|value| value.try_into().ok())
        .map(u16::from_le_bytes)
        .unwrap_or(0)
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    bytes
        .get(offset..offset.saturating_add(4))
        .and_then(|value| value.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    bytes
        .get(offset..offset.saturating_add(8))
        .and_then(|value| value.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0)
}

fn checksum(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::{Seek, SeekFrom, Write};

    fn test_path(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("synos-system-disk-{label}-{stamp}"))
    }

    #[test]
    fn provision_validate_and_reload_persistent_state() {
        let disk_path = test_path("persistent.raw");
        let kernel_path = test_path("kernel.bin");
        fs::write(&kernel_path, b"test kernel image").unwrap();

        let install = SystemDiskInstall::new(&kernel_path)
            .with_boot_args("console=serial0")
            .with_machine_identity("machine-a")
            .with_network_identity("network-a")
            .with_setting("test.key", "test-value");
        let manifest = SystemDiskProvisioner::provision_with_options(
            &disk_path,
            SystemDiskCreateOptions::new(SYSTEM_DISK_MIN_SIZE),
            &install,
        )
        .unwrap();

        let validated = SystemDiskProvisioner::validate(&disk_path).unwrap();
        assert_eq!(validated, manifest);
        let artifacts = SystemDiskProvisioner::load_boot_artifacts(&disk_path).unwrap();
        assert_eq!(artifacts.kernel, b"test kernel image");
        assert_eq!(artifacts.setting("test.key"), Some("test-value"));

        fs::remove_file(&disk_path).ok();
        fs::remove_file(&kernel_path).ok();
    }

    #[test]
    fn validation_rejects_corrupt_settings_metadata() {
        let disk_path = test_path("corrupt-settings.raw");
        let kernel_path = test_path("corrupt-settings-kernel.bin");
        fs::write(&kernel_path, b"test kernel image").unwrap();
        let install = SystemDiskInstall::new(&kernel_path);
        let manifest = SystemDiskProvisioner::provision(&disk_path, &install).unwrap();

        let mut file = OpenOptions::new().write(true).open(&disk_path).unwrap();
        file.seek(SeekFrom::Start(manifest.layout.settings_offset + 8))
            .unwrap();
        file.write_all(&[0xff]).unwrap();
        file.sync_all().unwrap();

        assert!(SystemDiskProvisioner::validate(&disk_path).is_err());

        fs::remove_file(&disk_path).ok();
        fs::remove_file(&kernel_path).ok();
    }
}
