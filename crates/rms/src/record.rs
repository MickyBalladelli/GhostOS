use ghostos_status::{IntoStatus, Status};
use ghostos_ghostfs::{
    Error as SynFsError, FileVersion, MappedRecordFile, RecordDescriptor, RecordFileInfo,
    RecordImageBuilder, RecordRead, RecordSelector, RmsError, RmsMapHandle, SynFs,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordLocation {
    pub position: u32,
}

pub trait StructuredRecord: Sized {
    fn encode(&self, destination: &mut [u8]) -> Result<usize, Status>;
    fn decode(source: &[u8]) -> Result<Self, Status>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordError {
    Codec(Status),
    CountOverflow,
    Storage(RmsError),
}

impl From<RmsError> for RecordError {
    fn from(error: RmsError) -> Self {
        Self::Storage(error)
    }
}

impl From<SynFsError> for RecordError {
    fn from(error: SynFsError) -> Self {
        Self::Storage(error.into())
    }
}

impl IntoStatus for RecordError {
    fn status(self) -> Status {
        match self {
            Self::Codec(status) => status,
            Self::CountOverflow => Status::NO_SPACE,
            Self::Storage(error) => error.status(),
        }
    }
}

pub struct RecordFile<'fs, 'path, const MAX_BLOCKS: usize> {
    filesystem: &'fs mut SynFs<MAX_BLOCKS>,
    path: &'path str,
    descriptor: RecordDescriptor,
}

impl<'fs, 'path, const MAX_BLOCKS: usize> RecordFile<'fs, 'path, MAX_BLOCKS> {
    pub fn create(
        filesystem: &'fs mut SynFs<MAX_BLOCKS>,
        path: &'path str,
        descriptor: RecordDescriptor,
        records: &[&[u8]],
        scratch: &mut [u8],
    ) -> Result<(Self, RecordFileInfo), RecordError> {
        let info = filesystem.write_records(path, descriptor, records, scratch)?;
        Ok((
            Self {
                filesystem,
                path,
                descriptor,
            },
            info,
        ))
    }

    pub fn open(
        filesystem: &'fs mut SynFs<MAX_BLOCKS>,
        path: &'path str,
        scratch: &mut [u8],
    ) -> Result<Self, RecordError> {
        let info = filesystem.record_info(path, scratch)?;
        Ok(Self {
            filesystem,
            path,
            descriptor: info.descriptor,
        })
    }

    pub const fn path(&self) -> &str {
        self.path
    }

    pub const fn descriptor(&self) -> RecordDescriptor {
        self.descriptor
    }

    pub fn info(&self, scratch: &mut [u8]) -> Result<RecordFileInfo, RecordError> {
        Ok(self.filesystem.record_info(self.path, scratch)?)
    }

    pub fn read(
        &self,
        selector: RecordSelector<'_>,
        file_scratch: &mut [u8],
        destination: &mut [u8],
    ) -> Result<RecordRead, RecordError> {
        Ok(self
            .filesystem
            .read_record(self.path, selector, file_scratch, destination)?)
    }

    pub fn read_structured<T: StructuredRecord>(
        &self,
        selector: RecordSelector<'_>,
        file_scratch: &mut [u8],
        record_scratch: &mut [u8],
    ) -> Result<T, RecordError> {
        let read = self.read(selector, file_scratch, record_scratch)?;
        T::decode(&record_scratch[..read.bytes_read]).map_err(RecordError::Codec)
    }

    pub fn resolve(
        &self,
        selector: RecordSelector<'_>,
        file_scratch: &mut [u8],
    ) -> Result<RecordLocation, RecordError> {
        let image = self.load_image(file_scratch)?;
        let record = image.record(selector)?;
        for (position, candidate) in image.records().enumerate() {
            if candidate? == record {
                return Ok(RecordLocation {
                    position: position as u32,
                });
            }
        }
        Err(RecordError::Storage(RmsError::RecordNotFound))
    }

    pub fn insert(
        &mut self,
        record: &[u8],
        file_scratch: &mut [u8],
        image_scratch: &mut [u8],
    ) -> Result<RecordFileInfo, RecordError> {
        let image = self.load_image(file_scratch)?;
        let record_count = image
            .info()
            .record_count
            .checked_add(1)
            .ok_or(RecordError::CountOverflow)?;
        let mut builder = RecordImageBuilder::new(self.descriptor, record_count, image_scratch)?;
        for existing in image.records() {
            builder.push(existing?)?;
        }
        builder.push(record)?;
        let bytes = builder.finish()?;
        let file = self.filesystem.write(self.path, bytes)?;
        Ok(info(file, self.descriptor, record_count))
    }

    pub fn insert_structured<T: StructuredRecord>(
        &mut self,
        record: &T,
        file_scratch: &mut [u8],
        image_scratch: &mut [u8],
        record_scratch: &mut [u8],
    ) -> Result<RecordFileInfo, RecordError> {
        let length = record.encode(record_scratch).map_err(RecordError::Codec)?;
        if length > record_scratch.len() {
            return Err(RecordError::Codec(Status::CORRUPT));
        }
        self.insert(&record_scratch[..length], file_scratch, image_scratch)
    }

    pub fn update(
        &mut self,
        selector: RecordSelector<'_>,
        replacement: &[u8],
        file_scratch: &mut [u8],
        image_scratch: &mut [u8],
    ) -> Result<RecordFileInfo, RecordError> {
        let image = self.load_image(file_scratch)?;
        let selected = selected_position(&image, selector)?;
        let record_count = image.info().record_count;
        let mut builder = RecordImageBuilder::new(self.descriptor, record_count, image_scratch)?;
        for (position, existing) in image.records().enumerate() {
            if position as u32 == selected {
                builder.push(replacement)?;
            } else {
                builder.push(existing?)?;
            }
        }
        let bytes = builder.finish()?;
        let file = self.filesystem.write(self.path, bytes)?;
        Ok(info(file, self.descriptor, record_count))
    }

    pub fn delete(
        &mut self,
        selector: RecordSelector<'_>,
        file_scratch: &mut [u8],
        image_scratch: &mut [u8],
    ) -> Result<RecordFileInfo, RecordError> {
        let image = self.load_image(file_scratch)?;
        let selected = selected_position(&image, selector)?;
        let record_count = image
            .info()
            .record_count
            .checked_sub(1)
            .ok_or(RecordError::CountOverflow)?;
        let mut builder = RecordImageBuilder::new(self.descriptor, record_count, image_scratch)?;
        for (position, existing) in image.records().enumerate() {
            let existing = existing?;
            if position as u32 != selected {
                builder.push(existing)?;
            }
        }
        let bytes = builder.finish()?;
        let file = self.filesystem.write(self.path, bytes)?;
        Ok(info(file, self.descriptor, record_count))
    }

    fn load_image<'a>(&self, scratch: &'a mut [u8]) -> Result<MappedRecordFile<'a>, RecordError> {
        let file = self.filesystem.lookup(self.path)?;
        let required = usize::try_from(file.size)
            .map_err(|_| RecordError::Storage(RmsError::NotRecordFile))?;
        if scratch.len() < required {
            return Err(RecordError::Storage(RmsError::BufferTooSmall { required }));
        }
        self.filesystem.read(self.path, &mut scratch[..required])?;
        let capability = RmsMapHandle::from_capability(1_u64 << 32)
            .expect("synthetic in-process mapping capability");
        Ok(MappedRecordFile::open(capability, &scratch[..required])?)
    }
}

fn selected_position(
    image: &MappedRecordFile<'_>,
    selector: RecordSelector<'_>,
) -> Result<u32, RecordError> {
    let selected = image.record(selector)?;
    for (position, record) in image.records().enumerate() {
        if record? == selected {
            return Ok(position as u32);
        }
    }
    Err(RecordError::Storage(RmsError::RecordNotFound))
}

fn info(file: FileVersion, descriptor: RecordDescriptor, record_count: u32) -> RecordFileInfo {
    RecordFileInfo {
        file,
        descriptor,
        record_count,
    }
}
