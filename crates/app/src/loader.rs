//! Native ELF image validation and mapping contract.
//!
//! The parser is Ring 3 safe code. A kernel-backed [`ImageMapper`] performs
//! the privileged page operations, so this crate never needs raw pointers or
//! executable memory in its address space.

use synos_status::{IntoStatus, Status};
use synos_system_model::ContentId;

pub const PAGE_SIZE: u64 = 4096;
pub const MAX_LOAD_SEGMENTS: usize = 16;
pub const MAX_RELOCATIONS: usize = 256;
pub const MAX_EXECUTABLE_PAGES: usize = 16_384;
pub const MAX_ARGUMENTS: usize = 32;
pub const MAX_ENVIRONMENT: usize = 64;
pub const MAX_STACK_ARGUMENT_BYTES: usize = 64 * 1024;
pub const DEFAULT_STACK_BYTES: u64 = 1024 * 1024;
pub const DEFAULT_HEAP_BYTES: u64 = 64 * 1024;
pub const DEFAULT_GUARD_PAGES: u8 = 1;

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u32 = 1;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const EM_X86_64: u16 = 62;
const EM_AARCH64: u16 = 183;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_TLS: u32 = 7;
const PT_GNU_STACK: u32 = 0x6474e551;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;
const SHT_RELA: u32 = 4;
const SHT_REL: u32 = 9;
const R_X86_64_RELATIVE: u32 = 8;
const R_AARCH64_RELATIVE: u32 = 1_027;
const DT_NULL: i64 = 0;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;
const DT_RELAENT: i64 = 9;
const DT_REL: i64 = 17;
const DT_RELSZ: i64 = 18;
const DT_RELENT: i64 = 19;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ImageArchitecture {
    X86_64 = 1,
    Aarch64 = 2,
}

impl ImageArchitecture {
    pub const fn machine(self) -> u16 {
        match self {
            Self::X86_64 => EM_X86_64,
            Self::Aarch64 => EM_AARCH64,
        }
    }

