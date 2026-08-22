//! Block-device image file parser.
//!
//! Supports three backing formats:
//!   * RAW - a plain byte-for-byte image of the disk;
//!   * VHD (fixed-size) - the Microsoft Virtual Hard Disk format;
//!   * QCOW2 - the QEMU Copy-On-Write v2 format with L1/L2 cluster
//!     indirection.
//!
//! The image is opened read/write when the file is writable, otherwise
//! read-only. Write-back is immediate and transparent to the controller
//! layer, which simply issues sector-aligned reads and writes.

use crate::devices::storage::StorageError;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_os = "linux"))]
use std::process::Command;
use std::process::Stdio;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// Disk image format as detected from the file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskFormat {
    Raw,
    Vhd,
    Qcow2,
}

/// Sector size used by the block layer (both AHCI and NVMe expose 512-byte
/// logical sectors).
pub const SECTOR_SIZE: u64 = 512;

const QCOW_CLUSTER_OFFSET_MASK: u64 = 0x00FF_FFFF_FFFF_FE00;

/// Magic numbers of the supported image formats.
const VHD_MAGIC: &[u8; 8] = b"conectix";
const QCOW2_MAGIC: &[u8; 4] = b"QFI\xfb";

/// Parsed QCOW2 header (versions 2 and 3 share the same header layout).
struct QcowHeader {
    cluster_bits: u32,
    disk_size: u64,
    l1_table_offset: u64,
    l1_size: u32,
    l2_bits: u32,
}

/// A parsed QCOW2 image.
struct QcowFile {
    file: File,
    header: QcowHeader,
    cluster_size: u64,
    l2_entries: u32,
    l1_cache: Vec<u64>,
    /// L2 table cache keyed by its physical file offset (0 = not loaded).
    l2_cache_offset: u64,
    l2_cache: Vec<u64>,
    writable: bool,
}

/// A parsed fixed-size VHD image.
struct VhdFile {
    file: File,
    disk_size: u64,
    writable: bool,
}

/// A block-device image backing a virtual disk.
pub struct DiskImage {
    format: DiskFormat,
    size: u64,
    raw: Option<File>,
    vhd: Option<VhdFile>,
    qcow: Option<QcowFile>,
    writable: bool,
    filename: String,
    lock: Option<DiskLock>,
    cleanup_path: Option<PathBuf>,
}

/// A process-local ownership marker for a writable VM disk.
///
/// The marker is deliberately a separate file so read-only users can share
/// an image. `create_new` makes competing writable VM launches fail without
/// relying on platform-specific file-locking APIs.
struct DiskLock {
    path: PathBuf,
    token: String,
    file: Option<File>,
}

