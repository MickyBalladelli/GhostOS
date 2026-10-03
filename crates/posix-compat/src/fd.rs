use ghostos_runtime::File;

use crate::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccessMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
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
    pub append: bool,
}

pub(crate) struct FdTable<const CAPACITY: usize> {
    files: [Option<File>; CAPACITY],
    entries: [NativeEntry; CAPACITY],
}

impl<const CAPACITY: usize> FdTable<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            files: [None; CAPACITY],
            entries: [NativeEntry::EMPTY; CAPACITY],
        }
    }

    pub fn has_free_slot(&self) -> bool {
        unsafe { ghostos_posix_fd_has_free(self.entries.as_ptr(), CAPACITY) }
    }

    pub fn insert(
        &mut self,
        file: File,
        access: AccessMode,
        append: bool,
    ) -> Result<i32, Error> {
        let mut fd = 0;
        if !unsafe { ghostos_posix_fd_insert(self.entries.as_mut_ptr(), CAPACITY, access as u8, append, &mut fd) } {
            return Err(Error::TooManyFiles)
        }
        self.files[(fd - 3) as usize] = Some(file);
        Ok(fd)
    }

    pub fn get(&self, fd: i32) -> Result<Entry, Error> {
        self.get_with_access(fd, 0)
    }

    pub fn get_readable(&self, fd: i32) -> Result<Entry, Error> {
        self.get_with_access(fd, 1)
    }

    pub fn get_writable(&self, fd: i32) -> Result<Entry, Error> {
        self.get_with_access(fd, 2)
    }

    fn get_with_access(&self, fd: i32, required: u8) -> Result<Entry, Error> {
        let mut native = NativeEntry::EMPTY;
        match unsafe { ghostos_posix_fd_get(self.entries.as_ptr(), CAPACITY, fd, required, &mut native) } {
            0 => {},
            1 => return Err(Error::BadFileDescriptor),
            2 => return Err(Error::PermissionDenied),
            _ => unreachable!("native POSIX descriptor result"),
        }
        let file = self.files[(fd - 3) as usize].ok_or(Error::BadFileDescriptor)?;
        Ok(native.to_public(file))
    }

    pub fn update_offset(&mut self, fd: i32, offset: u64) -> Result<(), Error> {
        if unsafe { ghostos_posix_fd_offset(self.entries.as_mut_ptr(), CAPACITY, fd, offset) } {
            Ok(())
        } else { Err(Error::BadFileDescriptor) }
    }

    pub fn remove(&mut self, fd: i32) -> Result<Entry, Error> {
        let mut native = NativeEntry::EMPTY;
        if !unsafe { ghostos_posix_fd_remove(self.entries.as_mut_ptr(), CAPACITY, fd, &mut native) } {
            return Err(Error::BadFileDescriptor)
        }
        let file = self.files[(fd - 3) as usize].take().ok_or(Error::BadFileDescriptor)?;
        Ok(native.to_public(file))
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeEntry { offset: u64, access: u8, append: bool, occupied: bool }
impl NativeEntry {
    const EMPTY: Self = Self { offset: 0, access: 0, append: false, occupied: false };
    fn to_public(self, file: File) -> Entry {
        Entry { file, offset: self.offset, append: self.append }
    }
}
const _: () = {
    assert!(core::mem::size_of::<NativeEntry>() == 16);
    assert!(core::mem::offset_of!(NativeEntry, occupied) == 10);
};
unsafe extern "C" {
    fn ghostos_posix_fd_has_free(entries: *const NativeEntry, capacity: usize) -> bool;
    fn ghostos_posix_fd_insert(entries: *mut NativeEntry, capacity: usize,
        access: u8, append: bool, fd: *mut i32) -> bool;
    fn ghostos_posix_fd_get(entries: *const NativeEntry, capacity: usize,
        fd: i32, required: u8, entry: *mut NativeEntry) -> i32;
    fn ghostos_posix_fd_offset(entries: *mut NativeEntry, capacity: usize, fd: i32, offset: u64) -> bool;
    fn ghostos_posix_fd_remove(entries: *mut NativeEntry, capacity: usize, fd: i32, entry: *mut NativeEntry) -> bool;
}
