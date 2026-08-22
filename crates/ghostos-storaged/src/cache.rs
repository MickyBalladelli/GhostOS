use crate::service::StoragePath;
use ghostos_durability::{CrashBoundary, CrashDomain, InterruptionInjector, NoInterruption};
use ghostos_ipc::{BufferError, BufferLease, BufferOwner};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheMode {
    Disabled,
    ReadThrough,
    CopyOnWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheError {
    Capacity,
    BufferTooSmall,
    NotFound,
    ReadOnly,
    Remote(u16),
    Interrupted,
    Capability(BufferError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CacheEntry<const BYTES: usize> {
    occupied: bool,
    dirty: bool,
    path: StoragePath,
    bytes: [u8; BYTES],
    len: usize,
}

impl<const BYTES: usize> CacheEntry<BYTES> {
    const EMPTY: Self = Self {
        occupied: false,
        dirty: false,
        path: StoragePath::ROOT,
        bytes: [0; BYTES],
        len: 0,
    };
}

pub trait RemoteFileBackend {
    fn read(&mut self, path: StoragePath, destination: &mut [u8]) -> Result<usize, u16>;
    fn write(&mut self, path: StoragePath, contents: &[u8]) -> Result<(), u16>;
    /// Complete only after earlier writes are durable at the remote service.
    fn flush(&mut self) -> Result<(), u16>;
}

/// Bounded read-through / CoW cache for remote mounts.
///
/// A write in `CopyOnWrite` mode only updates a private cache entry. `flush`
/// publishes each dirty entry to the remote backend, so a failed flush keeps
/// the private copy available for retry.
pub struct CowCache<const ENTRIES: usize = 32, const BYTES: usize = { 64 * 1024 }> {
    mode: CacheMode,
    entries: [CacheEntry<BYTES>; ENTRIES],
    pending_flush: bool,
}

impl<const ENTRIES: usize, const BYTES: usize> CowCache<ENTRIES, BYTES> {
    pub const fn new(mode: CacheMode) -> Self {
        Self {
            mode,
            entries: [CacheEntry::EMPTY; ENTRIES],
            pending_flush: false,
        }
    }

    pub const fn mode(&self) -> CacheMode {
        self.mode
    }

    pub fn read(
        &mut self,
        backend: &mut impl RemoteFileBackend,
        path: StoragePath,
        destination: &mut [u8],
    ) -> Result<usize, CacheError> {
        if let Some(entry) = self.entries.iter().find(|entry| entry.occupied && entry.path == path) {
            if destination.len() < entry.len {
                return Err(CacheError::BufferTooSmall)
            }
            destination[..entry.len].copy_from_slice(&entry.bytes[..entry.len]);
            return Ok(entry.len)
        }
        if self.mode == CacheMode::Disabled {
            return backend.read(path, destination).map_err(CacheError::Remote)
        }
        let length = backend.read(path, destination).map_err(CacheError::Remote)?;
        if length > BYTES {
            return Err(CacheError::Capacity)
        }
        let entry = self.allocate(path)?;
        entry.bytes[..length].copy_from_slice(&destination[..length]);
        entry.len = length;
        Ok(length)
    }

    pub fn write(
        &mut self,
        backend: &mut impl RemoteFileBackend,
        path: StoragePath,
        contents: &[u8],
    ) -> Result<(), CacheError> {
        if self.mode == CacheMode::Disabled {
            backend.write(path, contents).map_err(CacheError::Remote)?;
            self.pending_flush = true;
            return Ok(())
        }
        if contents.len() > BYTES {
            return Err(CacheError::Capacity)
        }
        let mode = self.mode;
        {
            let entry = self.allocate(path)?;
            entry.bytes[..contents.len()].copy_from_slice(contents);
            entry.len = contents.len();
            entry.dirty = mode == CacheMode::CopyOnWrite;
            if mode == CacheMode::ReadThrough {
                backend.write(path, contents).map_err(CacheError::Remote)?;
                entry.dirty = false;
            }
        }
        if mode == CacheMode::ReadThrough {
            self.pending_flush = true;
        }
        Ok(())
    }

    pub fn flush(&mut self, backend: &mut impl RemoteFileBackend) -> Result<usize, CacheError> {
        let mut no_interruption = NoInterruption;
        self.flush_with_interruption(backend, &mut no_interruption)
    }

    /// Read directly into a capability-guarded storage buffer. Disabled and
    /// read-through modes keep the backend on the caller's buffer; CoW mode
    /// makes only its required cache copy.
    pub fn read_loaned(
        &mut self,
        backend: &mut impl RemoteFileBackend,
        path: StoragePath,
        destination: &mut BufferLease<'_>,
    ) -> Result<usize, CacheError> {
        if destination.owner() != BufferOwner::Storage {
            return Err(CacheError::Capability(BufferError::OwnerMismatch));
        }
        let bytes = destination
            .as_mut_slice()
            .map_err(CacheError::Capability)?;
        self.read(backend, path, bytes)
    }

    /// Write directly from a capability-guarded storage buffer. The remote
    /// backend borrows the same bytes and never receives an intermediate frame.
    pub fn write_loaned(
        &mut self,
        backend: &mut impl RemoteFileBackend,
        path: StoragePath,
        contents: &BufferLease<'_>,
    ) -> Result<(), CacheError> {
        if contents.owner() != BufferOwner::Storage {
            return Err(CacheError::Capability(BufferError::OwnerMismatch));
        }
        let bytes = contents.as_slice().map_err(CacheError::Capability)?;
        self.write(backend, path, bytes)
    }

    pub fn flush_with_interruption<I: InterruptionInjector>(
        &mut self,
        backend: &mut impl RemoteFileBackend,
        injector: &mut I,
    ) -> Result<usize, CacheError> {
        let mut dirty = 0;
        for entry in &mut self.entries {
            if !entry.occupied || !entry.dirty {
                continue
            }
            backend
                .write(entry.path, &entry.bytes[..entry.len])
                .map_err(CacheError::Remote)?;
            dirty += 1;
        }
        if dirty != 0 || self.pending_flush {
            backend.flush().map_err(CacheError::Remote)?;
        }
        if injector.checkpoint(CrashDomain::Storage, CrashBoundary::Flush) {
            return Err(CacheError::Interrupted)
        }
        for entry in &mut self.entries {
            if entry.occupied && entry.dirty {
                entry.dirty = false;
            }
        }
        self.pending_flush = false;
        Ok(dirty)
    }

    /// Flush dirty cache entries and return only after the remote durability
    /// fence has completed. A successful return is the cache equivalent of
    /// `fsync`; an error leaves dirty entries available for retry.
    pub fn fsync(&mut self, backend: &mut impl RemoteFileBackend) -> Result<usize, CacheError> {
        self.flush(backend)
    }

    pub fn sync(&mut self, backend: &mut impl RemoteFileBackend) -> Result<usize, CacheError> {
        self.fsync(backend)
    }

    pub fn invalidate(&mut self, path: StoragePath) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.occupied && entry.path == path) {
            *entry = CacheEntry::EMPTY;
            true
        } else {
            false
        }
    }

    pub fn dirty_entries(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.occupied && entry.dirty)
            .count()
    }

    fn allocate(&mut self, path: StoragePath) -> Result<&mut CacheEntry<BYTES>, CacheError> {
        if let Some(index) = self.entries.iter().position(|entry| entry.occupied && entry.path == path) {
            return Ok(&mut self.entries[index])
        }
        let index = self
            .entries
            .iter()
            .position(|entry| !entry.occupied)
            .ok_or(CacheError::Capacity)?;
        self.entries[index] = CacheEntry {
            occupied: true,
            dirty: false,
            path,
            bytes: [0; BYTES],
            len: 0,
        };
        Ok(&mut self.entries[index])
    }
}