const LOCK_RECORD_VERSION: u32 = 2;
static LOCK_TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Metadata for a disk ownership marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskLockInfo {
    pub path: PathBuf,
    pub owner: String,
    pub owner_identity: Option<String>,
    pub pid: Option<u32>,
    pub start_time: Option<String>,
    pub host_identity: Option<String>,
    pub image_path: Option<PathBuf>,
    pub format: Option<DiskFormat>,
    pub stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskFindingSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInspectionFinding {
    pub code: String,
    pub severity: DiskFindingSeverity,
    pub message: String,
    pub repairable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInspectionReport {
    pub path: PathBuf,
    pub file_size: Option<u64>,
    pub format: Option<DiskFormat>,
    pub capacity: Option<u64>,
    pub lock: Option<DiskLockInfo>,
    pub findings: Vec<DiskInspectionFinding>,
}

impl DiskInspectionReport {
    pub fn is_healthy(&self) -> bool {
        !self
            .findings
            .iter()
            .any(|finding| finding.severity == DiskFindingSeverity::Error)
    }

    pub fn repairable_findings(&self) -> impl Iterator<Item = &DiskInspectionFinding> {
        self.findings.iter().filter(|finding| finding.repairable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskRepairReport {
    pub path: PathBuf,
    pub format: DiskFormat,
    pub actions: Vec<String>,
}

impl Drop for DiskLock {
    fn drop(&mut self) {
        let owned = fs::read_to_string(&self.path)
            .ok()
            .and_then(|record| lock_field(&record, "lock_token").map(str::to_string))
            .is_some_and(|token| token == self.token);
        drop(self.file.take());
        if owned && fs::remove_file(&self.path).is_ok() {
            let _ = sync_parent_directory(&self.path);
        }
    }
}

impl DiskImage {
    /// Open `path`, detect its format, and prepare it for sector I/O.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        match Self::open_with_access(&path, true) {
            Ok(image) => Ok(image),
            Err(StorageError::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                Self::open_with_access(path, false)
            }
            Err(error) => Err(error),
        }
    }

    /// Open an image with an explicit access mode.
    pub fn open_with_access<P: AsRef<Path>>(
        path: P,
        writable: bool,
    ) -> Result<Self, StorageError> {
        let path = path.as_ref();

        let (mut file, writable) = if writable {
            // Open read/write when requested. A caller that wants a
            // read-only image must say so explicitly; silently downgrading a
            // writable VM disk would hide configuration errors.
            (OpenOptions::new().read(true).write(true).open(path)?, true)
        } else {
            (File::open(path)?, false)
        };

        let file_len = file.metadata()?.len();
        if file_len < 8 {
            return Err(StorageError::InvalidImage(
                "disk image header is truncated".to_string(),
            ));
        }

        let mut magic = [0u8; 8];
        file.read_exact(&mut magic)?;

        // VHD stores its "conectix" cookie in a 512-byte footer at the end
        // of the file, so detect it last by seeking to the tail.
        let format = detect_format(&mut file, file_len, magic)?;

        match format {
            DiskFormat::Raw => {
                let size = file_len;
                validate_sector_capacity(size, "RAW")?;
                Ok(Self {
                    format,
                    size,
                    raw: Some(file),
                    vhd: None,
                    qcow: None,
                    writable,
                    filename: path.display().to_string(),
                    lock: None,
                    cleanup_path: None,
                })
            }
            DiskFormat::Vhd => {
                let vhd = Self::open_vhd(file, writable)?;
                let size = vhd.disk_size;
                Ok(Self {
                    format,
                    size,
                    raw: None,
                    vhd: Some(vhd),
                    qcow: None,
                    writable,
                    filename: path.display().to_string(),
                    lock: None,
                    cleanup_path: None,
                })
            }
            DiskFormat::Qcow2 => {
                let qcow = Self::open_qcow2(file, writable)?;
                let size = qcow.header.disk_size;
                Ok(Self {
                    format,
                    size,
                    raw: None,
                    vhd: None,
                    qcow: Some(qcow),
                    writable,
                    filename: path.display().to_string(),
                    lock: None,
                    cleanup_path: None,
                })
            }
        }
    }

    /// Inspect an image without taking a writable lock or opening it for
    /// writes. Failed checks remain in the report so callers can see why an
    /// image is unhealthy.
    pub fn inspect_report<P: AsRef<Path>>(path: P) -> DiskInspectionReport {
        let requested_path = path.as_ref();
        let canonical_path = fs::canonicalize(requested_path)
            .unwrap_or_else(|_| requested_path.to_path_buf());
        let file_size = fs::metadata(requested_path).ok().map(|metadata| metadata.len());
        let mut report = DiskInspectionReport {
            path: canonical_path,
            file_size,
            format: None,
            capacity: None,
            lock: None,
            findings: Vec::new(),
        };

        match Self::inspect_lock(requested_path) {
            Ok(lock) => report.lock = lock,
            Err(error) => report.findings.push(DiskInspectionFinding {
                code: "lock.inspect".to_string(),
                severity: DiskFindingSeverity::Warning,
                message: format!("could not inspect ownership lock: {error}"),
                repairable: false,
            }),
        }

        match Self::open_with_access(requested_path, false) {
            Ok(image) => {
                report.format = Some(image.format());
                report.capacity = Some(image.size());
                report.findings.push(DiskInspectionFinding {
                    code: "image.valid".to_string(),
                    severity: DiskFindingSeverity::Info,
                    message: format!(
                        "{} image is structurally valid with {} bytes capacity",
                        format_name(image.format()),
                        image.size()
                    ),
                    repairable: false,
                });
            }
            Err(error) => {
                if vhd_footer_is_repairable(requested_path).unwrap_or(false) {
                    report.format = Some(DiskFormat::Vhd);
                    report.findings.push(DiskInspectionFinding {
                        code: "vhd.footer_checksum".to_string(),
                        severity: DiskFindingSeverity::Error,
                        message: "fixed VHD footer checksum is incorrect".to_string(),
                        repairable: true,
                    });
                } else {
                    report.findings.push(DiskInspectionFinding {
                        code: "image.invalid".to_string(),
                        severity: DiskFindingSeverity::Error,
                        message: error.to_string(),
                        repairable: false,
                    });
                }
            }
        }

        report
    }

    /// Repair only metadata that can be reconstructed without guessing guest
    /// data. The caller must select this operation explicitly.
    pub fn repair<P: AsRef<Path>>(path: P) -> Result<DiskRepairReport, StorageError> {
        let path = fs::canonicalize(path)?;
        if !vhd_footer_is_repairable(&path)? {
            return Err(StorageError::InvalidImage(
                "no supported, safe repair is available".to_string(),
            ));
        }

        let lock = Self::acquire_lock(&path, DiskFormat::Vhd)?;
        let result = repair_vhd_footer_checksum(&path);
        drop(lock);
        result.map(|_| DiskRepairReport {
            path,
            format: DiskFormat::Vhd,
            actions: vec!["recomputed fixed VHD footer checksum".to_string()],
        })
    }

    /// Path used for the ownership marker of a writable VM attachment.
    pub fn lock_path<P: AsRef<Path>>(path: P) -> PathBuf {
        let path = path.as_ref();
        let image_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        PathBuf::from(format!("{}.ghostos.lock", image_path.display()))
    }

    /// Pre-rename SynOS lock path, still accepted for inspect/recover.
    fn legacy_lock_path<P: AsRef<Path>>(path: P) -> PathBuf {
        let path = path.as_ref();
        let image_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        PathBuf::from(format!("{}.synos.lock", image_path.display()))
    }

    fn resolved_lock_path<P: AsRef<Path>>(path: P) -> PathBuf {
        let current = Self::lock_path(&path);
        if current.exists() {
            return current;
        }
        let legacy = Self::legacy_lock_path(&path);
        if legacy.exists() {
            return legacy;
        }
        current
    }

    /// Open an image for a VM and claim exclusive writable ownership.
    pub(crate) fn open_for_vm<P: AsRef<Path>>(
        path: P,
        writable: bool,
    ) -> Result<Self, StorageError> {
        let path = fs::canonicalize(path)?;
        let format = Self::open_with_access(&path, false)?.format();
        let lock = if writable {
            Some(Self::acquire_lock(&path, format)?)
        } else {
            None
        };
        match Self::open_with_access(&path, writable) {
            Ok(mut image) => {
                image.lock = lock;
                Ok(image)
            }
            Err(error) => {
                drop(lock);
                Err(error)
            }
        }
    }

    fn acquire_lock(path: &Path, format: DiskFormat) -> Result<DiskLock, StorageError> {
        let lock_path = Self::lock_path(path);
        let legacy_lock_path = Self::legacy_lock_path(path);
        if !lock_path.exists() && legacy_lock_path.exists() {
            let owner = fs::read_to_string(&legacy_lock_path)
                .unwrap_or_else(|_| "owner metadata unavailable".to_string());
            return Err(StorageError::Locked {
                path: legacy_lock_path.display().to_string(),
                owner,
            });
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut lock_file = match options.open(&lock_path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let owner = fs::read_to_string(&lock_path)
                    .unwrap_or_else(|_| "owner metadata unavailable".to_string());
                return Err(StorageError::Locked {
                    path: lock_path.display().to_string(),
                    owner,
                });
            }
            Err(error) => return Err(StorageError::Io(error)),
        };
        #[cfg(unix)]
        if let Err(error) = lock_file_exclusive(&lock_file, true) {
            drop(lock_file);
            let _ = fs::remove_file(&lock_path);
            let _ = sync_parent_directory(&lock_path);
            return Err(StorageError::Io(error));
        }
        let lock_token = new_lock_token();
        let publication = writeln!(
            lock_file,
            "version={}\nimage_identity={}\nowner_identity={}\npid={}\nstart_time={}\nhost_identity={}\nformat={}\nlock_token={}",
            LOCK_RECORD_VERSION,
            path.display(),
            owner_identity(),
            std::process::id(),
            process_start_time(std::process::id()).unwrap_or_else(|| "unknown".to_string()),
            host_identity(),
            format_name(format),
            lock_token,
        )
        .and_then(|_| lock_file.sync_all())
        .and_then(|_| sync_parent_directory(&lock_path));
        if let Err(error) = publication {
            drop(lock_file);
            let _ = fs::remove_file(&lock_path);
            let _ = sync_parent_directory(&lock_path);
            return Err(StorageError::Io(error));
        }
        Ok(DiskLock {
            path: lock_path,
            token: lock_token,
            file: Some(lock_file),
        })
    }

    /// Remove an ownership marker after stale-state validation.
    ///
    /// Keep this helper private so every public recovery request must pass
    /// through `recover_stale_lock`.
    fn recover_lock<P: AsRef<Path>>(path: P, expected_owner: &str) -> Result<(), StorageError> {
        let lock_path = Self::resolved_lock_path(path);
        let mut lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)?;
        let mut owner = String::new();
        lock_file.read_to_string(&mut owner)?;
        if owner != expected_owner {
            return Err(StorageError::Locked {
                path: lock_path.display().to_string(),
                owner,
            });
        }
        #[cfg(unix)]
        lock_file_exclusive(&lock_file, true).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                StorageError::Locked {
                    path: lock_path.display().to_string(),
                    owner: expected_owner.to_string(),
                }
            } else {
                StorageError::Io(error)
            }
        })?;
        drop(lock_file);
        fs::remove_file(&lock_path)?;
        sync_parent_directory(&lock_path)?;
        Ok(())
    }

    /// Read ownership metadata without changing the lock.
    pub fn inspect_lock<P: AsRef<Path>>(
        path: P,
    ) -> Result<Option<DiskLockInfo>, StorageError> {
        let image_path = path.as_ref();
        let lock_path = Self::resolved_lock_path(image_path);
        if !lock_path.exists() {
            return Ok(None);
        }
        let owner = fs::read_to_string(&lock_path)?;
        let version = lock_field(&owner, "version").and_then(|value| value.parse::<u32>().ok());
        let owner_identity = lock_field(&owner, "owner_identity").map(str::to_string);
        let pid = lock_field(&owner, "pid").and_then(|value| value.parse::<u32>().ok());
        let start_time = lock_field(&owner, "start_time")
            .filter(|value| *value != "unknown")
            .map(str::to_string);
        let stored_host_identity = lock_field(&owner, "host_identity").map(str::to_string);
        let stored_image = lock_field(&owner, "image_identity")
            .or_else(|| lock_field(&owner, "image"))
            .map(PathBuf::from);
        let format = lock_field(&owner, "format").and_then(parse_format);

        let metadata_complete = version == Some(LOCK_RECORD_VERSION)
            && owner_identity.is_some()
            && pid.is_some()
            && start_time.is_some()
            && stored_host_identity.is_some()
            && stored_image.is_some()
            && format.is_some();
        let image_matches = fs::canonicalize(image_path)
            .ok()
            .zip(stored_image.as_ref())
            .is_some_and(|(actual, stored)| actual == *stored);
        let host_matches = stored_host_identity
            .as_deref()
            .is_some_and(|stored| stored == host_identity());
        let process_matches = pid.is_some_and(|pid| {
            if !process_exists(pid) {
                return false;
            }
            start_time.as_deref().map_or(true, |start| {
                process_start_time(pid).is_some_and(|actual| actual == start)
            })
        });
        let format_mismatch = format
            .zip(Self::open_with_access(image_path, false).ok().map(|image| image.format()))
            .is_some_and(|(stored, actual)| stored != actual);
        let stale = !process_matches
            || (metadata_complete && (!image_matches || !host_matches || format_mismatch));

        Ok(Some(DiskLockInfo {
            path: lock_path,
            owner,
            pid,
            owner_identity,
            start_time,
            host_identity: stored_host_identity,
            image_path: stored_image,
            format,
            stale,
        }))
    }

    /// Remove an ownership marker only when its recorded process is gone.
    pub fn recover_stale_lock<P: AsRef<Path>>(path: P) -> Result<DiskLockInfo, StorageError> {
        let info = Self::inspect_lock(&path)?.ok_or_else(|| {
            StorageError::InvalidImage("disk has no ownership lock".to_string())
        })?;
        if !info.stale {
            return Err(StorageError::Locked {
                path: info.path.display().to_string(),
                owner: info.owner.clone(),
            });
        }
        Self::recover_lock(path, &info.owner)?;
        Ok(info)
    }

    /// Parse and validate a fixed-size VHD footer.
    fn open_vhd(mut file: File, writable: bool) -> Result<VhdFile, StorageError> {
        let file_len = file.metadata()?.len();
        if file_len < 1024 {
            return Err(StorageError::InvalidImage(
                "VHD image is truncated".to_string(),
            ));
        }
        file.seek(SeekFrom::End(-512))?;
        let mut footer = [0u8; 512];
        file.read_exact(&mut footer)?;

        // Fixed VHD footer layout (all multi-byte fields are big-endian):
        //   36..39 original size, 40..43 current size, 44..47 disk geometry,
        //   48..51 disk type (2 = fixed), 52..55 checksum.
        let original_size = read_be_u32(&footer, 36, "VHD original size")?;
        let current_size = read_be_u32(&footer, 40, "VHD current size")?;
        let disk_type = read_be_u32(&footer, 48, "VHD disk type")?;
        let stored_checksum = read_be_u32(&footer, 52, "VHD checksum")?;

        if disk_type != 2 {
            return Err(StorageError::Unsupported(format!(
                "VHD disk type {disk_type} (only fixed is supported)"
            )));
        }
        if original_size != current_size {
            return Err(StorageError::InvalidImage(
                "VHD original/current size mismatch".to_string(),
            ));
        }
        let disk_size = current_size as u64;
        validate_sector_capacity(disk_size, "VHD")?;
        let expected_file_len = disk_size.checked_add(512).ok_or_else(|| {
            StorageError::InvalidImage("VHD image size overflows host limits".to_string())
        })?;
        if file_len != expected_file_len {
            return Err(StorageError::InvalidImage(format!(
                "VHD file is {} bytes, footer declares {} bytes of disk data",
                file_len, disk_size
            )));
        }

        // Checksum is the one's-complement of the sum of all footer bytes
        // with the checksum field zeroed.
        let sum: u32 = footer
            .iter()
            .enumerate()
            .filter(|(i, _)| !(52..56).contains(i))
            .map(|(_, b)| *b as u32)
            .sum();
        let checksum_ok = !sum == stored_checksum;
        if !checksum_ok {
            return Err(StorageError::InvalidImage(
                "VHD footer checksum mismatch".to_string(),
            ));
        }

        file.seek(SeekFrom::Start(0))?;
        Ok(VhdFile {
            file,
            disk_size,
            writable,
        })
    }

    /// Parse and validate a QCOW2 header (versions 2 and 3).
    fn open_qcow2(mut file: File, writable: bool) -> Result<QcowFile, StorageError> {
        let file_len = file.metadata()?.len();
        if file_len < 104 {
            return Err(StorageError::InvalidImage(
                "QCOW2 header is truncated".to_string(),
            ));
        }
        file.seek(SeekFrom::Start(0))?;
        let mut hdr = [0u8; 104];
        file.read_exact(&mut hdr)?;

        let u32_at = |o: usize| {
            let bytes = hdr
                .get(o..o + 4)
                .ok_or_else(|| StorageError::InvalidImage("QCOW2 header field is truncated".to_string()))?;
            let bytes: [u8; 4] = bytes
                .try_into()
                .map_err(|_| StorageError::InvalidImage("QCOW2 header field is truncated".to_string()))?;
            Ok::<u32, StorageError>(u32::from_be_bytes(bytes))
        };
        let u64_at = |o: usize| {
            let bytes = hdr
                .get(o..o + 8)
                .ok_or_else(|| StorageError::InvalidImage("QCOW2 header field is truncated".to_string()))?;
            let bytes: [u8; 8] = bytes
                .try_into()
                .map_err(|_| StorageError::InvalidImage("QCOW2 header field is truncated".to_string()))?;
            Ok::<u64, StorageError>(u64::from_be_bytes(bytes))
        };

        let version = u32_at(4)?;
        if version != 2 && version != 3 {
            return Err(StorageError::Unsupported(format!(
                "QCOW version {version} (only 2 and 3 are supported)"
            )));
        }
        let backing_file_offset = u64_at(8)?;
        let backing_file_size = u32_at(16)?;
        if backing_file_offset != 0 || backing_file_size != 0 {
            return Err(StorageError::Unsupported(
                "QCOW2 backing files are not supported".to_string(),
            ));
        }
        if u32_at(32)? != 0 {
            return Err(StorageError::Unsupported(
                "QCOW2 encryption is not supported".to_string(),
            ));
        }
        let cluster_bits = u32_at(20)?;
        if !(9..=21).contains(&cluster_bits) {
            return Err(StorageError::InvalidImage(format!(
                "invalid QCOW2 cluster_bits {cluster_bits}"
            )));
        }
        // Incompatible features: bit 0 = dirty bit, bit 1 = corrupt bit.
        // We do not perform journal recovery, so refuse damaged images to
        // avoid silent corruption.
        let incompatible = u64_at(72)?;
        if incompatible != 0 {
            return Err(StorageError::Unsupported(format!(
                "QCOW2 incompatible features 0x{incompatible:x} (dirty/corrupt images unsupported)"
            )));
        }
        // L1 table size (number of 8-byte entries) at offset 36; the L1
        // table itself at offset 40.
        let l1_size = u32_at(36)?;
        if l1_size == 0 || l1_size > (1 << 22) {
            return Err(StorageError::InvalidImage(
                "invalid QCOW2 L1 size".to_string(),
            ));
        }

        let cluster_size = 1u64 << cluster_bits;
        let disk_size = u64_at(24)?;
        validate_sector_capacity(disk_size, "QCOW2")?;
        if version == 3 {
            let header_length = u32_at(100)? as u64;
            if header_length < 104 || header_length > file_len {
                return Err(StorageError::InvalidImage(
                    "invalid QCOW2 header length".to_string(),
                ));
            }
        }
        let mut l2_bits = 0u32;
        while (1u64 << l2_bits) < cluster_size / 8 {
            l2_bits += 1;
        }
        let l2_entries = 1u32 << l2_bits;
        let coverage = cluster_size.checked_mul(l2_entries as u64).ok_or_else(|| {
            StorageError::InvalidImage("QCOW2 L1 coverage overflows".to_string())
        })?;

        let l1_table_offset = u64_at(40)?;
        if l1_table_offset % cluster_size != 0 {
            return Err(StorageError::InvalidImage(
                "QCOW2 L1 table is not cluster-aligned".to_string(),
            ));
        }
        let l1_bytes = (l1_size as u64).checked_mul(8).ok_or_else(|| {
            StorageError::InvalidImage("QCOW2 L1 table size overflows".to_string())
        })?;
        let l1_end = l1_table_offset.checked_add(l1_bytes).ok_or_else(|| {
            StorageError::InvalidImage("QCOW2 L1 table range overflows".to_string())
        })?;
        if l1_table_offset < cluster_size || l1_end > file_len {
            return Err(StorageError::InvalidImage(
                "QCOW2 L1 table is outside the image".to_string(),
            ));
        }
        if coverage
            .checked_mul(l1_size as u64)
            .map_or(true, |value| value < disk_size)
        {
            return Err(StorageError::InvalidImage(
                "QCOW2 L1 table does not cover the virtual disk".to_string(),
            ));
        }
        let mut l1_cache = vec![0u64; l1_size as usize];
        file.seek(SeekFrom::Start(l1_table_offset))?;
        for slot in l1_cache.iter_mut() {
            let mut b = [0u8; 8];
            file.read_exact(&mut b)?;
            *slot = u64::from_be_bytes(b);
        }

        for entry in &l1_cache {
            let offset = qcow_cluster_offset(*entry);
            if *entry != 0 && offset == 0 {
                return Err(StorageError::InvalidImage(
                    "QCOW2 L1 entry has no table offset".to_string(),
                ));
            }
            if offset != 0
                && (offset % cluster_size != 0
                    || offset
                        .checked_add(cluster_size)
                        .map_or(true, |end| end > file_len))
            {
                return Err(StorageError::InvalidImage(
                    "QCOW2 L2 table is outside the image".to_string(),
                ));
            }
            if offset != 0 {
                validate_qcow_l2_table(
                    &mut file,
                    offset,
                    cluster_size,
                    l2_entries,
                    file_len,
                )?;
            }
        }

        Ok(QcowFile {
            file,
            header: QcowHeader {
                cluster_bits,
                disk_size,
                l1_table_offset,
                l1_size,
                l2_bits,
            },
            cluster_size,
            l2_entries,
            l1_cache,
            l2_cache_offset: 0,
            l2_cache: Vec::new(),
            writable,
        })
    }

    pub fn format(&self) -> DiskFormat {
        self.format
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn sector_count(&self) -> u64 {
        self.size.div_ceil(SECTOR_SIZE)
    }

    pub fn writable(&self) -> bool {
        self.writable
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Flush buffered writes to the host file.
    pub fn flush(&mut self) -> Result<(), StorageError> {
        match (self.raw.as_mut(), self.vhd.as_mut(), self.qcow.as_mut()) {
            (Some(file), _, _) => file.flush()?,
            (_, Some(vhd), _) => vhd.file.flush()?,
            (_, _, Some(qcow)) => qcow.file.flush()?,
            _ => return Err(StorageError::InvalidImage("no backing store".into())),
        }
        Ok(())
    }

    /// Flush and request durable host storage for this image.
    pub fn sync(&mut self) -> Result<(), StorageError> {
        self.flush()?;
        match (self.raw.as_mut(), self.vhd.as_mut(), self.qcow.as_mut()) {
            (Some(file), _, _) => file.sync_all()?,
            (_, Some(vhd), _) => vhd.file.sync_all()?,
            (_, _, Some(qcow)) => qcow.file.sync_all()?,
            _ => return Err(StorageError::InvalidImage("no backing store".into())),
        }
        Ok(())
    }

    /// Flush and close the image, returning any host I/O error.
    pub fn close(mut self) -> Result<(), StorageError> {
        self.sync()
    }

    pub(crate) fn set_cleanup_path(&mut self, path: PathBuf) {
        self.cleanup_path = Some(path);
    }

    /// Read one 512-byte logical sector at `lba`.
    pub fn read_sector(&mut self, lba: u64, buf: &mut [u8; 512]) -> Result<(), StorageError> {
        self.bounds_check(lba)?;
        match (self.raw.as_mut(), self.vhd.as_mut(), self.qcow.as_mut()) {
            (Some(f), _, _) => {
                f.seek(SeekFrom::Start(lba * SECTOR_SIZE))?;
            }
            (_, Some(v), _) => {
                v.file.seek(SeekFrom::Start(lba * SECTOR_SIZE))?;
            }
            (_, _, Some(q)) => return q.read_range(lba * SECTOR_SIZE, buf),
            _ => return Err(StorageError::InvalidImage("no backing store".into())),
        }
        match self.raw.as_mut() {
            Some(f) => f.read_exact(buf)?,
            None => match self.vhd.as_mut() {
                Some(v) => v.file.read_exact(buf)?,
                None => return Err(StorageError::InvalidImage("no backing store".into())),
            },
        }
        Ok(())
    }

    /// Write one 512-byte logical sector at `lba`.
    pub fn write_sector(&mut self, lba: u64, buf: &[u8; 512]) -> Result<(), StorageError> {
        if !self.writable {
            return Err(StorageError::ReadOnly);
        }
        self.bounds_check(lba)?;
        match (self.raw.as_mut(), self.vhd.as_mut(), self.qcow.as_mut()) {
            (Some(f), _, _) => {
                f.seek(SeekFrom::Start(lba * SECTOR_SIZE))?;
                f.write_all(buf)?;
                f.flush()?;
            }
            (_, Some(v), _) => {
                if !v.writable {
                    return Err(StorageError::ReadOnly);
                }
                v.file.seek(SeekFrom::Start(lba * SECTOR_SIZE))?;
                v.file.write_all(buf)?;
                v.file.flush()?;
            }
            (_, _, Some(q)) => q.write_range(lba * SECTOR_SIZE, buf)?,
            _ => return Err(StorageError::InvalidImage("no backing store".into())),
        }
        Ok(())
    }

    fn bounds_check(&self, lba: u64) -> Result<(), StorageError> {
        let end = lba
            .checked_mul(SECTOR_SIZE)
            .and_then(|offset| offset.checked_add(SECTOR_SIZE))
            .ok_or(StorageError::OutOfRange)?;
        if end > self.size {
            return Err(StorageError::OutOfRange);
        }
        Ok(())
    }
}

fn process_exists(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn sync_parent_directory(path: &Path) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    #[cfg(unix)]
    {
        File::open(parent)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        Ok(())
    }
}

fn lock_field<'a>(record: &'a str, key: &str) -> Option<&'a str> {
    record.lines().find_map(|line| {
        let (field, value) = line.split_once('=')?;
        (field == key).then_some(value.trim())
    })
}

