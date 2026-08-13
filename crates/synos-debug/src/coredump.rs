//! Crash capture for isolated Ring 3 processes.
//!
//! A runtime supplies a frozen, CoW-backed view. The engine only reads that
//! view and commits a bounded SynFS transaction, so the crashed process does
//! not stop other services or the microkernel.

use synos_fabric::PAGE_SIZE;
use synos_init::{CrashReason, ProcessId};
use synos_synfs::{FileType, SynFs};

use crate::{Error, gdb::RegisterFile};

pub const CORE_VERSION: u16 = 1;
pub const CORE_PAGE_BYTES: usize = PAGE_SIZE as usize;
pub const CORE_PAGE_HEADER_BYTES: usize = 16;
pub const MAX_CORE_PATH_BYTES: usize = 192;
pub const CORE_METADATA_BYTES: usize = 8 + 2 + 1 + 1 + 8 + 8 + 4 + 8 * 32;
pub const CORE_ROOT: &str = "/cores";
pub const MAX_CORE_DIRECTORY_ENTRIES: usize = 128;

const CORE_MAGIC: [u8; 8] = *b"SYNCORE1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreDumpRequest<'a> {
    pub process: ProcessId,
    pub reason: CrashReason,
    pub timestamp_us: u64,
    pub directory: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenPage {
    pub virtual_address: u64,
    pub length: usize,
}

pub trait FrozenProcess {
    fn registers(&self) -> RegisterFile;
    fn page_count(&self) -> usize;
    fn read_page(&self, index: usize, destination: &mut [u8]) -> Result<FrozenPage, Error>;
}

pub trait CoreDumpRuntime {
    type Frozen: FrozenProcess;

    fn freeze(&mut self, process: ProcessId, reason: CrashReason) -> Result<Self::Frozen, Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CorePath {
    bytes: [u8; MAX_CORE_PATH_BYTES],
    len: u16,
}

impl CorePath {
    pub fn new(path: &str) -> Result<Self, Error> {
        if path.is_empty()
            || path.len() > MAX_CORE_PATH_BYTES
            || path.contains('\0')
            || !path.starts_with('/')
            || path.ends_with('/')
            || path.contains("//")
            || path.contains(';')
            || path
                .split('/')
                .any(|component| component == "." || component == "..")
        {
            return Err(Error::InvalidInput);
        }
        let mut result = Self {
            bytes: [0; MAX_CORE_PATH_BYTES],
            len: path.len() as u16,
        };
        result.bytes[..path.len()].copy_from_slice(path.as_bytes());
        Ok(result)
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).expect("core path invariant")
    }

    pub(crate) fn child(&self, suffix: &str) -> Result<Self, Error> {
        let required = self.len as usize + 1 + suffix.len();
        if required > MAX_CORE_PATH_BYTES {
            return Err(Error::BufferTooSmall { required });
        }
        let mut result = *self;
        result.bytes[self.len as usize] = b'/';
        result.bytes[self.len as usize + 1..required].copy_from_slice(suffix.as_bytes());
        result.len = required as u16;
        Ok(result)
    }

    pub(crate) fn page(&self, index: usize) -> Result<Self, Error> {
        if index > 99_999_999 {
            return Err(Error::Capacity);
        }
        let mut suffix = [0; 13];
        suffix[..5].copy_from_slice(b"PAGE-");
        let mut value = index as u64;
        for position in 0..8 {
            suffix[12 - position] = b'0' + (value % 10) as u8;
            value /= 10;
        }
        let suffix = core::str::from_utf8(&suffix).expect("static core path suffix");
        self.child(suffix)
    }

