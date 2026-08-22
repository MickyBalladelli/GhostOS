use crate::{Error, FileVersion, SynFs};
use ghostos_status::{IntoStatus, Severity, Status, facility};

const MAGIC: &[u8; 8] = b"SYNRMS01";
const HEADER_BYTES: usize = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IndexDefinition {
    pub key_offset: u16,
    pub key_length: u16,
    pub unique: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordOrganization {
    Sequential,
    Indexed(IndexDefinition),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordFormat {
    Fixed { length: u16 },
    Variable { maximum: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordDescriptor {
    pub organization: RecordOrganization,
    pub format: RecordFormat,
}

impl RecordDescriptor {
    pub fn validate(self) -> Result<Self, RmsError> {
        let maximum = match self.format {
            RecordFormat::Fixed { length } if length != 0 => length,
            RecordFormat::Variable { maximum } if maximum != 0 => maximum,
            _ => return Err(RmsError::InvalidDescriptor),
        };
        if let RecordOrganization::Indexed(index) = self.organization {
            if index.key_length == 0
                || index
                    .key_offset
                    .checked_add(index.key_length)
                    .is_none_or(|end| end > maximum)
            {
                return Err(RmsError::InvalidDescriptor);
            }
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordSelector<'a> {
    Position(u32),
    Key(&'a [u8]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordFileInfo {
    pub file: FileVersion,
    pub descriptor: RecordDescriptor,
    pub record_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordRead {
    pub file: RecordFileInfo,
    pub position: u32,
    pub bytes_read: usize,
}

/// Generation-checked kernel capability authorizing a read-only RMS mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct RmsMapHandle(u64);

impl RmsMapHandle {
    /// Convert a handle after the kernel has mapped its immutable GhostFS pages.
    pub const fn from_capability(raw: u64) -> Option<Self> {
        if raw >> 32 == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn from_valid_capability(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappedRecordInfo {
    pub capability: RmsMapHandle,
    pub descriptor: RecordDescriptor,
    pub record_count: u32,
    pub byte_length: usize,
}

/// In-process, zero-copy parser for capability-mapped immutable RMS pages.
///
/// The RMS daemon validates the capability and maps the current CoW data
/// snapshot read-only. Record parsing and indexed lookup then need no IPC.
pub struct MappedRecordFile<'a> {
    info: MappedRecordInfo,
    bytes: &'a [u8],
}

impl<'a> MappedRecordFile<'a> {
    pub fn open(capability: RmsMapHandle, bytes: &'a [u8]) -> Result<Self, RmsError> {
        let (descriptor, record_count) = decode_header(bytes)?;
        let mut cursor = HEADER_BYTES;
        for position in 0..record_count {
            let record = next_record(bytes, &mut cursor)?;
            validate_record(descriptor, record, position)?;
        }
        if cursor != bytes.len() {
            return Err(RmsError::NotRecordFile);
        }
        Ok(Self {
            info: MappedRecordInfo {
                capability,
                descriptor,
                record_count,
                byte_length: bytes.len(),
            },
            bytes,
        })
    }

    pub const fn info(&self) -> MappedRecordInfo {
        self.info
    }

    pub fn records(&self) -> RecordIter<'a> {
        RecordIter {
            bytes: self.bytes,
            remaining: self.info.record_count,
            cursor: HEADER_BYTES,
        }
    }

    pub fn record(&self, selector: RecordSelector<'_>) -> Result<&'a [u8], RmsError> {
        if matches!(selector, RecordSelector::Key(_))
            && matches!(
                self.info.descriptor.organization,
                RecordOrganization::Sequential
            )
        {
            return Err(RmsError::InvalidSelector);
        }
        let mut cursor = HEADER_BYTES;
        for position in 0..self.info.record_count {
            let record = next_record(self.bytes, &mut cursor)?;
            let selected = match selector {
                RecordSelector::Position(wanted) => position == wanted,
                RecordSelector::Key(wanted) => {
                    let RecordOrganization::Indexed(index) = self.info.descriptor.organization
                    else {
                        return Err(RmsError::InvalidSelector);
                    };
                    let start = index.key_offset as usize;
                    let end = start + index.key_length as usize;
                    wanted == &record[start..end]
                }
            };
            if selected {
                return Ok(record);
            }
        }
        Err(RmsError::RecordNotFound)
    }
}

pub struct RecordIter<'a> {
    bytes: &'a [u8],
    remaining: u32,
    cursor: usize,
}

impl<'a> Iterator for RecordIter<'a> {
    type Item = Result<&'a [u8], RmsError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None
        }
        self.remaining -= 1;
        Some(next_record(self.bytes, &mut self.cursor))
    }
}

/// Incremental serializer for an immutable RMS record image.
pub struct RecordImageBuilder<'a> {
    descriptor: RecordDescriptor,
    record_count: u32,
    written: u32,
    cursor: usize,
    bytes: &'a mut [u8],
}

impl<'a> RecordImageBuilder<'a> {
    pub fn new(
        descriptor: RecordDescriptor,
        record_count: u32,
        bytes: &'a mut [u8],
    ) -> Result<Self, RmsError> {
        let descriptor = descriptor.validate()?;
        if bytes.len() < HEADER_BYTES {
            return Err(RmsError::BufferTooSmall {
                required: HEADER_BYTES,
            })
        }
        encode_header(&mut bytes[..HEADER_BYTES], descriptor, record_count);
        Ok(Self {
            descriptor,
            record_count,
            written: 0,
            cursor: HEADER_BYTES,
            bytes,
        })
    }

    pub fn push(&mut self, record: &[u8]) -> Result<(), RmsError> {
        if self.written >= self.record_count {
            return Err(RmsError::InvalidRecord {
                position: self.written,
            })
        }
        validate_record(self.descriptor, record, self.written)?;
        self.validate_unique_key(record)?;
        let required = self
            .cursor
            .checked_add(2 + record.len())
            .ok_or(RmsError::BufferTooSmall {
                required: usize::MAX,
            })?;
        if self.bytes.len() < required {
            return Err(RmsError::BufferTooSmall { required })
        }
        self.bytes[self.cursor..self.cursor + 2]
            .copy_from_slice(&(record.len() as u16).to_le_bytes());
        self.cursor += 2;
        self.bytes[self.cursor..self.cursor + record.len()].copy_from_slice(record);
        self.cursor += record.len();
        self.written += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<&'a [u8], RmsError> {
        if self.written != self.record_count {
            return Err(RmsError::IncompleteImage {
                expected: self.record_count,
                written: self.written,
            })
        }
        Ok(&self.bytes[..self.cursor])
    }

    fn validate_unique_key(&self, record: &[u8]) -> Result<(), RmsError> {
        let RecordOrganization::Indexed(index) = self.descriptor.organization else {
            return Ok(())
        };
        if !index.unique {
            return Ok(())
        }
        let start = index.key_offset as usize;
        let end = start + index.key_length as usize;
        let key = &record[start..end];
        let mut cursor = HEADER_BYTES;
        for _ in 0..self.written {
            let existing = next_record(&self.bytes[..self.cursor], &mut cursor)?;
            if &existing[start..end] == key {
                return Err(RmsError::DuplicateKey)
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RmsError {
    BufferTooSmall { required: usize },
    DuplicateKey,
    File(Error),
    InvalidDescriptor,
    IncompleteImage { expected: u32, written: u32 },
    InvalidRecord { position: u32 },
    InvalidSelector,
    NotRecordFile,
    RecordNotFound,
}

impl From<Error> for RmsError {
    fn from(error: Error) -> Self {
        Self::File(error)
    }
}

impl IntoStatus for RmsError {
    fn status(self) -> Status {
        match self {
            Self::File(error) => error.status(),
            Self::BufferTooSmall { .. } => {
                Status::new(Severity::Error, facility::RMS, 1, 0)
                    .unwrap_or(Status::INVALID_ARGUMENT)
            }
            Self::DuplicateKey => {
                Status::new(Severity::Error, facility::RMS, 2, 0)
                    .unwrap_or(Status::INVALID_ARGUMENT)
            }
            Self::IncompleteImage { .. }
            | Self::InvalidDescriptor
            | Self::InvalidRecord { .. }
            | Self::InvalidSelector => {
                Status::INVALID_ARGUMENT
            }
            Self::NotRecordFile => {
                Status::new(Severity::Error, facility::RMS, 3, 0)
                    .unwrap_or(Status::INVALID_ARGUMENT)
            }
            Self::RecordNotFound => Status::NOT_FOUND,
        }
    }
}

impl<const MAX_BLOCKS: usize> SynFs<MAX_BLOCKS> {
    /// Store a versioned RMS file. `scratch` receives the on-disk record image.
    pub fn write_records(
        &mut self,
        path: &str,
        descriptor: RecordDescriptor,
        records: &[&[u8]],
        scratch: &mut [u8],
    ) -> Result<RecordFileInfo, RmsError> {
        let descriptor = descriptor.validate()?;
        let required = records.iter().try_fold(HEADER_BYTES, |size, record| {
            size.checked_add(2 + record.len())
                .ok_or(RmsError::BufferTooSmall {
                    required: usize::MAX,
                })
        })?;
        if scratch.len() < required {
            return Err(RmsError::BufferTooSmall { required });
        }
        if records.len() > u32::MAX as usize {
            return Err(RmsError::InvalidDescriptor);
        }
        for (position, record) in records.iter().enumerate() {
            validate_record(descriptor, record, position as u32)?;
        }
        validate_unique_keys(descriptor, records)?;

        encode_header(
            &mut scratch[..HEADER_BYTES],
            descriptor,
            records.len() as u32,
        );
        let mut cursor = HEADER_BYTES;
        for record in records {
            let length = record.len() as u16;
            scratch[cursor..cursor + 2].copy_from_slice(&length.to_le_bytes());
            cursor += 2;
            scratch[cursor..cursor + record.len()].copy_from_slice(record);
            cursor += record.len()
        }
        let file = self.write(path, &scratch[..cursor])?;
        Ok(RecordFileInfo {
            file,
            descriptor,
            record_count: records.len() as u32,
        })
    }

    pub fn record_info(&self, path: &str, scratch: &mut [u8]) -> Result<RecordFileInfo, RmsError> {
        let file = self.lookup(path)?;
        let required = usize::try_from(file.size).map_err(|_| RmsError::NotRecordFile)?;
        if scratch.len() < required {
            return Err(RmsError::BufferTooSmall { required });
        }
        self.read(path, &mut scratch[..required])?;
        let (descriptor, record_count) = decode_header(&scratch[..required])?;
        Ok(RecordFileInfo {
            file,
            descriptor,
            record_count,
        })
    }

    /// Read by ordinal position or by an indexed key.
    pub fn read_record(
        &self,
        path: &str,
        selector: RecordSelector<'_>,
        scratch: &mut [u8],
        destination: &mut [u8],
    ) -> Result<RecordRead, RmsError> {
        let info = self.record_info(path, scratch)?;
        if matches!(selector, RecordSelector::Key(_))
            && matches!(info.descriptor.organization, RecordOrganization::Sequential)
        {
            return Err(RmsError::InvalidSelector);
        }

        let required = usize::try_from(info.file.size).map_err(|_| RmsError::NotRecordFile)?;
        let mut cursor = HEADER_BYTES;
        for position in 0..info.record_count {
            let record = next_record(&scratch[..required], &mut cursor)?;
            validate_record(info.descriptor, record, position)?;
            let selected = match selector {
                RecordSelector::Position(wanted) => position == wanted,
                RecordSelector::Key(wanted) => {
                    let RecordOrganization::Indexed(index) = info.descriptor.organization else {
                        return Err(RmsError::InvalidSelector);
                    };
                    let start = index.key_offset as usize;
                    let end = start + index.key_length as usize;
                    wanted == &record[start..end]
                }
            };
            if selected {
                if destination.len() < record.len() {
                    return Err(RmsError::BufferTooSmall {
                        required: record.len(),
                    });
                }
                destination[..record.len()].copy_from_slice(record);
                return Ok(RecordRead {
                    file: info,
                    position,
                    bytes_read: record.len(),
                });
            }
        }
        Err(RmsError::RecordNotFound)
    }
}

fn validate_record(
    descriptor: RecordDescriptor,
    record: &[u8],
    position: u32,
) -> Result<(), RmsError> {
    let valid = match descriptor.format {
        RecordFormat::Fixed { length } => record.len() == length as usize,
        RecordFormat::Variable { maximum } => {
            !record.is_empty()
                && record.len() <= maximum as usize
                && record.len() <= u16::MAX as usize
        }
    };
    if !valid {
        return Err(RmsError::InvalidRecord { position });
    }
    if let RecordOrganization::Indexed(index) = descriptor.organization {
        let end = index.key_offset as usize + index.key_length as usize;
        if end > record.len() {
            return Err(RmsError::InvalidRecord { position });
        }
    }
    Ok(())
}

fn validate_unique_keys(descriptor: RecordDescriptor, records: &[&[u8]]) -> Result<(), RmsError> {
    let RecordOrganization::Indexed(index) = descriptor.organization else {
        return Ok(());
    };
    if !index.unique {
        return Ok(());
    }
    let start = index.key_offset as usize;
    let end = start + index.key_length as usize;
    for left in 0..records.len() {
        for right in left + 1..records.len() {
            if records[left][start..end] == records[right][start..end] {
                return Err(RmsError::DuplicateKey);
            }
        }
    }
    Ok(())
}

fn encode_header(header: &mut [u8], descriptor: RecordDescriptor, record_count: u32) {
    header.fill(0);
    header[..8].copy_from_slice(MAGIC);
    match descriptor.organization {
        RecordOrganization::Sequential => header[8] = 0,
        RecordOrganization::Indexed(index) => {
            header[8] = 1;
            header[10..12].copy_from_slice(&index.key_offset.to_le_bytes());
            header[12..14].copy_from_slice(&index.key_length.to_le_bytes());
            header[14] = u8::from(index.unique)
        }
    }
    match descriptor.format {
        RecordFormat::Fixed { length } => {
            header[9] = 0;
            header[16..18].copy_from_slice(&length.to_le_bytes())
        }
        RecordFormat::Variable { maximum } => {
            header[9] = 1;
            header[16..18].copy_from_slice(&maximum.to_le_bytes())
        }
    }
    header[20..24].copy_from_slice(&record_count.to_le_bytes())
}

fn decode_header(bytes: &[u8]) -> Result<(RecordDescriptor, u32), RmsError> {
    if bytes.len() < HEADER_BYTES || &bytes[..8] != MAGIC {
        return Err(RmsError::NotRecordFile);
    }
    let organization = match bytes[8] {
        0 => RecordOrganization::Sequential,
        1 => RecordOrganization::Indexed(IndexDefinition {
            key_offset: u16::from_le_bytes([bytes[10], bytes[11]]),
            key_length: u16::from_le_bytes([bytes[12], bytes[13]]),
            unique: bytes[14] != 0,
        }),
        _ => return Err(RmsError::NotRecordFile),
    };
    let length = u16::from_le_bytes([bytes[16], bytes[17]]);
    let format = match bytes[9] {
        0 => RecordFormat::Fixed { length },
        1 => RecordFormat::Variable { maximum: length },
        _ => return Err(RmsError::NotRecordFile),
    };
    let descriptor = RecordDescriptor {
        organization,
        format,
    }
    .validate()?;
    let record_count = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Ok((descriptor, record_count))
}

fn next_record<'a>(bytes: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], RmsError> {
    let length_bytes = bytes
        .get(*cursor..*cursor + 2)
        .ok_or(RmsError::NotRecordFile)?;
    let length = u16::from_le_bytes([length_bytes[0], length_bytes[1]]) as usize;
    *cursor += 2;
    let record = bytes
        .get(*cursor..*cursor + length)
        .ok_or(RmsError::NotRecordFile)?;
    *cursor += length;
    Ok(record)
}