fn new_lock_token() -> String {
    let sequence = LOCK_TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{sequence}", std::process::id())
}

#[cfg(unix)]
fn lock_file_exclusive(file: &File, nonblocking: bool) -> std::io::Result<()> {
    let mut operation = libc::LOCK_EX;
    if nonblocking {
        operation |= libc::LOCK_NB;
    }
    if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn parse_format(value: &str) -> Option<DiskFormat> {
    match value {
        "raw" => Some(DiskFormat::Raw),
        "vhd" => Some(DiskFormat::Vhd),
        "qcow2" => Some(DiskFormat::Qcow2),
        _ => None,
    }
}

fn format_name(format: DiskFormat) -> &'static str {
    match format {
        DiskFormat::Raw => "raw",
        DiskFormat::Vhd => "vhd",
        DiskFormat::Qcow2 => "qcow2",
    }
}

fn owner_identity() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn host_identity() -> String {
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            fs::read_to_string("/etc/hostname")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let machine_id = fs::read_to_string("/etc/machine-id")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    format!("hostname={hostname};machine={machine_id}")
}

fn process_start_time(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let fields = stat.rsplit_once(") ")?.1.split_whitespace().collect::<Vec<_>>();
        let start_ticks = fields.get(19)?;
        let proc_stat = fs::read_to_string("/proc/stat").ok()?;
        let boot_time = proc_stat
            .lines()
            .find_map(|line| line.strip_prefix("btime "))?
            .trim();
        return Some(format!("linux:{boot_time}:{start_ticks}"));
    }

    #[cfg(not(target_os = "linux"))]
    {
        let output = Command::new("ps")
            .args(["-o", "lstart=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let start = String::from_utf8(output.stdout).ok()?.trim().to_string();
        (!start.is_empty()).then_some(start)
    }
}

fn validate_sector_capacity(size: u64, format: &str) -> Result<(), StorageError> {
    if size == 0 || size % SECTOR_SIZE != 0 {
        return Err(StorageError::InvalidImage(format!(
            "{format} capacity {size} is not a non-zero sector multiple"
        )));
    }
    Ok(())
}

fn detect_format(file: &mut File, file_len: u64, magic: [u8; 8]) -> Result<DiskFormat, StorageError> {
    if magic.starts_with(QCOW2_MAGIC) {
        return Ok(DiskFormat::Qcow2);
    }
    if &magic[..8] == VHD_MAGIC {
        return Ok(DiskFormat::Vhd);
    }
    if file_len >= 512 {
        let mut footer = [0u8; 8];
        file.seek(SeekFrom::End(-512))?;
        file.read_exact(&mut footer)?;
        file.seek(SeekFrom::Start(0))?;
        if &footer[..8] == VHD_MAGIC {
            return Ok(DiskFormat::Vhd);
        }
    }
    Ok(DiskFormat::Raw)
}

fn vhd_footer_is_repairable<P: AsRef<Path>>(path: P) -> Result<bool, StorageError> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    if file_len < 1024 {
        return Ok(false);
    }
    file.seek(SeekFrom::End(-512))?;
    let mut footer = [0u8; 512];
    file.read_exact(&mut footer)?;
    if &footer[..8] != VHD_MAGIC {
        return Ok(false);
    }

    let disk_size = read_be_u32(&footer, 40, "VHD current size")? as u64;
    let original_size = read_be_u32(&footer, 36, "VHD original size")? as u64;
    let disk_type = read_be_u32(&footer, 48, "VHD disk type")?;
    if disk_type != 2 || original_size != disk_size {
        return Ok(false);
    }
    validate_sector_capacity(disk_size, "VHD")?;
    if disk_size.checked_add(512) != Some(file_len) {
        return Ok(false);
    }
    let stored_checksum = read_be_u32(&footer, 52, "VHD checksum")?;
    let sum: u32 = footer
        .iter()
        .enumerate()
        .filter(|(index, _)| !(52..56).contains(index))
        .map(|(_, value)| *value as u32)
        .sum();
    Ok(!sum != stored_checksum)
}