    pub(crate) fn metadata(&self) -> Result<Self, Error> {
        self.child("META")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreDumpMetadata {
    pub process: ProcessId,
    pub reason: CrashReason,
    pub timestamp_us: u64,
    pub pages: u32,
    pub registers: RegisterFile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreDumpPage {
    pub virtual_address: u64,
    pub length: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreDumpReceipt {
    pub process: ProcessId,
    pub directory: CorePath,
    pub generation: u64,
    pub pages: u32,
    pub bytes: u64,
}

pub struct CoreDumpEngine;

pub struct CrashDumpStore;

impl CrashDumpStore {
    pub fn capture<R, const MAX_BLOCKS: usize>(
        runtime: &mut R,
        filesystem: &mut SynFs<MAX_BLOCKS>,
        request: CoreDumpRequest<'_>,
    ) -> Result<CoreDumpReceipt, Error>
    where
        R: CoreDumpRuntime,
    {
        let directory = CorePath::new(request.directory)?;
        if !is_core_directory(&directory) {
            return Err(Error::InvalidInput);
        }
        CoreDumpEngine::capture(runtime, filesystem, request)
    }

    pub fn read_metadata<const MAX_BLOCKS: usize>(
        filesystem: &SynFs<MAX_BLOCKS>,
        directory: &str,
    ) -> Result<CoreDumpMetadata, Error> {
        let directory = CorePath::new(directory)?;
        if !is_core_directory(&directory) {
            return Err(Error::InvalidInput);
        }
        let metadata = directory.metadata()?;
        let file = filesystem.lookup(metadata.as_str()).map_err(map_storage_error)?;
        if file.file_type != FileType::Regular || file.size != CORE_METADATA_BYTES as u64 {
            return Err(Error::Corrupt);
        }
        let mut bytes = [0; CORE_METADATA_BYTES];
        let read = filesystem
            .read(metadata.as_str(), &mut bytes)
            .map_err(map_storage_error)?;
        if read.bytes_read != bytes.len() {
            return Err(Error::Corrupt);
        }
        decode_metadata(&bytes)
    }

    pub fn delete<const MAX_BLOCKS: usize>(
        filesystem: &mut SynFs<MAX_BLOCKS>,
        directory: &str,
    ) -> Result<(), Error> {
        let directory = CorePath::new(directory)?;
        if !is_core_directory(&directory) {
            return Err(Error::InvalidInput);
        }
        let file = filesystem.lookup(directory.as_str()).map_err(map_storage_error)?;
        if file.file_type != FileType::Directory {
            return Err(Error::InvalidInput);
        }

        let mut entries = [synos_synfs::DirectoryEntry::EMPTY; MAX_CORE_DIRECTORY_ENTRIES];
        let count = filesystem
            .list_directory(directory.as_str(), &mut entries)
            .map_err(map_storage_error)?;
        let mut transaction = filesystem.transaction();
        for entry in entries[..count].iter().copied() {
            if entry.file_type == FileType::Directory {
                return Err(Error::InvalidInput);
            }
            let path = directory.child(entry.name.as_str())?;
            transaction.delete(path.as_str()).map_err(Error::Storage)?;
        }
        transaction
            .remove_directory(directory.as_str())
            .map_err(Error::Storage)?;
        transaction.commit().map_err(Error::Storage)?;
        Ok(())
    }
}

impl CoreDumpEngine {
    pub fn capture<R, const MAX_BLOCKS: usize>(
        runtime: &mut R,
        filesystem: &mut SynFs<MAX_BLOCKS>,
        request: CoreDumpRequest<'_>,
    ) -> Result<CoreDumpReceipt, Error>
    where
        R: CoreDumpRuntime,
    {
        let directory = CorePath::new(request.directory)?;
        let frozen = runtime.freeze(request.process, request.reason)?;
        let page_count = frozen.page_count();
        let pages = u32::try_from(page_count).map_err(|_| Error::Capacity)?;
        let mut metadata = [0; CORE_METADATA_BYTES];
        encode_metadata(&mut metadata, request, frozen.registers(), pages)?;

        let mut transaction = filesystem.transaction();
        match transaction.lookup(directory.as_str()) {
            Ok(file) if file.file_type == FileType::Directory => {}
            Ok(_) => return Err(Error::InvalidInput),
            Err(synos_synfs::Error::NotFound) => {
                transaction
                    .create_directory(directory.as_str(), true)
                    .map_err(Error::Storage)?;
            }
            Err(error) => return Err(Error::Storage(error)),
        }

        let metadata_path = directory.child("META")?;
        transaction
            .write(metadata_path.as_str(), &metadata)
            .map_err(Error::Storage)?;

        let mut page_bytes = [0; CORE_PAGE_BYTES];
        let mut page_storage = [0; CORE_PAGE_HEADER_BYTES + CORE_PAGE_BYTES];
        let mut total_bytes = 0_u64;
        for index in 0..page_count {
            let page = frozen.read_page(index, &mut page_bytes)?;
            if page.length == 0 || page.length > page_bytes.len() {
                return Err(Error::Snapshot);
            }
            page_storage[..8].copy_from_slice(&page.virtual_address.to_le_bytes());
            page_storage[8..16].copy_from_slice(&(page.length as u64).to_le_bytes());
            page_storage[CORE_PAGE_HEADER_BYTES..CORE_PAGE_HEADER_BYTES + page.length]
                .copy_from_slice(&page_bytes[..page.length]);
            let path = directory.page(index)?;
            transaction
                .write(
                    path.as_str(),
                    &page_storage[..CORE_PAGE_HEADER_BYTES + page.length],
                )
                .map_err(Error::Storage)?;
            total_bytes = total_bytes.saturating_add(page.length as u64);
        }

        let commit = transaction.commit().map_err(Error::Storage)?;
        Ok(CoreDumpReceipt {
            process: request.process,
            directory,
            generation: commit.generation,
            pages,
            bytes: total_bytes,
        })
    }
}

fn encode_metadata(
    output: &mut [u8; CORE_METADATA_BYTES],
    request: CoreDumpRequest<'_>,
    registers: RegisterFile,
    pages: u32,
) -> Result<(), Error> {
    output[..8].copy_from_slice(&CORE_MAGIC);
    output[8..10].copy_from_slice(&CORE_VERSION.to_le_bytes());
    output[10] = crash_reason(request.reason);
    output[11] = registers.count() as u8;
    output[12..20].copy_from_slice(&request.process.raw().to_le_bytes());
    output[20..28].copy_from_slice(&request.timestamp_us.to_le_bytes());
    output[28..32].copy_from_slice(&pages.to_le_bytes());
    for (index, value) in registers.values().iter().copied().enumerate() {
        let start = 32 + index * 8;
        output[start..start + 8].copy_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

pub fn decode_metadata(input: &[u8]) -> Result<CoreDumpMetadata, Error> {
    if input.len() < CORE_METADATA_BYTES {
        return Err(Error::BufferTooSmall {
            required: CORE_METADATA_BYTES,
        });
    }
    if input[..8] != CORE_MAGIC || u16::from_le_bytes([input[8], input[9]]) != CORE_VERSION {
        return Err(Error::Corrupt);
    }
    let reason = decode_crash_reason(input[10]).ok_or(Error::Corrupt)?;
    let register_count = input[11] as usize;
    if register_count == 0 || register_count > 32 {
        return Err(Error::Corrupt);
    }
    let process = ProcessId::new(u64::from_le_bytes(
        input[12..20].try_into().map_err(|_| Error::Corrupt)?,
    ))
    .ok_or(Error::Corrupt)?;
    let timestamp_us = u64::from_le_bytes(input[20..28].try_into().map_err(|_| Error::Corrupt)?);
    let pages = u32::from_le_bytes(input[28..32].try_into().map_err(|_| Error::Corrupt)?);
    let mut values = [0; 32];
    for (index, value) in values[..register_count].iter_mut().enumerate() {
        let start = 32 + index * 8;
        *value = u64::from_le_bytes(input[start..start + 8].try_into().map_err(|_| Error::Corrupt)?);
    }
    let registers = RegisterFile::from_slice(&values[..register_count])?;
    Ok(CoreDumpMetadata {
        process,
        reason,
        timestamp_us,
        pages,
        registers,
    })
}

fn decode_crash_reason(value: u8) -> Option<CrashReason> {
    match value {
        1 => Some(CrashReason::Panic),
        2 => Some(CrashReason::ProtectionFault),
        3 => Some(CrashReason::IllegalInstruction),
        4 => Some(CrashReason::Watchdog),
        5 => Some(CrashReason::UnexpectedExit),
        _ => None,
    }
}

fn map_storage_error(error: synos_synfs::Error) -> Error {
    match error {
        synos_synfs::Error::NotFound => Error::NotFound,
        error => Error::Storage(error),
    }
}

fn is_core_directory(path: &CorePath) -> bool {
    path.as_str() != CORE_ROOT
        && path
            .as_str()
            .strip_prefix(CORE_ROOT)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn crash_reason(reason: CrashReason) -> u8 {
    match reason {
        CrashReason::Panic => 1,
        CrashReason::ProtectionFault => 2,
        CrashReason::IllegalInstruction => 3,
        CrashReason::Watchdog => 4,
        CrashReason::UnexpectedExit => 5,
    }
}
