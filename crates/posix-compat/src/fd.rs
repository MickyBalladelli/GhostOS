use synos_runtime::File;

use crate::Error;

#[derive(Clone, Copy)]
pub(crate) struct Entry {
    pub file: File,
    pub offset: u64,
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

    pub fn insert(&mut self, file: File, append: bool) -> Result<i32, Error> {
        let slot = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(Error::TooManyFiles)?;
        self.entries[slot] = Some(Entry {
            file,
            offset: 0,
            append,
        });
        i32::try_from(slot + 3).map_err(|_| Error::TooManyFiles)
    }

    pub fn get(&self, fd: i32) -> Result<Entry, Error> {
        self.entries
            .get(Self::slot(fd)?)
            .and_then(|entry| *entry)
            .ok_or(Error::BadFileDescriptor)
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