fn read_be_u32(bytes: &[u8], offset: usize, field: &str) -> Result<u32, StorageError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| StorageError::InvalidImage(format!("{field} is truncated")))?;
    let value: [u8; 4] = value
        .try_into()
        .map_err(|_| StorageError::InvalidImage(format!("{field} is truncated")))?;
    Ok(u32::from_be_bytes(value))
}

fn repair_vhd_footer_checksum<P: AsRef<Path>>(path: P) -> Result<(), StorageError> {
    let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
    file.seek(SeekFrom::End(-512))?;
    let mut footer = [0u8; 512];
    file.read_exact(&mut footer)?;
    let sum: u32 = footer
        .iter()
        .enumerate()
        .filter(|(index, _)| !(52..56).contains(index))
        .map(|(_, value)| *value as u32)
        .sum();
    footer[52..56].copy_from_slice(&(!sum).to_be_bytes());
    file.seek(SeekFrom::End(-512))?;
    file.write_all(&footer)?;
    file.sync_all()?;
    sync_parent_directory(path.as_ref())?;
    Ok(())
}

fn qcow_cluster_offset(entry: u64) -> u64 {
    entry & QCOW_CLUSTER_OFFSET_MASK
}

fn validate_qcow_l2_table(
    file: &mut File,
    l2_phys: u64,
    cluster_size: u64,
    l2_entries: u32,
    file_len: u64,
) -> Result<(), StorageError> {
    file.seek(SeekFrom::Start(l2_phys))?;
    for _ in 0..l2_entries {
        let mut bytes = [0u8; 8];
        file.read_exact(&mut bytes)?;
        let entry = u64::from_be_bytes(bytes);
        let offset = qcow_cluster_offset(entry);
        if entry & (1 << 62) != 0 {
            return Err(StorageError::Unsupported(
                "QCOW2 compressed clusters are not supported".to_string(),
            ));
        }
        if offset != 0
            && (offset % cluster_size != 0
                || offset
                    .checked_add(cluster_size)
                    .map_or(true, |end| end > file_len))
        {
            return Err(StorageError::InvalidImage(
                "QCOW2 data cluster is outside the image".to_string(),
            ));
        }
    }
    Ok(())
}

