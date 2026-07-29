#![no_std]
#![forbid(unsafe_code)]

use synos_status::{IntoStatus, Status};
use synos_synfs::{
    CheckpointId, CheckpointInfo, Error as FileError, FileVersion, MAX_PATH_BYTES,
    RmsMapHandle, SynFs,
};

const ARCHIVE_MAGIC: &[u8; 8] = b"SYNBACK1";
const TRAILER_MAGIC: &[u8; 8] = b"SYNBEND1";
const FORMAT_VERSION: u16 = 1;
const ARCHIVE_HEADER_BYTES: usize = 32;
const ENTRY_FIXED_BYTES: usize = 36;
const TRAILER_BYTES: usize = 16;
pub const STREAM_BUFFER_BYTES: usize = 4096;

pub trait BackupSink {
    /// Append the complete slice or return an error without accepting it.
    fn append(&mut self, bytes: &[u8]) -> Result<(), Status>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupError {
    File(FileError),
    InvalidBudget,
    NotComplete,
    Sink(Status),
}

impl From<FileError> for BackupError {
    fn from(error: FileError) -> Self {
        Self::File(error)
    }
}

impl IntoStatus for BackupError {
    fn status(self) -> Status {
        match self {
            Self::File(error) => error.status(),
            Self::InvalidBudget => Status::INVALID_ARGUMENT,
            Self::NotComplete => Status::BUSY,
            Self::Sink(status) => status,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupState {
    VolumeHeader,
    FileHeader,
    FileData,
    Trailer,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackupProgress {
    pub checkpoint: CheckpointInfo,
    pub state: BackupState,
    pub files_streamed: u32,
    pub bytes_streamed: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackupReport {
    pub checkpoint: CheckpointInfo,
    pub files_streamed: u32,
    pub bytes_streamed: u64,
}

/// Cooperative volume backup. Each poll copies at most `byte_budget` bytes
/// from one pinned SynFS CoW root to a caller-provided sink.
pub struct BackupJob {
    checkpoint: CheckpointInfo,
    capability: RmsMapHandle,
    state: BackupState,
    header: [u8; ENTRY_FIXED_BYTES + MAX_PATH_BYTES],
    header_length: usize,
    header_offset: usize,
    file_index: u32,
    file_offset: u64,
    current_file: Option<FileVersion>,
    files_streamed: u32,
    bytes_streamed: u64,
    released: bool,
}

impl BackupJob {
    pub fn start<const MAX_BLOCKS: usize>(
        filesystem: &mut SynFs<MAX_BLOCKS>,
        capability: RmsMapHandle,
    ) -> Result<Self, BackupError> {
        let checkpoint = filesystem.create_checkpoint()?;
        let mut job = Self {
            checkpoint,
            capability,
            state: BackupState::VolumeHeader,
            header: [0; ENTRY_FIXED_BYTES + MAX_PATH_BYTES],
            header_length: ARCHIVE_HEADER_BYTES,
            header_offset: 0,
            file_index: 0,
            file_offset: 0,
            current_file: None,
            files_streamed: 0,
            bytes_streamed: 0,
            released: false,
        };
        job.encode_volume_header();
        Ok(job)
    }

    pub const fn checkpoint(&self) -> CheckpointInfo {
        self.checkpoint
    }

    pub const fn progress(&self) -> BackupProgress {
        BackupProgress {
            checkpoint: self.checkpoint,
            state: self.state,
            files_streamed: self.files_streamed,
            bytes_streamed: self.bytes_streamed,
        }
    }

    pub fn poll<const MAX_BLOCKS: usize>(
        &mut self,
        filesystem: &SynFs<MAX_BLOCKS>,
        sink: &mut impl BackupSink,
        byte_budget: usize,
    ) -> Result<BackupProgress, BackupError> {
        if byte_budget == 0 {
            return Err(BackupError::InvalidBudget)
        }
        let snapshot =
            filesystem.checkpoint_snapshot(self.checkpoint.id, self.capability)?;
        let mut remaining = byte_budget;

        while remaining != 0 && self.state != BackupState::Complete {
            match self.state {
                BackupState::VolumeHeader
                | BackupState::FileHeader
                | BackupState::Trailer => {
                    let amount = (self.header_length - self.header_offset).min(remaining);
                    sink.append(
                        &self.header[self.header_offset..self.header_offset + amount],
                    )
                    .map_err(BackupError::Sink)?;
                    self.header_offset += amount;
                    self.bytes_streamed = self.bytes_streamed.saturating_add(amount as u64);
                    remaining -= amount;
                    if self.header_offset == self.header_length {
                        match self.state {
                            BackupState::VolumeHeader => {
                                self.prepare_next_file(&snapshot)?
                            }
                            BackupState::FileHeader => {
                                self.state = BackupState::FileData
                            }
                            BackupState::Trailer => self.state = BackupState::Complete,
                            _ => unreachable!(),
                        }
                    }
                }
                BackupState::FileData => {
                    let file = self.current_file.ok_or(FileError::Corrupt)?;
                    let available = file.size.saturating_sub(self.file_offset);
                    let amount = remaining
                        .min(STREAM_BUFFER_BYTES)
                        .min(usize::try_from(available).unwrap_or(usize::MAX));
                    if amount == 0 {
                        self.files_streamed = self.files_streamed.saturating_add(1);
                        self.file_index = self.file_index.saturating_add(1);
                        self.prepare_next_file(&snapshot)?;
                        continue
                    }
                    let mut buffer = [0; STREAM_BUFFER_BYTES];
                    let copied = snapshot.read_file_at(
                        self.file_index,
                        self.file_offset,
                        &mut buffer[..amount],
                    )?;
                    if copied != amount {
                        return Err(FileError::Corrupt.into())
                    }
                    sink.append(&buffer[..copied]).map_err(BackupError::Sink)?;
                    self.file_offset = self.file_offset.saturating_add(copied as u64);
                    self.bytes_streamed =
                        self.bytes_streamed.saturating_add(copied as u64);
                    remaining -= copied
                }
                BackupState::Complete => {}
            }
        }
        Ok(self.progress())
    }

    pub fn finish<const MAX_BLOCKS: usize>(
        &mut self,
        filesystem: &mut SynFs<MAX_BLOCKS>,
    ) -> Result<BackupReport, BackupError> {
        if self.state != BackupState::Complete {
            return Err(BackupError::NotComplete)
        }
        if !self.released {
            filesystem.release_checkpoint(self.checkpoint.id)?;
            self.released = true
        }
        Ok(BackupReport {
            checkpoint: self.checkpoint,
            files_streamed: self.files_streamed,
            bytes_streamed: self.bytes_streamed,
        })
    }

    pub fn cancel<const MAX_BLOCKS: usize>(
        &mut self,
        filesystem: &mut SynFs<MAX_BLOCKS>,
    ) -> Result<(), BackupError> {
        if !self.released {
            filesystem.release_checkpoint(self.checkpoint.id)?;
            self.released = true
        }
        Ok(())
    }

    fn prepare_next_file<const MAX_BLOCKS: usize>(
        &mut self,
        snapshot: &synos_synfs::ReadOnlySnapshot<'_, MAX_BLOCKS>,
    ) -> Result<(), BackupError> {
        let Some(file) = snapshot.file_at(self.file_index)? else {
            self.encode_trailer();
            return Ok(())
        };
        self.current_file = Some(file);
        self.file_offset = 0;
        self.encode_file_header(file);
        Ok(())
    }

    fn encode_volume_header(&mut self) {
        self.header.fill(0);
        self.header[..8].copy_from_slice(ARCHIVE_MAGIC);
        self.header[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        self.header[12..20].copy_from_slice(&self.checkpoint.id.raw().to_le_bytes());
        self.header[20..28].copy_from_slice(&self.checkpoint.generation.to_le_bytes());
        self.header[28..32].copy_from_slice(&(STREAM_BUFFER_BYTES as u32).to_le_bytes());
        self.header_length = ARCHIVE_HEADER_BYTES;
        self.header_offset = 0;
        self.state = BackupState::VolumeHeader
    }

    fn encode_file_header(&mut self, file: FileVersion) {
        let name = file.file.as_bytes();
        self.header.fill(0);
        self.header[..4].copy_from_slice(b"FILE");
        let header_length = ENTRY_FIXED_BYTES + name.len();
        self.header[4..6].copy_from_slice(&(header_length as u16).to_le_bytes());
        self.header[6..8].copy_from_slice(&(name.len() as u16).to_le_bytes());
        self.header[8..12].copy_from_slice(&file.version.to_le_bytes());
        self.header[12..20].copy_from_slice(&file.size.to_le_bytes());
        self.header[20..28].copy_from_slice(&file.checksum.to_le_bytes());
        self.header[28..36].copy_from_slice(&file.created_at.to_le_bytes());
        self.header[ENTRY_FIXED_BYTES..header_length].copy_from_slice(name);
        self.header_length = header_length;
        self.header_offset = 0;
        self.state = BackupState::FileHeader
    }

    fn encode_trailer(&mut self) {
        self.current_file = None;
        self.header.fill(0);
        self.header[..8].copy_from_slice(TRAILER_MAGIC);
        self.header[8..12].copy_from_slice(&self.files_streamed.to_le_bytes());
        self.header_length = TRAILER_BYTES;
        self.header_offset = 0;
        self.state = BackupState::Trailer
    }
}

pub const fn archive_magic() -> [u8; 8] {
    *ARCHIVE_MAGIC
}

pub const fn archive_format_version() -> u16 {
    FORMAT_VERSION
}

pub const fn checkpoint_id(report: BackupReport) -> CheckpointId {
    report.checkpoint.id
}
