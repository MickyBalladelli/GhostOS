#![no_std]
#![forbid(unsafe_code)]

mod fd;
mod container;
mod pseudo;
mod syscall;

use fd::FdTable;
use synos_ipc::SharedBuffer;
use synos_runtime::{OpenOptions, Runtime, SystemCall};

pub use container::{
    ContainerMemoryPolicy, ZeroCopyContainerMemory, ZeroCopyMemoryError,
};
pub use pseudo::{
    LogicalNameResolver, MAX_PSEUDO_PATH_BYTES, PseudoFileSystem, PseudoFsError, PseudoPath,
    PseudoPathError, PseudoResource, PseudoResourceKind,
};
pub use syscall::{
    LinuxArchitecture, LinuxErrno, LinuxSyscall, LinuxSyscallRequest, LinuxSyscallResponse,
    LinuxUserMemory, UserMemoryError, AT_FDCWD, MAX_LINUX_PATH_BYTES,
};

pub const DEFAULT_MAX_FILES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferError {
    Empty,
    NotMapped,
    PermissionDenied,
    TooLarge,
}

/// Maps a legacy process slice into an existing capability-owned shared region.
///
/// Implementations may use a fixed bounce window for C libraries that cannot
/// supply shared memory. Native SynOS callers should return their already
/// mapped descriptor so reads and writes stay zero-copy.
pub trait SharedBuffers {
    fn map_input(&mut self, input: &[u8]) -> Result<SharedBuffer, BufferError>;
    fn map_output(&mut self, output: &mut [u8]) -> Result<SharedBuffer, BufferError>;
    fn release(&mut self, buffer: SharedBuffer);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    BadFileDescriptor,
    Buffer(BufferError),
    InvalidArgument,
    Runtime(synos_runtime::Error),
    TooManyFiles,
}

impl Error {
    pub fn linux_errno(self) -> LinuxErrno {
        match self {
            Self::BadFileDescriptor => LinuxErrno::BadFileDescriptor,
            Self::Buffer(BufferError::Empty | BufferError::NotMapped) => LinuxErrno::BadAddress,
            Self::Buffer(BufferError::PermissionDenied) => LinuxErrno::BadAddress,
            Self::Buffer(BufferError::TooLarge) => LinuxErrno::InvalidArgument,
            Self::InvalidArgument => LinuxErrno::InvalidArgument,
            Self::TooManyFiles => LinuxErrno::TooManyOpenFiles,
            Self::Runtime(error) => match error {
                synos_runtime::Error::InvalidResponse => LinuxErrno::Io,
                synos_runtime::Error::Status(status) => {
                    if status == synos_status::Status::NOT_FOUND {
                        LinuxErrno::NoSuchFile
                    } else if status == synos_status::Status::ACCESS_DENIED {
                        LinuxErrno::PermissionDenied
                    } else if status == synos_status::Status::NO_SPACE {
                        LinuxErrno::NoSpace
                    } else if status == synos_status::Status::BUSY {
                        LinuxErrno::Busy
                    } else {
                        LinuxErrno::Io
                    }
                }
            },
        }
    }
}

impl From<BufferError> for Error {
    fn from(error: BufferError) -> Self {
        Self::Buffer(error)
    }
}