impl QcowFile {
    /// Read `buf.len()` bytes starting at file offset `offset`.
    fn read_range(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), StorageError> {
        let mut done = 0usize;
        while done < buf.len() {
            let cur = offset + done as u64;
            let cluster_idx = cur >> self.header.cluster_bits;
            let in_cluster = (cur & (self.cluster_size - 1)) as usize;
            let chunk = (buf.len() - done).min(self.cluster_size as usize - in_cluster);

            let phys = self.cluster_physical(cluster_idx)?;
            if let Some(p) = phys {
                self.file.seek(SeekFrom::Start(p + in_cluster as u64))?;
                self.file.read_exact(&mut buf[done..done + chunk])?;
            } else {
                // Unallocated clusters read as zeroes.
                buf[done..done + chunk].fill(0);
            }
            done += chunk;
        }
        Ok(())
    }

    /// Write `buf.len()` bytes starting at file offset `offset`, allocating
    /// clusters copy-on-write as needed.
    fn write_range(&mut self, offset: u64, buf: &[u8]) -> Result<(), StorageError> {
        if !self.writable {
            return Err(StorageError::ReadOnly);
        }
        let mut done = 0usize;
        while done < buf.len() {
            let cur = offset + done as u64;
            let cluster_idx = cur >> self.header.cluster_bits;
            let in_cluster = (cur & (self.cluster_size - 1)) as usize;
            let chunk = (buf.len() - done).min(self.cluster_size as usize - in_cluster);

            let phys = self.allocate_cluster(cluster_idx)?;
            self.file.seek(SeekFrom::Start(phys + in_cluster as u64))?;
            self.file.write_all(&buf[done..done + chunk])?;
            done += chunk;
        }
        self.file.flush()?;
        Ok(())
    }

    /// Resolve `cluster_idx`'s physical file offset, loading the L2 table
    /// when needed. Returns `None` for an unallocated (all-zero) cluster.
    fn cluster_physical(&mut self, cluster_idx: u64) -> Result<Option<u64>, StorageError> {
        let l1_index = (cluster_idx >> self.header.l2_bits) as usize;
        if l1_index >= self.header.l1_size as usize {
            return Err(StorageError::OutOfRange);
        }
        let l1_entry = self.l1_cache[l1_index];
        if l1_entry == 0 {
            return Ok(None);
        }
        let l2_phys = qcow_cluster_offset(l1_entry);
        if l2_phys == 0 {
            return Err(StorageError::InvalidImage(
                "QCOW2 L1 entry has no table offset".to_string(),
            ));
        }
        self.load_l2(l2_phys)?;

        let l2_index = (cluster_idx & (self.l2_entries as u64 - 1)) as usize;
        let entry = self.l2_cache[l2_index];
        let offset = qcow_cluster_offset(entry);
        if offset == 0 {
            // A zero host offset means the virtual cluster is unallocated.
            return Ok(None);
        }
        if entry & (1 << 62) != 0 {
            return Err(StorageError::Unsupported(
                "QCOW2 compressed clusters are not supported".to_string(),
            ));
        }
        Ok(Some(offset))
    }

