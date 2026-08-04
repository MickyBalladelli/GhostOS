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
}

/// Metadata for a disk ownership marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskLockInfo {
    pub path: PathBuf,
    pub owner: String,
    pub pid: Option<u32>,
    pub image_path: Option<PathBuf>,
    pub stale: bool,
}

impl Drop for DiskLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
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

        let mut magic = [0u8; 8];
        file.read_exact(&mut magic)?;

        // VHD stores its "conectix" cookie in a 512-byte footer at the end
        // of the file, so detect it last by seeking to the tail.
        let format = if magic.starts_with(QCOW2_MAGIC) {
            DiskFormat::Qcow2
        } else if &magic[..8] == VHD_MAGIC {
            DiskFormat::Vhd
        } else {
            let mut footer = [0u8; 8];
            match file.seek(SeekFrom::End(-512)).and_then(|_| file.read_exact(&mut footer)) {
                Ok(()) if &footer[..8] == VHD_MAGIC => {
                    file.seek(SeekFrom::Start(0))?;
                    DiskFormat::Vhd
                }
                _ => DiskFormat::Raw,
            }
        };

        match format {
            DiskFormat::Raw => {
                let size = file.metadata()?.len();
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

    /// Path used for the ownership marker of a writable VM attachment.
    pub fn lock_path<P: AsRef<Path>>(path: P) -> PathBuf {
        PathBuf::from(format!("{}.synos.lock", path.as_ref().display()))
    }

    /// Open an image for a VM and claim exclusive writable ownership.
    pub(crate) fn open_for_vm<P: AsRef<Path>>(
        path: P,
        writable: bool,
    ) -> Result<Self, StorageError> {
        let path = path.as_ref();
        let lock = if writable {
            Some(Self::acquire_lock(path)?)
        } else {
            None
        };
        match Self::open_with_access(path, writable) {
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

    fn acquire_lock(path: &Path) -> Result<DiskLock, StorageError> {
        let lock_path = Self::lock_path(path);
        let mut lock_file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
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
        writeln!(
            lock_file,
            "pid={}\nimage={}",
            std::process::id(),
            path.display()
        )?;
        lock_file.sync_all()?;
        Ok(DiskLock { path: lock_path })
    }

    /// Explicitly remove a stale ownership marker after diagnosing it.
    pub fn recover_lock<P: AsRef<Path>>(path: P) -> Result<(), StorageError> {
        let lock_path = Self::lock_path(path);
        fs::remove_file(&lock_path).map_err(StorageError::Io)
    }

    /// Read ownership metadata without changing the lock.
    pub fn inspect_lock<P: AsRef<Path>>(
        path: P,
    ) -> Result<Option<DiskLockInfo>, StorageError> {
        let lock_path = Self::lock_path(path);
        if !lock_path.exists() {
            return Ok(None);
        }
        let owner = fs::read_to_string(&lock_path)?;
        let pid = owner.lines().find_map(|line| {
            line.strip_prefix("pid=")?.trim().parse::<u32>().ok()
        });
        let image_path = owner
            .lines()
            .find_map(|line| line.strip_prefix("image=").map(PathBuf::from));
        let stale = pid.map(|value| !process_exists(value)).unwrap_or(true);
        Ok(Some(DiskLockInfo {
            path: lock_path,
            owner,
            pid,
            image_path,
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
        Self::recover_lock(path)?;
        Ok(info)
    }

    /// Parse and validate a fixed-size VHD footer.
    fn open_vhd(mut file: File, writable: bool) -> Result<VhdFile, StorageError> {
        file.seek(SeekFrom::End(-512))?;
        let mut footer = [0u8; 512];
        file.read_exact(&mut footer)?;

        // Fixed VHD footer layout (all multi-byte fields are big-endian):
        //   36..39 original size, 40..43 current size, 44..47 disk geometry,
        //   48..51 disk type (2 = fixed), 52..55 checksum.
        let original_size = u32::from_be_bytes(footer[36..40].try_into().unwrap());
        let current_size = u32::from_be_bytes(footer[40..44].try_into().unwrap());
        let disk_type = u32::from_be_bytes(footer[48..52].try_into().unwrap());
        let stored_checksum = u32::from_be_bytes(footer[52..56].try_into().unwrap());

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
            disk_size: current_size as u64,
            writable,
        })
    }

    /// Parse and validate a QCOW2 header (versions 2 and 3).
    fn open_qcow2(mut file: File, writable: bool) -> Result<QcowFile, StorageError> {
        file.seek(SeekFrom::Start(0))?;
        let mut hdr = [0u8; 104];
        file.read_exact(&mut hdr)?;

        let u32_at = |o: usize| u32::from_be_bytes(hdr[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_be_bytes(hdr[o..o + 8].try_into().unwrap());

        let version = u32_at(4);
        if version != 2 && version != 3 {
            return Err(StorageError::Unsupported(format!(
                "QCOW version {version} (only 2 and 3 are supported)"
            )));
        }
        let cluster_bits = u32_at(20);
        if !(9..=21).contains(&cluster_bits) {
            return Err(StorageError::InvalidImage(format!(
                "invalid QCOW2 cluster_bits {cluster_bits}"
            )));
        }
        // Incompatible features: bit 0 = dirty bit, bit 1 = corrupt bit.
        // We do not perform journal recovery, so refuse damaged images to
        // avoid silent corruption.
        let incompatible = u64_at(72);
        if incompatible != 0 {
            return Err(StorageError::Unsupported(format!(
                "QCOW2 incompatible features 0x{incompatible:x} (dirty/corrupt images unsupported)"
            )));
        }
        // L1 table size (number of 8-byte entries) at offset 36; the L1
        // table itself at offset 40.
        let l1_size = u32_at(36);
        if l1_size == 0 || l1_size > (1 << 22) {
            return Err(StorageError::InvalidImage(
                "invalid QCOW2 L1 size".to_string(),
            ));
        }

        let cluster_size = 1u64 << cluster_bits;
        let mut l2_bits = 0u32;
        while (1u64 << l2_bits) < cluster_size / 8 {
            l2_bits += 1;
        }
        let l2_entries = 1u32 << l2_bits;

        let l1_table_offset = u64_at(40);
        let mut l1_cache = vec![0u64; l1_size as usize];
        file.seek(SeekFrom::Start(l1_table_offset))?;
        for slot in l1_cache.iter_mut() {
            let mut b = [0u8; 8];
            file.read_exact(&mut b)?;
            *slot = u64::from_be_bytes(b);
        }

        Ok(QcowFile {
            file,
            header: QcowHeader {
                cluster_bits,
                disk_size: u64_at(24),
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
        if lba.saturating_mul(SECTOR_SIZE).saturating_add(SECTOR_SIZE) > self.size {
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
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
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
        let l2_phys = l1_entry & 0x00FF_FFFF_FFFF_FE00;
        self.load_l2(l2_phys)?;

        let l2_index = (cluster_idx & (self.l2_entries as u64 - 1)) as usize;
        let entry = self.l2_cache[l2_index];
        if entry & 1 == 0 {
            // Bit 0 clear = unallocated.
            return Ok(None);
        }
        let offset = entry & 0x00FF_FFFF_FFFF_FE00;
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
                if entry & 1 != 0 {
                    return Ok(entry & 0x00FF_FFFF_FFFF_FE00);
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
        if existing & 1 != 0 {
            let l2_phys = existing & 0x00FF_FFFF_FFFF_FE00;
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
        if self.cluster_size <= (1 << 20) {
            self.file.write_all(&zeros)?;
        }

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

    #[test]
    fn raw_image_round_trip() {
        let dir = std::env::temp_dir();
        let path = dir.join("synos_vm_raw_test.img");
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
        let path = dir.join("synos_vm_raw_ro_test.img");
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
        let path = dir.join("synos_vm_vhd_test.vhd");
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
        let path = dir.join("synos_vm_qcow2_test.qcow2");
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
        // L1 entry at 0x20000 -> L2 at 0x10000.
        data[0x20000..0x20008].copy_from_slice(&(0x10000u64 | 1).to_be_bytes());

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
}
