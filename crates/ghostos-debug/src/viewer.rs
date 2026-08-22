//! Read-only crash-dump inspection for operators and recovery tools.

use ghostos_ghostfs::{DirectoryEntry, FileType, SynFs};

use crate::{CoreDumpMetadata, CoreDumpPage, CrashDumpStore, Error};
use crate::coredump::{
    CorePath, CORE_PAGE_BYTES, CORE_PAGE_HEADER_BYTES, CORE_ROOT, MAX_CORE_DIRECTORY_ENTRIES,
};

pub const MAX_CRASH_DUMPS: usize = MAX_CORE_DIRECTORY_ENTRIES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CrashDumpSummary {
    pub directory: CorePath,
    pub metadata: CoreDumpMetadata,
}

pub struct CrashDumpViewer<'a, const MAX_BLOCKS: usize> {
    filesystem: &'a SynFs<MAX_BLOCKS>,
}

impl<'a, const MAX_BLOCKS: usize> CrashDumpViewer<'a, MAX_BLOCKS> {
    pub const fn new(filesystem: &'a SynFs<MAX_BLOCKS>) -> Self {
        Self { filesystem }
    }

    pub fn list(
        &self,
        output: &mut [Option<CrashDumpSummary>; MAX_CRASH_DUMPS],
    ) -> Result<usize, Error> {
        output.fill(None);
        let mut entries = [DirectoryEntry::EMPTY; MAX_CORE_DIRECTORY_ENTRIES];
        let count = match self.filesystem.list_directory(CORE_ROOT, &mut entries) {
            Ok(count) => count,
            Err(ghostos_ghostfs::Error::NotFound) => return Ok(0),
            Err(error) => return Err(map_storage_error(error)),
        };
        let mut written = 0;
        for entry in entries[..count].iter().copied() {
            if entry.file_type != FileType::Directory {
                continue;
            }
            if written == output.len() {
                return Err(Error::BufferTooSmall {
                    required: written.saturating_add(1),
                });
            }
            let directory = CorePath::new(CORE_ROOT)?.child(entry.name.as_str())?;
            let metadata = CrashDumpStore::read_metadata(self.filesystem, directory.as_str())?;
            output[written] = Some(CrashDumpSummary { directory, metadata });
            written += 1;
        }
        Ok(written)
    }

    pub fn open(&self, directory: &str) -> Result<CrashDumpView<'a, MAX_BLOCKS>, Error> {
        let directory = CorePath::new(directory)?;
        if directory.as_str() == CORE_ROOT
            || !directory
                .as_str()
                .strip_prefix(CORE_ROOT)
                .is_some_and(|suffix| suffix.starts_with('/'))
        {
            return Err(Error::InvalidInput);
        }
        let file = self
            .filesystem
            .lookup(directory.as_str())
            .map_err(map_storage_error)?;
        if file.file_type != FileType::Directory {
            return Err(Error::InvalidInput);
        }
        let metadata = CrashDumpStore::read_metadata(self.filesystem, directory.as_str())?;
        Ok(CrashDumpView {
            filesystem: self.filesystem,
            directory,
            metadata,
        })
    }
}

pub type CrashViewer<'a, const MAX_BLOCKS: usize> = CrashDumpViewer<'a, MAX_BLOCKS>;

pub struct CrashDumpView<'a, const MAX_BLOCKS: usize> {
    filesystem: &'a SynFs<MAX_BLOCKS>,
    pub directory: CorePath,
    pub metadata: CoreDumpMetadata,
}

impl<const MAX_BLOCKS: usize> CrashDumpView<'_, MAX_BLOCKS> {
    pub fn read_page(
        &self,
        index: usize,
        destination: &mut [u8],
    ) -> Result<CoreDumpPage, Error> {
        if index >= self.metadata.pages as usize {
            return Err(Error::NotFound);
        }
        let page_path = self.directory.page(index)?;
        let mut header = [0; CORE_PAGE_HEADER_BYTES];
        let header_read = self
            .filesystem
            .read_at(page_path.as_str(), 0, &mut header)
            .map_err(map_storage_error)?;
        if header_read.bytes_read != CORE_PAGE_HEADER_BYTES {
            return Err(Error::Corrupt);
        }
        let virtual_address = u64::from_le_bytes(
            header[..8].try_into().map_err(|_| Error::Corrupt)?,
        );
        let length_u64 = u64::from_le_bytes(
            header[8..16].try_into().map_err(|_| Error::Corrupt)?,
        );
        let length = usize::try_from(length_u64).map_err(|_| Error::Corrupt)?;
        if length == 0 || length > CORE_PAGE_BYTES || length > destination.len() {
            return Err(Error::BufferTooSmall {
                required: length,
            });
        }
        let payload = self
            .filesystem
            .read_at(page_path.as_str(), CORE_PAGE_HEADER_BYTES as u64, &mut destination[..length])
            .map_err(map_storage_error)?;
        if payload.bytes_read != length || payload.file.size != (CORE_PAGE_HEADER_BYTES + length) as u64 {
            return Err(Error::Corrupt);
        }
        Ok(CoreDumpPage {
            virtual_address,
            length,
        })
    }
}

fn map_storage_error(error: ghostos_ghostfs::Error) -> Error {
    match error {
        ghostos_ghostfs::Error::NotFound => Error::NotFound,
        error => Error::Storage(error),
    }
}
