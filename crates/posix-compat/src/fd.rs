use ghostos_runtime::File;

use crate::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccessMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl AccessMode {
    pub const fn readable(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    pub const fn writable(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }
}

/// One process-local descriptor entry.
///
/// The filesystem capability is intentionally not shared between entries.
/// A descriptor owns exactly one open service handle, while its offset and
/// status flags stay in this process table. This keeps close ownership
/// unambiguous and prevents one descriptor from invalidating another one by
/// closing the same service capability.
#[derive(Clone, Copy)]
pub(crate) struct Entry {
    pub file: File,
    pub offset: u64,
    pub access: AccessMode,
    pub append: bool,
}

pub(crate) struct FdTable<const CAPACITY: usize> {
    entries: [Option<Entry>; CAPACITY],
}

impl<const CAPACITY: usize> FdTable<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            entries: [None; CAPACITY],
        }
    }

    pub fn has_free_slot(&self) -> bool {
        self.entries.iter().any(Option::is_none)
    }

    pub fn insert(
        &mut self,
        file: File,
        access: AccessMode,
        append: bool,
    ) -> Result<i32, Error> {
        let slot = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(Error::TooManyFiles)?;
        let fd = slot
            .checked_add(3)
            .and_then(|fd| i32::try_from(fd).ok())
            .ok_or(Error::TooManyFiles)?;
        self.entries[slot] = Some(Entry {
            file,
            offset: 0,
            access,
            append,
        });
        Ok(fd)
    }

    pub fn get(&self, fd: i32) -> Result<Entry, Error> {
        self.entries
            .get(Self::slot(fd)?)
            .and_then(|entry| *entry)
            .ok_or(Error::BadFileDescriptor)
    }

    pub fn get_readable(&self, fd: i32) -> Result<Entry, Error> {
        let entry = self.get(fd)?;
        if !entry.access.readable() {
            return Err(Error::PermissionDenied)
        }
        Ok(entry)
    }

    pub fn get_writable(&self, fd: i32) -> Result<Entry, Error> {
        let entry = self.get(fd)?;
        if !entry.access.writable() {
            return Err(Error::PermissionDenied)
        }
        Ok(entry)
    }

    pub fn update_offset(&mut self, fd: i32, offset: u64) -> Result<(), Error> {
        let entry = self
            .entries
            .get_mut(Self::slot(fd)?)
            .and_then(Option::as_mut)
            .ok_or(Error::BadFileDescriptor)?;
        entry.offset = offset;
        Ok(())
    }

    pub fn remove(&mut self, fd: i32) -> Result<Entry, Error> {
        self.entries
            .get_mut(Self::slot(fd)?)
            .and_then(Option::take)
            .ok_or(Error::BadFileDescriptor)
    }

    fn slot(fd: i32) -> Result<usize, Error> {
        let fd = usize::try_from(fd).map_err(|_| Error::BadFileDescriptor)?;
        fd.checked_sub(3).ok_or(Error::BadFileDescriptor)
    }
}