    /// Allocate (copy-on-write) the cluster and return its file offset.
    fn allocate_cluster(&mut self, cluster_idx: u64) -> Result<u64, StorageError> {
        let l1_index = (cluster_idx >> self.header.l2_bits) as usize;
        if l1_index >= self.header.l1_size as usize {
            return Err(StorageError::OutOfRange);
        }

        // Fast path: L2 already loaded and entry allocated.
        if self.l2_cache_offset != 0 {
            let l2_index = (cluster_idx & (self.l2_entries as u64 - 1)) as usize;
            if l2_index < self.l2_cache.len() {
                let entry = self.l2_cache[l2_index];
                let offset = qcow_cluster_offset(entry);
                if offset != 0 {
                    return Ok(offset);
                }
            }
        }

        let l2_phys = self.ensure_l2(l1_index)?;
        let l2_index = (cluster_idx & (self.l2_entries as u64 - 1)) as usize;

        let cluster_phys = {
            self.file.seek(SeekFrom::End(0))?;
            let end = self.file.stream_position()?;
            let aligned = (end + self.cluster_size - 1) & !(self.cluster_size - 1);
            self.file.set_len(aligned + self.cluster_size)?;
            aligned
        };

        let entry = cluster_phys | 1;
        let entry_bytes = entry.to_be_bytes();
        self.file
            .seek(SeekFrom::Start(l2_phys + (l2_index as u64) * 8))?;
        self.file.write_all(&entry_bytes)?;
        if l2_index < self.l2_cache.len() {
            self.l2_cache[l2_index] = entry;
        }
        Ok(cluster_phys)
    }

    /// Ensure the L1 entry for `l1_index` points at an allocated, zeroed L2
    /// table and load that table into the cache.
    fn ensure_l2(&mut self, l1_index: usize) -> Result<u64, StorageError> {
        let existing = self.l1_cache[l1_index];
        if qcow_cluster_offset(existing) != 0 {
            let l2_phys = qcow_cluster_offset(existing);
            self.load_l2(l2_phys)?;
            return Ok(l2_phys);
        }

        let l2_phys = {
            self.file.seek(SeekFrom::End(0))?;
            let end = self.file.stream_position()?;
            let aligned = (end + self.cluster_size - 1) & !(self.cluster_size - 1);
            self.file.set_len(aligned + self.cluster_size)?;
            aligned
        };

        // Zero the L2 table (set_len may leave garbage).
        self.file.seek(SeekFrom::Start(l2_phys))?;
        let zeros = vec![0u8; self.cluster_size as usize];
        self.file.write_all(&zeros)?;

        let entry = l2_phys | 1;
        let entry_bytes = entry.to_be_bytes();
        self.file.seek(SeekFrom::Start(
            self.header.l1_table_offset + (l1_index as u64) * 8,
        ))?;
        self.file.write_all(&entry_bytes)?;
        self.l1_cache[l1_index] = entry;

        self.l2_cache_offset = l2_phys;
        self.l2_cache = vec![0; self.l2_entries as usize];
        Ok(l2_phys)
    }

    /// Load the L2 table at `l2_phys` into the cache (if not already cached).
    fn load_l2(&mut self, l2_phys: u64) -> Result<(), StorageError> {
        if self.l2_cache_offset == l2_phys && !self.l2_cache.is_empty() {
            return Ok(());
        }
        let mut entries = Vec::with_capacity(self.l2_entries as usize);
        self.file.seek(SeekFrom::Start(l2_phys))?;
        for _ in 0..self.l2_entries {
            let mut b = [0u8; 8];
            self.file.read_exact(&mut b)?;
            entries.push(u64::from_be_bytes(b));
        }
        let file_len = self.file.metadata()?.len();
        for entry in &entries {
            let offset = qcow_cluster_offset(*entry);
            if *entry & (1 << 62) != 0 {
                return Err(StorageError::Unsupported(
                    "QCOW2 compressed clusters are not supported".to_string(),
                ));
            }
            if offset != 0
                && (offset % self.cluster_size != 0
                    || offset
                        .checked_add(self.cluster_size)
                        .map_or(true, |end| end > file_len))
            {
                return Err(StorageError::InvalidImage(
                    "QCOW2 data cluster is outside the image".to_string(),
                ));
            }
        }
        self.l2_cache_offset = l2_phys;
        self.l2_cache = entries;
        Ok(())
    }
}

impl std::fmt::Debug for DiskImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskImage")
            .field("format", &self.format)
            .field("size", &self.size)
            .field("writable", &self.writable)
            .field("filename", &self.filename)
            .finish_non_exhaustive()
    }
}