impl From<synos_runtime::Error> for Error {
    fn from(error: synos_runtime::Error) -> Self {
        Self::Runtime(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenFlags(u16);

impl OpenFlags {
    pub const READ_ONLY: Self = Self(0);
    pub const WRITE_ONLY: Self = Self(1 << 0);
    pub const READ_WRITE: Self = Self(1 << 1);
    pub const CREATE: Self = Self(1 << 2);
    pub const TRUNCATE: Self = Self(1 << 3);
    pub const APPEND: Self = Self(1 << 4);
    const ALL: u16 = (1 << 5) - 1;

    pub const fn from_bits(bits: u16) -> Option<Self> {
        if bits & !Self::ALL != 0
            || bits & Self::WRITE_ONLY.0 != 0 && bits & Self::READ_WRITE.0 != 0
        {
            None
        } else {
            Some(Self(bits))
        }
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn options(self) -> OpenOptions {
        let mut options = if self.0 & Self::READ_WRITE.0 != 0 {
            OpenOptions::READ.union(OpenOptions::WRITE)
        } else if self.0 & Self::WRITE_ONLY.0 != 0 {
            OpenOptions::WRITE
        } else {
            OpenOptions::READ
        };
        if self.0 & Self::CREATE.0 != 0 {
            options = options.union(OpenOptions::CREATE)
        }
        if self.0 & Self::TRUNCATE.0 != 0 {
            options = options.union(OpenOptions::TRUNCATE)
        }
        if self.0 & Self::APPEND.0 != 0 {
            options = options.union(OpenOptions::APPEND)
        }
        options
    }

    const fn append(self) -> bool {
        self.0 & Self::APPEND.0 != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeekFrom {
    Start(u64),
    Current(i64),
    End(i64),
}

/// Fixed-capacity translation layer for legacy C/POSIX-style file calls.
///
/// File descriptors are process-local indexes. Every backing object remains a
/// generation-checked SynOS capability and every buffer crosses the boundary
/// through a shared-region descriptor.
pub struct PosixCompat<S, M, const MAX_FILES: usize = DEFAULT_MAX_FILES> {
    runtime: Runtime<S>,
    buffers: M,
    files: FdTable<MAX_FILES>,
}

impl<S: SystemCall, M: SharedBuffers, const MAX_FILES: usize> PosixCompat<S, M, MAX_FILES> {
    pub const fn new(system: S, buffers: M) -> Self {
        Self {
            runtime: Runtime::new(system),
            buffers,
            files: FdTable::new(),
        }
    }

    pub fn open(&mut self, path: &[u8], flags: OpenFlags) -> Result<i32, Error> {
        if path.is_empty() || path.contains(&0) || core::str::from_utf8(path).is_err() {
            return Err(Error::InvalidArgument);
        }
        let descriptor = self.buffers.map_input(path)?;
        let result = self.runtime.open(descriptor, flags.options());
        self.buffers.release(descriptor);
        let file = result?;
        match self.files.insert(file, flags.append()) {
            Ok(fd) => Ok(fd),
            Err(error) => {
                let _ = self.runtime.close(file);
                Err(error)
            }
        }
    }

    pub fn close(&mut self, fd: i32) -> Result<(), Error> {
        let entry = self.files.get(fd)?;
        self.runtime.close(entry.file)?;
        self.files.remove(fd)?;
        Ok(())
    }

    pub fn read(&mut self, fd: i32, output: &mut [u8]) -> Result<usize, Error> {
        if output.is_empty() {
            return Ok(0);
        }
        let entry = self.files.get(fd)?;
        let output_length =
            u32::try_from(output.len()).map_err(|_| Error::Buffer(BufferError::TooLarge))?;
        let descriptor = self.buffers.map_output(output)?;
        if !descriptor.writable || descriptor.length < output_length {
            self.buffers.release(descriptor);
            return Err(Error::Buffer(BufferError::PermissionDenied));
        }
        let result = self.runtime.read_at(entry.file, entry.offset, descriptor);
        self.buffers.release(descriptor);
        let read = result?;
        if read > output.len() {
            return Err(Error::Runtime(synos_runtime::Error::InvalidResponse));
        }
        let offset = entry
            .offset
            .checked_add(read as u64)
            .ok_or(Error::InvalidArgument)?;
        self.files.update_offset(fd, offset)?;
        Ok(read)
    }

    pub fn write(&mut self, fd: i32, input: &[u8]) -> Result<usize, Error> {
        if input.is_empty() {
            return Ok(0);
        }
        let entry = self.files.get(fd)?;
        let input_length =
            u32::try_from(input.len()).map_err(|_| Error::Buffer(BufferError::TooLarge))?;
        let descriptor = self.buffers.map_input(input)?;
        if descriptor.length < input_length {
            self.buffers.release(descriptor);
            return Err(Error::Buffer(BufferError::TooLarge));
        }
        let write_offset = if entry.append {
            match self.runtime.metadata(entry.file) {
                Ok(metadata) => metadata.length,
                Err(error) => {
                    self.buffers.release(descriptor);
                    return Err(error.into());
                }
            }
        } else {
            entry.offset
        };
        let result = self.runtime.write_at(entry.file, write_offset, descriptor);
        self.buffers.release(descriptor);
        let written = result?;
        if written > input.len() {
            return Err(Error::Runtime(synos_runtime::Error::InvalidResponse));
        }
        let offset = write_offset
            .checked_add(written as u64)
            .ok_or(Error::InvalidArgument)?;
        self.files.update_offset(fd, offset)?;
        Ok(written)
    }

    pub fn seek(&mut self, fd: i32, from: SeekFrom) -> Result<u64, Error> {
        let entry = self.files.get(fd)?;
        let offset = match from {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(delta) => {
                if delta < 0 {
                    entry
                        .offset
                        .checked_sub(delta.unsigned_abs())
                        .ok_or(Error::InvalidArgument)?
                } else {
                    entry
                        .offset
                        .checked_add(delta as u64)
                        .ok_or(Error::InvalidArgument)?
                }
            }
            SeekFrom::End(delta) => {
                let end = self.runtime.metadata(entry.file)?.length;
                if delta < 0 {
                    end.checked_sub(delta.unsigned_abs())
                        .ok_or(Error::InvalidArgument)?
                } else {
                    end.checked_add(delta as u64)
                        .ok_or(Error::InvalidArgument)?
                }
            }
        };
        self.files.update_offset(fd, offset)?;
        Ok(offset)
    }

    pub fn monotonic_time_nanoseconds(&self) -> Result<u64, Error> {
        self.runtime.clock_now().map_err(Into::into)
    }

    pub const fn runtime(&self) -> &Runtime<S> {
        &self.runtime
    }

    pub const fn buffers(&self) -> &M {
        &self.buffers
    }

    /// Dispatches the Linux syscall numbers supported by the compatibility
    /// vector. The trap handler passes register values and a capability-checked
    /// user-memory view; no global POSIX state is consulted.
    pub fn dispatch_linux_syscall<U: LinuxUserMemory>(
        &mut self,
        request: LinuxSyscallRequest,
        memory: &mut U,
    ) -> LinuxSyscallResponse {
        syscall::dispatch(self, request, memory)
    }
}