    pub const fn relative_relocation(self) -> u32 {
        match self {
            Self::X86_64 => R_X86_64_RELATIVE,
            Self::Aarch64 => R_AARCH64_RELATIVE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ImageFormat {
    Fixed = 1,
    PositionIndependent = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct SegmentPermissions(u8);

impl SegmentPermissions {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);

    pub const fn new(flags: u32) -> Option<Self> {
        if flags & !(PF_R | PF_W | PF_X) != 0 || (flags & (PF_W | PF_X)) == (PF_W | PF_X) {
            None
        } else {
            Some(Self(
                ((flags & PF_R != 0) as u8) * Self::READ.0
                    | ((flags & PF_W != 0) as u8) * Self::WRITE.0
                    | ((flags & PF_X != 0) as u8) * Self::EXECUTE.0,
            ))
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn readable(self) -> bool {
        self.0 & Self::READ.0 != 0
    }

    pub const fn writable(self) -> bool {
        self.0 & Self::WRITE.0 != 0
    }

    pub const fn executable(self) -> bool {
        self.0 & Self::EXECUTE.0 != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadSegment {
    pub virtual_address: u64,
    pub file_offset: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub alignment: u64,
    pub permissions: SegmentPermissions,
}

impl LoadSegment {
    pub const fn file_end(self) -> u64 {
        self.file_offset.saturating_add(self.file_size)
    }

    pub const fn memory_end(self) -> u64 {
        self.virtual_address.saturating_add(self.memory_size)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TlsImage {
    pub file_offset: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub alignment: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelativeRelocation {
    pub virtual_address: u64,
    pub addend: i64,
    pub addend_from_memory: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageLayout {
    pub architecture: ImageArchitecture,
    pub format: ImageFormat,
    pub entry: u64,
    pub image_base: u64,
    pub image_end: u64,
    pub segments: [Option<LoadSegment>; MAX_LOAD_SEGMENTS],
    pub segment_count: u8,
    pub relocations: [Option<RelativeRelocation>; MAX_RELOCATIONS],
    pub relocation_count: u16,
    pub tls: Option<TlsImage>,
    pub payload_measurement: ContentId,
    pub executable_page_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoaderError {
    Truncated,
    InvalidHeader,
    UnsupportedClass,
    UnsupportedEndian,
    UnsupportedMachine,
    UnsupportedFormat,
    InvalidProgramHeader,
    InvalidSegment,
    SegmentOverflow,
    SegmentCapacity,
    MissingExecutableEntry,
    WritableExecutable,
    ExecutableStack,
    InvalidTls,
    InvalidSectionHeader,
    RelocationCapacity,
    UnsupportedRelocation,
    InvalidRelocation,
    MappingFailed,
    MappingOverflow,
    InvalidArguments,
    ResourceLimit,
    ProcessNotFound,
    InvalidTransition,
    BackendFailure,
    MeasurementMismatch,
    Capacity,
}

impl IntoStatus for LoaderError {
    fn status(self) -> Status {
        match self {
            Self::Truncated
            | Self::InvalidHeader
            | Self::UnsupportedClass
            | Self::UnsupportedEndian
            | Self::UnsupportedMachine
            | Self::UnsupportedFormat
            | Self::InvalidProgramHeader
            | Self::InvalidSegment
            | Self::SegmentOverflow
            | Self::MissingExecutableEntry
            | Self::WritableExecutable
            | Self::ExecutableStack
            | Self::InvalidTls
            | Self::InvalidSectionHeader
            | Self::UnsupportedRelocation
            | Self::InvalidRelocation
            | Self::InvalidArguments
            | Self::ResourceLimit
            | Self::InvalidTransition => Status::INVALID_ARGUMENT,
            Self::SegmentCapacity | Self::RelocationCapacity | Self::Capacity => Status::NO_SPACE,
            Self::MappingFailed | Self::MappingOverflow | Self::ProcessNotFound => Status::BUSY,
            Self::BackendFailure => Status::BUSY,
            Self::MeasurementMismatch => Status::CORRUPT,
        }
    }
}

pub fn parse_image(
    bytes: &[u8],
    expected_architecture: ImageArchitecture,
) -> Result<ImageLayout, LoaderError> {
    if bytes.len() < 64 {
        return Err(LoaderError::Truncated);
    }
    if &bytes[..4] != ELF_MAGIC {
        return Err(LoaderError::InvalidHeader);
    }
    if bytes[4] != ELFCLASS64 {
        return Err(LoaderError::UnsupportedClass);
    }
    if bytes[5] != ELFDATA2LSB {
        return Err(LoaderError::UnsupportedEndian);
    }
    if read_u32(bytes, 20)? != EV_CURRENT {
        return Err(LoaderError::InvalidHeader);
    }

    let image_type = read_u16(bytes, 16)?;
    let machine = read_u16(bytes, 18)?;
    if machine != expected_architecture.machine() {
        return Err(LoaderError::UnsupportedMachine);
    }
    let format = match image_type {
        ET_EXEC => ImageFormat::Fixed,
        ET_DYN => ImageFormat::PositionIndependent,
        _ => return Err(LoaderError::UnsupportedFormat),
    };
    if read_u16(bytes, 52)? != 64 {
        return Err(LoaderError::InvalidHeader);
    }

    let entry = read_u64(bytes, 24)?;
    let program_offset = read_u64(bytes, 32)?;
    let program_entry_size = read_u16(bytes, 54)?;
    let program_count = read_u16(bytes, 56)? as usize;
    if program_entry_size != 56 || program_count > 128 {
        return Err(LoaderError::InvalidProgramHeader);
    }
    range(
        bytes,
        program_offset,
        (program_count as u64)
            .checked_mul(56)
            .ok_or(LoaderError::SegmentOverflow)?,
    )?;

    let mut segments = [None; MAX_LOAD_SEGMENTS];
    let mut segment_count = 0usize;
    let mut image_base = u64::MAX;
    let mut image_end = 0u64;
    let mut tls = None;
    let mut dynamic = None;
    let mut executable_entry = false;

    for index in 0..program_count {
        let offset = program_offset
            .checked_add((index as u64) * 56)
            .ok_or(LoaderError::SegmentOverflow)? as usize;
        let kind = read_u32(bytes, offset)?;
        let flags = read_u32(bytes, offset + 4)?;
        let file_offset = read_u64(bytes, offset + 8)?;
        let virtual_address = read_u64(bytes, offset + 16)?;
        let file_size = read_u64(bytes, offset + 32)?;
        let memory_size = read_u64(bytes, offset + 40)?;
        let alignment = read_u64(bytes, offset + 48)?;

        match kind {
            PT_LOAD => {
                if segment_count == MAX_LOAD_SEGMENTS {
                    return Err(LoaderError::SegmentCapacity);
                }
                if memory_size < file_size
                    || range(bytes, file_offset, file_size).is_err()
                    || virtual_address.checked_add(memory_size).is_none()
                {
                    return Err(LoaderError::InvalidSegment);
                }
                if alignment > 1
                    && (!alignment.is_power_of_two()
                        || virtual_address % alignment != file_offset % alignment)
                {
                    return Err(LoaderError::InvalidSegment);
                }
                if virtual_address % PAGE_SIZE != file_offset % PAGE_SIZE {
                    return Err(LoaderError::InvalidSegment);
                }
                let permissions =
                    SegmentPermissions::new(flags).ok_or(LoaderError::WritableExecutable)?;
                if permissions.bits() == 0 {
                    return Err(LoaderError::InvalidSegment);
                }
                let segment = LoadSegment {
                    virtual_address,
                    file_offset,
                    file_size,
                    memory_size,
                    alignment: alignment.max(PAGE_SIZE),
                    permissions,
                };
                image_base = image_base.min(align_down(virtual_address));
                image_end = image_end.max(align_up(segment.memory_end())?);
                if permissions.executable()
                    && entry >= virtual_address
                    && entry < segment.memory_end()
                {
                    executable_entry = true;
                }
                segments[segment_count] = Some(segment);
                segment_count += 1;
            }
            PT_TLS => {
                if tls.is_some()
                    || memory_size < file_size
                    || range(bytes, file_offset, file_size).is_err()
                    || alignment > 1 && !alignment.is_power_of_two()
                {
                    return Err(LoaderError::InvalidTls);
                }
                tls = Some(TlsImage {
                    file_offset,
                    file_size,
                    memory_size,
                    alignment: alignment.max(1),
                });
            }
            PT_DYNAMIC => {
                dynamic = Some((file_offset, file_size));
            }
            PT_GNU_STACK if flags & PF_X != 0 => return Err(LoaderError::ExecutableStack),
            _ => {}
        }
    }

    if segment_count == 0 || !executable_entry || image_base == u64::MAX {
        return Err(LoaderError::MissingExecutableEntry);
    }
    validate_non_overlapping(&segments, segment_count)?;

    let mut relocations = [None; MAX_RELOCATIONS];
    let mut relocation_count = 0usize;
    parse_section_relocations(
        bytes,
        expected_architecture,
        &segments,
        segment_count,
        &mut relocations,
        &mut relocation_count,
    )?;
    if relocation_count == 0 {
        if let Some(dynamic) = dynamic {
            parse_dynamic_relocations(
                bytes,
                dynamic,
                expected_architecture,
                &segments,
                segment_count,
                &mut relocations,
                &mut relocation_count,
            )?;
        }
    }

    let executable_page_count = executable_pages(&segments, segment_count)?;
    Ok(ImageLayout {
        architecture: expected_architecture,
        format,
        entry,
        image_base,
        image_end,
        segments,
        segment_count: segment_count as u8,
        relocations,
        relocation_count: relocation_count as u16,
        tls,
        payload_measurement: ContentId::hash(bytes),
        executable_page_count,
    })
}

pub trait PageMeasurer {
    type Error;

    fn executable_page(
        &mut self,
        virtual_address: u64,
        page: &[u8; PAGE_SIZE as usize],
    ) -> Result<(), Self::Error>;

    fn finish(&mut self) -> Result<ContentId, Self::Error>;
}

pub fn measure_executable_pages<M: PageMeasurer>(
    bytes: &[u8],
    layout: &ImageLayout,
    measurer: &mut M,
) -> Result<ContentId, LoaderError> {
    let mut page = [0u8; PAGE_SIZE as usize];
    for segment in layout
        .segments
        .iter()
        .flatten()
        .take(layout.segment_count as usize)
    {
        if !segment.permissions.executable() {
            continue;
        }
        let start = align_down(segment.virtual_address);
        let end = align_up(segment.memory_end())?;
        let mut address = start;
        while address < end {
            page.fill(0);
            let page_end = address + PAGE_SIZE;
            let copy_start = address.max(segment.virtual_address);
            let copy_end = page_end.min(segment.virtual_address + segment.file_size);
            if copy_start < copy_end {
                let source_offset = segment
                    .file_offset
                    .checked_add(copy_start - segment.virtual_address)
                    .ok_or(LoaderError::SegmentOverflow)?;
                let source_length = (copy_end - copy_start) as usize;
                let source = range(bytes, source_offset, source_length as u64)?;
                let destination = (copy_start - address) as usize;
                page[destination..destination + source_length].copy_from_slice(source);
            }
            measurer
                .executable_page(address, &page)
                .map_err(|_| LoaderError::BackendFailure)?;
            address = page_end;
        }
    }
    measurer.finish().map_err(|_| LoaderError::BackendFailure)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappingRequest {
    pub size: u64,
    pub alignment: u64,
    pub preferred_base: Option<u64>,
    pub fixed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mapping {
    pub base: u64,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeSegment {
    pub address: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub permissions: SegmentPermissions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StackRequest {
    pub size: u64,
    pub guard_pages: u8,
    pub executable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TlsRequest {
    pub file_size: u64,
    pub memory_size: u64,
    pub alignment: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessContext {
    pub entry: u64,
    pub stack_pointer: u64,
    pub tls_pointer: Option<u64>,
    pub heap_base: u64,
    pub heap_size: u64,
}

pub struct ImageLoadRequest<'a> {
    pub bytes: &'a [u8],
    pub architecture: ImageArchitecture,
    pub expected_payload: Option<ContentId>,
    pub heap_bytes: u64,
    pub stack: StackRequest,
    pub arguments: ProcessArguments<'a>,
}

pub trait ImageMapper {
    type Error;

    fn reserve(&mut self, request: MappingRequest) -> Result<Mapping, Self::Error>;
    fn map_segment(
        &mut self,
        mapping: Mapping,
        segment: RuntimeSegment,
        source: &[u8],
    ) -> Result<(), Self::Error>;
    fn zero_fill(&mut self, mapping: Mapping, address: u64, length: u64)
        -> Result<(), Self::Error>;
    fn apply_relative_relocation(
        &mut self,
        mapping: Mapping,
        address: u64,
        addend: i64,
        addend_from_memory: bool,
    ) -> Result<(), Self::Error>;
    fn protect(
        &mut self,
        mapping: Mapping,
        address: u64,
        length: u64,
        permissions: SegmentPermissions,
    ) -> Result<(), Self::Error>;
    fn allocate_stack(
        &mut self,
        mapping: Mapping,
        request: StackRequest,
        arguments: ProcessArguments<'_>,
    ) -> Result<u64, Self::Error>;
    fn allocate_heap(&mut self, mapping: Mapping, size: u64) -> Result<u64, Self::Error>;
    fn allocate_tls(
        &mut self,
        mapping: Mapping,
        request: TlsRequest,
        source: &[u8],
    ) -> Result<u64, Self::Error>;
    fn install_context(
        &mut self,
        mapping: Mapping,
        context: ProcessContext,
    ) -> Result<(), Self::Error>;
    fn release(&mut self, mapping: Mapping);
}

pub struct LoadedImage {
    pub layout: ImageLayout,
    pub mapping: Mapping,
    pub context: ProcessContext,
}

pub fn load_image<M: ImageMapper>(
    mapper: &mut M,
    request: ImageLoadRequest<'_>,
) -> Result<LoadedImage, LoaderError> {
    request.arguments.validate()?;
    if request.stack.size == 0 || request.stack.executable {
        return Err(LoaderError::ResourceLimit);
    }
    if request.heap_bytes == 0 {
        return Err(LoaderError::ResourceLimit);
    }
    let layout = parse_image(request.bytes, request.architecture)?;
    if request
        .expected_payload
        .is_some_and(|expected| expected != layout.payload_measurement)
    {
        return Err(LoaderError::MeasurementMismatch);
    }
    let mapping = mapper
        .reserve(MappingRequest {
            size: layout.image_end - layout.image_base,
            alignment: PAGE_SIZE,
            preferred_base: (layout.format == ImageFormat::Fixed).then_some(layout.image_base),
            fixed: layout.format == ImageFormat::Fixed,
        })
        .map_err(|_| LoaderError::MappingFailed)?;
    if mapping.size < layout.image_end - layout.image_base
        || layout.format == ImageFormat::Fixed && mapping.base != layout.image_base
    {
        mapper.release(mapping);
        return Err(LoaderError::MappingOverflow);
    }

    let result = (|| {
        let load_bias = mapping
            .base
            .checked_sub(layout.image_base)
            .ok_or(LoaderError::MappingOverflow)?;
        for segment in layout
            .segments
            .iter()
            .flatten()
            .take(layout.segment_count as usize)
        {
            let address = load_bias
                .checked_add(segment.virtual_address)
                .ok_or(LoaderError::MappingOverflow)?;
            let source = range(request.bytes, segment.file_offset, segment.file_size)?;
            mapper
                .map_segment(
                    mapping,
                    RuntimeSegment {
                        address,
                        file_size: segment.file_size,
                        memory_size: segment.memory_size,
                        permissions: segment.permissions,
                    },
                    source,
                )
                .map_err(|_| LoaderError::MappingFailed)?;
            if segment.memory_size > segment.file_size {
                mapper
                    .zero_fill(
                        mapping,
                        address + segment.file_size,
                        segment.memory_size - segment.file_size,
                    )
                    .map_err(|_| LoaderError::MappingFailed)?;
            }
        }
        let heap_base = mapper
            .allocate_heap(mapping, request.heap_bytes)
            .map_err(|_| LoaderError::MappingFailed)?;
        for relocation in layout
            .relocations
            .iter()
            .flatten()
            .take(layout.relocation_count as usize)
        {
            let address = load_bias
                .checked_add(relocation.virtual_address)
                .ok_or(LoaderError::MappingOverflow)?;
            mapper
                .apply_relative_relocation(
                    mapping,
                    address,
                    relocation.addend,
                    relocation.addend_from_memory,
                )
                .map_err(|_| LoaderError::MappingFailed)?;
        }
        for segment in layout
            .segments
            .iter()
            .flatten()
            .take(layout.segment_count as usize)
        {
            let address = load_bias
                .checked_add(align_down(segment.virtual_address))
                .ok_or(LoaderError::MappingOverflow)?;
            mapper
                .protect(
                    mapping,
                    address,
                    align_up(segment.memory_end())? - align_down(segment.virtual_address),
                    segment.permissions,
                )
                .map_err(|_| LoaderError::MappingFailed)?;
        }

        let stack_pointer = mapper
            .allocate_stack(mapping, request.stack, request.arguments)
            .map_err(|_| LoaderError::MappingFailed)?;
        let tls_pointer = if let Some(tls) = layout.tls {
            let source = range(request.bytes, tls.file_offset, tls.file_size)?;
            Some(
                mapper
                    .allocate_tls(
                        mapping,
                        TlsRequest {
                            file_size: tls.file_size,
                            memory_size: tls.memory_size,
                            alignment: tls.alignment,
                        },
                        source,
                    )
                    .map_err(|_| LoaderError::MappingFailed)?,
            )
        } else {
            None
        };
        let context = ProcessContext {
            entry: load_bias
                .checked_add(layout.entry)
                .ok_or(LoaderError::MappingOverflow)?,
            stack_pointer,
            tls_pointer,
            heap_base,
            heap_size: request.heap_bytes,
        };
        mapper
            .install_context(mapping, context)
            .map_err(|_| LoaderError::MappingFailed)?;
        Ok(LoadedImage {
            layout,
            mapping,
            context,
        })
    })();
    if result.is_err() {
        mapper.release(mapping);
    }
    result
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessArguments<'a> {
    pub argv: &'a [&'a str],
    pub environment: &'a [(&'a str, &'a str)],
}

impl ProcessArguments<'_> {
    pub fn validate(self) -> Result<(), LoaderError> {
        if self.argv.len() > MAX_ARGUMENTS || self.environment.len() > MAX_ENVIRONMENT {
            return Err(LoaderError::InvalidArguments);
        }
        let mut bytes = 0usize;
        for argument in self.argv {
            bytes = bytes
                .checked_add(argument.len() + 1)
                .ok_or(LoaderError::InvalidArguments)?;
            if argument.as_bytes().contains(&0) {
                return Err(LoaderError::InvalidArguments);
            }
        }
        for (key, value) in self.environment {
            bytes = bytes
                .checked_add(key.len() + value.len() + 2)
                .ok_or(LoaderError::InvalidArguments)?;
            if key.is_empty() || key.as_bytes().contains(&0) || value.as_bytes().contains(&0) {
                return Err(LoaderError::InvalidArguments);
            }
        }
        if bytes > MAX_STACK_ARGUMENT_BYTES {
            return Err(LoaderError::ResourceLimit);
        }
        Ok(())
    }
}

fn parse_section_relocations(
    bytes: &[u8],
    architecture: ImageArchitecture,
    segments: &[Option<LoadSegment>; MAX_LOAD_SEGMENTS],
    segment_count: usize,
    relocations: &mut [Option<RelativeRelocation>; MAX_RELOCATIONS],
    relocation_count: &mut usize,
) -> Result<(), LoaderError> {
    let section_offset = read_u64(bytes, 40)?;
    let section_entry_size = read_u16(bytes, 58)?;
    let section_count = read_u16(bytes, 60)? as usize;
    if section_count == 0 {
        return Ok(());
    }
    if section_entry_size != 64 || section_count > 256 {
        return Err(LoaderError::InvalidSectionHeader);
    }
    range(
        bytes,
        section_offset,
        (section_count as u64)
            .checked_mul(64)
            .ok_or(LoaderError::SegmentOverflow)?,
    )?;
    for index in 0..section_count {
        let offset = (section_offset + index as u64 * 64) as usize;
        let kind = read_u32(bytes, offset + 4)?;
        if kind != SHT_RELA && kind != SHT_REL {
            continue;
        }
        let table_offset = read_u64(bytes, offset + 24)?;
        let table_size = read_u64(bytes, offset + 32)?;
        let entry_size = read_u64(bytes, offset + 56)?;
        let expected = if kind == SHT_RELA { 24 } else { 16 };
        if entry_size != expected || table_size % entry_size != 0 {
            return Err(LoaderError::InvalidSectionHeader);
        }
        let count = (table_size / entry_size) as usize;
        range(bytes, table_offset, table_size)?;
        for relocation_index in 0..count {
            let relocation_offset = table_offset + relocation_index as u64 * entry_size;
            let virtual_address = read_u64(bytes, relocation_offset as usize)?;
            let info = read_u64(bytes, relocation_offset as usize + 8)?;
            let addend = if kind == SHT_RELA {
                read_i64(bytes, relocation_offset as usize + 16)?
            } else {
                0
            };
            add_relocation(
                architecture,
                segments,
                segment_count,
                relocations,
                relocation_count,
                virtual_address,
                info,
                addend,
                kind == SHT_REL,
            )?;
        }
    }
    Ok(())
}

fn parse_dynamic_relocations(
    bytes: &[u8],
    dynamic: (u64, u64),
    architecture: ImageArchitecture,
    segments: &[Option<LoadSegment>; MAX_LOAD_SEGMENTS],
    segment_count: usize,
    relocations: &mut [Option<RelativeRelocation>; MAX_RELOCATIONS],
    relocation_count: &mut usize,
) -> Result<(), LoaderError> {
    let dynamic_bytes = range(bytes, dynamic.0, dynamic.1)?;
    let mut rela = None;
    let mut relasz = 0u64;
    let mut relaent = 24u64;
    let mut rel = None;
    let mut relsz = 0u64;
    let mut relent = 16u64;
    let mut chunks = dynamic_bytes.chunks_exact(16);
    for chunk in &mut chunks {
        let tag = read_i64(chunk, 0)?;
        let value = read_u64(chunk, 8)?;
        match tag {
            DT_NULL => break,
            DT_RELA => rela = Some(value),
            DT_RELASZ => relasz = value,
            DT_RELAENT => relaent = value,
            DT_REL => rel = Some(value),
            DT_RELSZ => relsz = value,
            DT_RELENT => relent = value,
            _ => {}
        }
    }
    if !chunks.remainder().is_empty() {
        return Err(LoaderError::InvalidSectionHeader)
    }
    if let Some(address) = rela {
        let file_offset = virtual_to_file(bytes, segments, segment_count, address)?;
        parse_dynamic_table(
            bytes,
            file_offset,
            relasz,
            relaent,
            false,
            architecture,
            segments,
            segment_count,
            relocations,
            relocation_count,
        )?;
    }
    if let Some(address) = rel {
        let file_offset = virtual_to_file(bytes, segments, segment_count, address)?;
        parse_dynamic_table(
            bytes,
            file_offset,
            relsz,
            relent,
            true,
            architecture,
            segments,
            segment_count,
            relocations,
            relocation_count,
        )?;
    }
    Ok(())
}

fn parse_dynamic_table(
    bytes: &[u8],
    offset: u64,
    size: u64,
    entry_size: u64,
    addend_from_memory: bool,
    architecture: ImageArchitecture,
    segments: &[Option<LoadSegment>; MAX_LOAD_SEGMENTS],
    segment_count: usize,
    relocations: &mut [Option<RelativeRelocation>; MAX_RELOCATIONS],
    relocation_count: &mut usize,
) -> Result<(), LoaderError> {
    if entry_size != if addend_from_memory { 16 } else { 24 } || size % entry_size != 0 {
        return Err(LoaderError::InvalidSectionHeader);
    }
    range(bytes, offset, size)?;
    for index in 0..(size / entry_size) as usize {
        let entry = offset + index as u64 * entry_size;
        let virtual_address = read_u64(bytes, entry as usize)?;
        let info = read_u64(bytes, entry as usize + 8)?;
        let addend = if addend_from_memory {
            0
        } else {
            read_i64(bytes, entry as usize + 16)?
        };
        add_relocation(
            architecture,
            segments,
            segment_count,
            relocations,
            relocation_count,
            virtual_address,
            info,
            addend,
            addend_from_memory,
        )?;
    }
    Ok(())
}

fn add_relocation(
    architecture: ImageArchitecture,
    segments: &[Option<LoadSegment>; MAX_LOAD_SEGMENTS],
    segment_count: usize,
    relocations: &mut [Option<RelativeRelocation>; MAX_RELOCATIONS],
    relocation_count: &mut usize,
    virtual_address: u64,
    info: u64,
    addend: i64,
    addend_from_memory: bool,
) -> Result<(), LoaderError> {
    let symbol = info >> 32;
    let kind = info as u32;
    if symbol != 0 || kind != architecture.relative_relocation() {
        return Err(LoaderError::UnsupportedRelocation);
    }
    if !contains_address(segments, segment_count, virtual_address, 8) {
        return Err(LoaderError::InvalidRelocation);
    }
    if *relocation_count == MAX_RELOCATIONS {
        return Err(LoaderError::RelocationCapacity);
    }
    relocations[*relocation_count] = Some(RelativeRelocation {
        virtual_address,
        addend,
        addend_from_memory,
    });
    *relocation_count += 1;
    Ok(())
}

fn validate_non_overlapping(
    segments: &[Option<LoadSegment>; MAX_LOAD_SEGMENTS],
    count: usize,
) -> Result<(), LoaderError> {
    for (index, left) in segments.iter().flatten().take(count).enumerate() {
        for right in segments.iter().flatten().take(count).skip(index + 1) {
            if left.virtual_address < right.memory_end()
                && right.virtual_address < left.memory_end()
            {
                return Err(LoaderError::InvalidSegment);
            }
        }
    }
    Ok(())
}

fn executable_pages(
    segments: &[Option<LoadSegment>; MAX_LOAD_SEGMENTS],
    count: usize,
) -> Result<u32, LoaderError> {
    let mut pages = 0u64;
    for segment in segments.iter().flatten().take(count) {
        if segment.permissions.executable() {
            pages = pages
                .checked_add(
                    (align_up(segment.memory_end())? - align_down(segment.virtual_address))
                        / PAGE_SIZE,
                )
                .ok_or(LoaderError::SegmentOverflow)?;
        }
    }
    if pages > MAX_EXECUTABLE_PAGES as u64 || pages > u32::MAX as u64 {
        return Err(LoaderError::ResourceLimit);
    }
    Ok(pages as u32)
}

fn contains_address(
    segments: &[Option<LoadSegment>; MAX_LOAD_SEGMENTS],
    count: usize,
    address: u64,
    length: u64,
) -> bool {
    segments.iter().flatten().take(count).any(|segment| {
        address >= segment.virtual_address && address.saturating_add(length) <= segment.memory_end()
    })
}

fn virtual_to_file(
    bytes: &[u8],
    segments: &[Option<LoadSegment>; MAX_LOAD_SEGMENTS],
    count: usize,
    address: u64,
) -> Result<u64, LoaderError> {
    let segment = segments
        .iter()
        .flatten()
        .take(count)
        .find(|segment| {
            address >= segment.virtual_address
                && address < segment.virtual_address + segment.file_size
        })
        .ok_or(LoaderError::InvalidRelocation)?;
    let offset = segment
        .file_offset
        .checked_add(address - segment.virtual_address)
        .ok_or(LoaderError::SegmentOverflow)?;
    range(bytes, offset, 1)?;
    Ok(offset)
}

fn align_down(value: u64) -> u64 {
    value / PAGE_SIZE * PAGE_SIZE
}

fn align_up(value: u64) -> Result<u64, LoaderError> {
    value
        .checked_add(PAGE_SIZE - 1)
        .map(|aligned| aligned / PAGE_SIZE * PAGE_SIZE)
        .ok_or(LoaderError::SegmentOverflow)
}

fn range(bytes: &[u8], offset: u64, length: u64) -> Result<&[u8], LoaderError> {
    let end = offset.checked_add(length).ok_or(LoaderError::Truncated)?;
    if end > bytes.len() as u64 {
        return Err(LoaderError::Truncated);
    }
    Ok(&bytes[offset as usize..end as usize])
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, LoaderError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or(LoaderError::Truncated)?
        .try_into()
        .map_err(|_| LoaderError::Truncated)?;
    Ok(u16::from_le_bytes(value))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, LoaderError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(LoaderError::Truncated)?
        .try_into()
        .map_err(|_| LoaderError::Truncated)?;
    Ok(u32::from_le_bytes(value))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, LoaderError> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or(LoaderError::Truncated)?
        .try_into()
        .map_err(|_| LoaderError::Truncated)?;
    Ok(u64::from_le_bytes(value))
}

fn read_i64(bytes: &[u8], offset: usize) -> Result<i64, LoaderError> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or(LoaderError::Truncated)?
        .try_into()
        .map_err(|_| LoaderError::Truncated)?;
    Ok(i64::from_le_bytes(value))
}

#[cfg(test)]
mod tests {
    use super::{LoaderError, read_u32};

    #[test]
    fn truncated_loader_scalar_is_an_error() {
        assert_eq!(read_u32(&[0; 3], 0), Err(LoaderError::Truncated));
    }
}