impl Drop for DiskImage {
    fn drop(&mut self) {
        let _ = self.flush();
        if let Some(path) = self.cleanup_path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static LOCK_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn lock_test_path(label: &str) -> PathBuf {
        let counter = LOCK_TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ghostos-vm-lock-{label}-{}-{counter}.raw",
            std::process::id()
        ))
    }

    fn create_lock_test_image(path: &Path) {
        File::create(path).unwrap().set_len(4096).unwrap();
    }

    fn remove_lock_test_image(path: &Path) {
        let _ = fs::remove_file(DiskImage::lock_path(path));
        let _ = fs::remove_file(path);
    }

    fn write_lock_test_record(path: &Path, pid: u32, start_time: &str) {
        let image = fs::canonicalize(path).unwrap();
        let mut lock = File::create(DiskImage::lock_path(path)).unwrap();
        writeln!(
            lock,
            "version={LOCK_RECORD_VERSION}\nimage_identity={}\nowner_identity=test\npid={pid}\nstart_time={start_time}\nhost_identity={}\nformat=raw",
            image.display(),
            host_identity(),
        )
        .unwrap();
        lock.sync_all().unwrap();
    }

    fn write_vhd_fixture(path: &Path, disk_size: u64, disk_type: u32, valid_checksum: bool) {
        let mut file = File::create(path).unwrap();
        file.set_len(disk_size + 512).unwrap();
        let mut footer = [0u8; 512];
        footer[0..8].copy_from_slice(VHD_MAGIC);
        footer[12..16].copy_from_slice(&1u32.to_be_bytes());
        footer[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        footer[36..40].copy_from_slice(&(disk_size as u32).to_be_bytes());
        footer[40..44].copy_from_slice(&(disk_size as u32).to_be_bytes());
        footer[48..52].copy_from_slice(&disk_type.to_be_bytes());
        let sum: u32 = footer
            .iter()
            .enumerate()
            .filter(|(index, _)| !(52..56).contains(index))
            .map(|(_, value)| *value as u32)
            .sum();
        footer[52..56].copy_from_slice(&(if valid_checksum { !sum } else { 0 }).to_be_bytes());
        file.seek(SeekFrom::End(-512)).unwrap();
        file.write_all(&footer).unwrap();
        file.sync_all().unwrap();
    }

    fn write_qcow2_fixture(
        path: &Path,
        disk_size: u64,
        l1_size: u32,
        incompatible_features: u64,
    ) {
        let cluster_size = 64 * 1024u64;
        let l1_offset = cluster_size;
        let l2_offset = cluster_size * 2;
        let mut file = File::create(path).unwrap();
        file.set_len(cluster_size * 3).unwrap();
        let mut header = [0u8; 104];
        header[0..4].copy_from_slice(QCOW2_MAGIC);
        header[4..8].copy_from_slice(&2u32.to_be_bytes());
        header[20..24].copy_from_slice(&16u32.to_be_bytes());
        header[24..32].copy_from_slice(&disk_size.to_be_bytes());
        header[36..40].copy_from_slice(&l1_size.to_be_bytes());
        header[40..48].copy_from_slice(&l1_offset.to_be_bytes());
        header[72..80].copy_from_slice(&incompatible_features.to_be_bytes());
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(&header).unwrap();
        if l1_size == 1 {
            file.seek(SeekFrom::Start(l1_offset)).unwrap();
            file.write_all(&l2_offset.to_be_bytes()).unwrap();
        }
        file.sync_all().unwrap();
    }

    #[test]
    fn writable_open_rejects_concurrent_owner_and_releases_lock() {
        let path = lock_test_path("concurrent");
        create_lock_test_image(&path);

        let first = DiskImage::open_for_vm(&path, true).unwrap();
        let info = DiskImage::inspect_lock(&path).unwrap().unwrap();
        assert_eq!(info.image_path, Some(fs::canonicalize(&path).unwrap()));
        assert_eq!(info.format, Some(DiskFormat::Raw));
        assert!(info.owner_identity.is_some());
        assert!(info.start_time.is_some());
        assert!(info.host_identity.is_some());
        assert!(!info.stale);

        let error = DiskImage::open_for_vm(&path, true).unwrap_err();
        assert!(matches!(error, StorageError::Locked { .. }));

        drop(first);
        let second = DiskImage::open_for_vm(&path, true).unwrap();
        drop(second);
        remove_lock_test_image(&path);
    }

    #[test]
    fn crashed_owner_lock_is_recoverable() {
        let path = lock_test_path("crash");
        create_lock_test_image(&path);
        write_lock_test_record(&path, 0, "crashed-process");

        let info = DiskImage::inspect_lock(&path).unwrap().unwrap();
        assert!(info.stale);
        DiskImage::recover_stale_lock(&path).unwrap();
        assert!(DiskImage::inspect_lock(&path).unwrap().is_none());

        remove_lock_test_image(&path);
    }

    #[test]
    fn reused_pid_with_new_start_time_is_stale() {
        let path = lock_test_path("pid-reuse");
        create_lock_test_image(&path);
        write_lock_test_record(&path, std::process::id(), "old-process-start");

        let info = DiskImage::inspect_lock(&path).unwrap().unwrap();
        assert!(info.stale);
        DiskImage::recover_stale_lock(&path).unwrap();

        remove_lock_test_image(&path);
    }

    #[test]
    fn copied_image_lock_is_stale_for_new_image_identity() {
        let source = lock_test_path("copy-source");
        let copy = lock_test_path("copy-target");
        create_lock_test_image(&source);
        let owner = DiskImage::open_for_vm(&source, true).unwrap();
        create_lock_test_image(&copy);
        fs::copy(&source, &copy).unwrap();
        fs::copy(DiskImage::lock_path(&source), DiskImage::lock_path(&copy)).unwrap();
        drop(owner);

        let info = DiskImage::inspect_lock(&copy).unwrap().unwrap();
        assert!(info.stale);
        DiskImage::recover_stale_lock(&copy).unwrap();
        let copied_owner = DiskImage::open_for_vm(&copy, true).unwrap();
        drop(copied_owner);

        remove_lock_test_image(&source);
        remove_lock_test_image(&copy);
    }

    #[test]
    fn read_only_attachment_does_not_claim_writable_lock() {
        let path = lock_test_path("read-only");
        create_lock_test_image(&path);

        let read_only = DiskImage::open_for_vm(&path, false).unwrap();
        assert!(DiskImage::inspect_lock(&path).unwrap().is_none());
        let writable = DiskImage::open_for_vm(&path, true).unwrap();
        drop(writable);
        drop(read_only);

        remove_lock_test_image(&path);
    }

    #[test]
    fn independent_raw_sparse_fixture_flushes_and_reopens() {
        let path = lock_test_path("raw-independent");
        let sparse_size = 32 * 1024 * 1024u64;
        File::create(&path).unwrap().set_len(sparse_size).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), sparse_size);

        let mut image = DiskImage::open_with_access(&path, true).unwrap();
        assert_eq!(image.format(), DiskFormat::Raw);
        let mut zeroes = [0xA5u8; 512];
        image.read_sector(0, &mut zeroes).unwrap();
        assert_eq!(zeroes, [0u8; 512]);
        let sector = [0x5Au8; 512];
        image.write_sector(7, &sector).unwrap();
        image.flush().unwrap();
        image.sync().unwrap();
        drop(image);

        let mut reopened = DiskImage::open_with_access(&path, false).unwrap();
        let mut read = [0u8; 512];
        reopened.read_sector(7, &mut read).unwrap();
        assert_eq!(read, sector);
        assert!(matches!(
            reopened.write_sector(7, &sector),
            Err(StorageError::ReadOnly)
        ));
        drop(reopened);
        remove_lock_test_image(&path);
    }

    #[test]
    fn independent_fixed_vhd_fixture_handles_maximum_capacity_and_flush() {
        let path = lock_test_path("vhd-independent");
        let disk_size = u32::MAX as u64 - 511;
        write_vhd_fixture(&path, disk_size, 2, true);

        let mut image = DiskImage::open_with_access(&path, true).unwrap();
        assert_eq!(image.format(), DiskFormat::Vhd);
        assert_eq!(image.size(), disk_size);
        let sector = [0x3Cu8; 512];
        image.write_sector(image.sector_count() - 1, &sector).unwrap();
        image.flush().unwrap();
        image.sync().unwrap();
        drop(image);

        let mut reopened = DiskImage::open_with_access(&path, false).unwrap();
        let mut read = [0u8; 512];
        reopened
            .read_sector(reopened.sector_count() - 1, &mut read)
            .unwrap();
        assert_eq!(read, sector);
        drop(reopened);
        remove_lock_test_image(&path);
    }

    #[test]
    fn independent_qcow2_fixture_reads_zeroes_allocates_and_flushes() {
        let path = lock_test_path("qcow2-independent");
        let cluster_size = 64 * 1024u64;
        let disk_size = cluster_size * (cluster_size / 8);
        write_qcow2_fixture(&path, disk_size, 1, 0);

        let mut image = DiskImage::open_with_access(&path, true).unwrap();
        assert_eq!(image.format(), DiskFormat::Qcow2);
        assert_eq!(image.size(), disk_size);
        let mut zeroes = [0xA5u8; 512];
        image.read_sector(0, &mut zeroes).unwrap();
        assert_eq!(zeroes, [0u8; 512]);
        let sector = [0xC3u8; 512];
        image.write_sector(0, &sector).unwrap();
        image.flush().unwrap();
        image.sync().unwrap();
        drop(image);

        let mut reopened = DiskImage::open_with_access(&path, false).unwrap();
        let mut read = [0u8; 512];
        reopened.read_sector(0, &mut read).unwrap();
        assert_eq!(read, sector);
        drop(reopened);
        remove_lock_test_image(&path);
    }

    #[test]
    fn independent_fixtures_reject_malformed_metadata_and_size_limits() {
        let bad_vhd_checksum = lock_test_path("vhd-bad-checksum");
        write_vhd_fixture(&bad_vhd_checksum, 1024 * 1024, 2, false);
        assert!(matches!(
            DiskImage::open_with_access(&bad_vhd_checksum, false),
            Err(StorageError::InvalidImage(_))
        ));

        let bad_vhd_type = lock_test_path("vhd-bad-type");
        write_vhd_fixture(&bad_vhd_type, 1024 * 1024, 3, true);
        assert!(matches!(
            DiskImage::open_with_access(&bad_vhd_type, false),
            Err(StorageError::Unsupported(_))
        ));

        let bad_qcow_features = lock_test_path("qcow2-bad-features");
        write_qcow2_fixture(&bad_qcow_features, 512 * 1024 * 1024, 1, 1);
        assert!(matches!(
            DiskImage::open_with_access(&bad_qcow_features, false),
            Err(StorageError::Unsupported(_))
        ));

        let oversized_qcow_l1 = lock_test_path("qcow2-oversized-l1");
        write_qcow2_fixture(&oversized_qcow_l1, 512 * 1024 * 1024, (1 << 22) + 1, 0);
        assert!(matches!(
            DiskImage::open_with_access(&oversized_qcow_l1, false),
            Err(StorageError::InvalidImage(_))
        ));

        for path in [
            bad_vhd_checksum,
            bad_vhd_type,
            bad_qcow_features,
            oversized_qcow_l1,
        ] {
            remove_lock_test_image(&path);
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_aliases_share_one_canonical_lock() {
        let path = lock_test_path("symlink-target");
        let alias = lock_test_path("symlink-alias");
        create_lock_test_image(&path);
        std::os::unix::fs::symlink(&path, &alias).unwrap();

        let owner = DiskImage::open_for_vm(&alias, true).unwrap();
        assert_eq!(DiskImage::lock_path(&path), DiskImage::lock_path(&alias));
        assert!(!DiskImage::inspect_lock(&path).unwrap().unwrap().stale);
        let error = DiskImage::open_for_vm(&path, true).unwrap_err();
        assert!(matches!(error, StorageError::Locked { .. }));
        drop(owner);

        let _ = fs::remove_file(&alias);
        remove_lock_test_image(&path);
    }

    #[cfg(unix)]
    #[test]
    fn lock_permission_failure_is_reported() {
        let directory = lock_test_path("permission-directory");
        fs::create_dir(&directory).unwrap();
        let path = directory.join("disk.raw");
        create_lock_test_image(&path);

        let original_permissions = fs::metadata(&directory).unwrap().permissions();
        let mut blocked_permissions = original_permissions.clone();
        blocked_permissions.set_mode(0o555);
        fs::set_permissions(&directory, blocked_permissions).unwrap();
        let result = DiskImage::open_for_vm(&path, true);
        fs::set_permissions(&directory, original_permissions).unwrap();

        if result.is_ok() {
            // Root can bypass directory write permissions.
            drop(result.unwrap());
        } else {
            assert!(matches!(
                result,
                Err(StorageError::Io(error))
                    if error.kind() == std::io::ErrorKind::PermissionDenied
            ));
        }
        remove_lock_test_image(&path);
        fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn raw_image_round_trip() {
        let dir = std::env::temp_dir();
        let path = dir.join("ghostos_vm_raw_test.img");
        let f = File::create(&path).unwrap();
        f.set_len(1024 * 1024).unwrap();
        drop(f);

        let mut img = DiskImage::open(&path).unwrap();
        assert_eq!(img.format(), DiskFormat::Raw);
        assert_eq!(img.size(), 1024 * 1024);
        assert_eq!(img.sector_count(), 2048);

        let mut sector = [0u8; 512];
        sector[0] = 0xAA;
        sector[511] = 0x55;
        img.write_sector(3, &sector).unwrap();

        let mut read = [0u8; 512];
        img.read_sector(3, &mut read).unwrap();
        assert_eq!(read, sector);
        assert!(img.read_sector(2048, &mut read).is_err());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn raw_read_only() {
        let dir = std::env::temp_dir();
        let path = dir.join("ghostos_vm_raw_ro_test.img");
        {
            let f = File::create(&path).unwrap();
            f.set_len(4096).unwrap();
        }
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).unwrap();

        let mut img = DiskImage::open(&path).unwrap();
        assert!(!img.writable());
        let sector = [0u8; 512];
        assert!(matches!(
            img.write_sector(0, &sector),
            Err(StorageError::ReadOnly)
        ));

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).ok();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn vhd_fixed_footer_parse() {
        let dir = std::env::temp_dir();
        let path = dir.join("ghostos_vm_vhd_test.vhd");
        let mut data = vec![0u8; 1024 * 1024 + 512];
        let footer = &mut data[1024 * 1024..];
        footer[0..8].copy_from_slice(VHD_MAGIC);
        footer[12..16].copy_from_slice(&1u32.to_be_bytes()); // version 1.0
        footer[16..20].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // data offset (fixed)
        footer[36..40].copy_from_slice(&(1024 * 1024u32).to_be_bytes()); // original size
        footer[40..44].copy_from_slice(&(1024 * 1024u32).to_be_bytes()); // current size
        footer[48..52].copy_from_slice(&2u32.to_be_bytes()); // disk type = fixed
        let sum: u32 = footer
            .iter()
            .enumerate()
            .filter(|(i, _)| !(52..56).contains(i))
            .map(|(_, b)| *b as u32)
            .sum();
        footer[52..56].copy_from_slice(&(!sum).to_be_bytes());

        File::create(&path).unwrap().write_all(&data).unwrap();

        let mut img = DiskImage::open(&path).unwrap();
        assert_eq!(img.format(), DiskFormat::Vhd);
        assert_eq!(img.size(), 1024 * 1024);

        let sector = [0x42u8; 512];
        img.write_sector(1, &sector).unwrap();
        let mut read = [0u8; 512];
        img.read_sector(1, &mut read).unwrap();
        assert_eq!(read, sector);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn qcow2_minimal() {
        let dir = std::env::temp_dir();
        let path = dir.join("ghostos_vm_qcow2_test.qcow2");
        // Hand-craft a tiny QCOW2: 1 MiB disk, 64 KiB clusters, one L1 entry.
        let mut data = vec![0u8; 0x40000];
        let hdr = &mut data[0..104];
        hdr[0..4].copy_from_slice(QCOW2_MAGIC);
        hdr[4..8].copy_from_slice(&2u32.to_be_bytes()); // version 2
        hdr[20..24].copy_from_slice(&16u32.to_be_bytes()); // cluster_bits = 16 (64 KiB)
        hdr[24..32].copy_from_slice(&(1024u64 * 1024).to_be_bytes()); // disk size
        hdr[36..40].copy_from_slice(&1u32.to_be_bytes()); // l1_size = 1
        hdr[40..48].copy_from_slice(&0x20000u64.to_be_bytes()); // L1 table offset
        hdr[48..56].copy_from_slice(&0x30000u64.to_be_bytes()); // refcount table
        hdr[56..60].copy_from_slice(&1u32.to_be_bytes()); // refcount clusters
        // L1 entry at 0x20000 -> L2 at 0x10000. The low flag bits are clear,
        // as they are in standard QCOW2 images.
        data[0x20000..0x20008].copy_from_slice(&0x10000u64.to_be_bytes());

        File::create(&path).unwrap().write_all(&data).unwrap();

        let mut img = DiskImage::open(&path).unwrap();
        assert_eq!(img.format(), DiskFormat::Qcow2);
        assert_eq!(img.size(), 1024 * 1024);

        // Unallocated clusters read as zeroes.
        let mut read = [0xFFu8; 512];
        img.read_sector(0, &mut read).unwrap();
        assert_eq!(read, [0u8; 512]);

        // Writes allocate a new cluster at the end of the file.
        let write = [0xABu8; 512];
        img.write_sector(0, &write).unwrap();
        let mut read2 = [0u8; 512];
        img.read_sector(0, &mut read2).unwrap();
        assert_eq!(read2, write);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn raw_image_rejects_unaligned_capacity() {
        let path = std::env::temp_dir().join("ghostos_vm_raw_unaligned_test.img");
        File::create(&path).unwrap().set_len(513).unwrap();

        let error = DiskImage::open(&path).unwrap_err();
        assert!(matches!(error, StorageError::InvalidImage(_)));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn qcow2_rejects_l2_table_outside_image() {
        let path = std::env::temp_dir().join("ghostos_vm_qcow2_invalid_test.qcow2");
        let mut data = vec![0u8; 0x40000];
        data[0..4].copy_from_slice(QCOW2_MAGIC);
        data[4..8].copy_from_slice(&2u32.to_be_bytes());
        data[20..24].copy_from_slice(&16u32.to_be_bytes());
        data[24..32].copy_from_slice(&(1024u64 * 1024).to_be_bytes());
        data[36..40].copy_from_slice(&1u32.to_be_bytes());
        data[40..48].copy_from_slice(&0x20000u64.to_be_bytes());
        data[0x20000..0x20008].copy_from_slice(&0x40000u64.to_be_bytes());
        File::create(&path).unwrap().write_all(&data).unwrap();

        let error = DiskImage::open(&path).unwrap_err();
        assert!(matches!(error, StorageError::InvalidImage(_)));

        std::fs::remove_file(&path).ok();
    }
}
