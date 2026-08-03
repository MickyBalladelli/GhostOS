use super::{OpenFlags, PosixCompat, SeekFrom, SharedBuffers, SystemCall};

pub const MAX_LINUX_PATH_BYTES: usize = 4096;
pub const AT_FDCWD: i32 = -100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum LinuxArchitecture {
    X86_64,
    Aarch64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinuxSyscall {
    Read,
    Write,
    Open,
    OpenAt,
    Close,
    Lseek,
    ClockGettime,
    GetPid,
    Exit,
    Unknown(u64),
}

impl LinuxSyscall {
    pub const fn from_number(architecture: LinuxArchitecture, number: u64) -> Self {
        match architecture {
            LinuxArchitecture::X86_64 => match number {
                0 => Self::Read,
                1 => Self::Write,
                2 => Self::Open,
                3 => Self::Close,
                8 => Self::Lseek,
                39 => Self::GetPid,
                60 => Self::Exit,
                228 => Self::ClockGettime,
                257 => Self::OpenAt,
                _ => Self::Unknown(number),
            },
            LinuxArchitecture::Aarch64 => match number {
                56 => Self::OpenAt,
                57 => Self::Close,
                62 => Self::Lseek,
                63 => Self::Read,
                64 => Self::Write,
                93 => Self::Exit,
                113 => Self::ClockGettime,
                172 => Self::GetPid,
                _ => Self::Unknown(number),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct LinuxSyscallRequest {
    pub architecture: LinuxArchitecture,
    pub number: u64,
    pub arguments: [u64; 6],
    pub process_id: u64,
}

impl LinuxSyscallRequest {
    pub const fn new(
        architecture: LinuxArchitecture,
        number: u64,
        arguments: [u64; 6],
        process_id: u64,
    ) -> Self {
        Self {
            architecture,
            number,
            arguments,
            process_id,
        }
    }

    pub const fn syscall(self) -> LinuxSyscall {
        LinuxSyscall::from_number(self.architecture, self.number)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i64)]
pub enum LinuxErrno {
    PermissionDenied = 1,
    NoSuchFile = 2,
    Interrupted = 4,
    Io = 5,
    BadFileDescriptor = 9,
    TryAgain = 11,
    OutOfMemory = 12,
    BadAddress = 14,
    Busy = 16,
    InvalidArgument = 22,
    TooManyOpenFiles = 24,
    NoSpace = 28,
    FunctionNotImplemented = 38,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct LinuxSyscallResponse {
    pub value: i64,
    pub exited: bool,
}

impl LinuxSyscallResponse {
    pub const fn success(value: i64) -> Self {
        Self {
            value,
            exited: false,
        }
    }

    pub const fn error(errno: LinuxErrno) -> Self {
        Self {
            value: -(errno as i64),
            exited: false,
        }
    }

    pub const fn exit() -> Self {
        Self {
            value: 0,
            exited: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserMemoryError {
    Unmapped,
    ReadOnly,
    TooLarge,
    MissingTerminator,
}

/// Capability-checked view of the trapped process address space.
///
/// Native loaders can return shared pages directly from `read` and `writable`,
/// so syscall translation does not require a bounce buffer. A loader that owns
/// ordinary C memory may provide a bounded view over its private pages.
pub trait LinuxUserMemory {
    fn read(&self, address: u64, length: usize) -> Result<&[u8], UserMemoryError>;
    fn writable(&mut self, address: u64, length: usize) -> Result<&mut [u8], UserMemoryError>;

    fn c_string(&self, address: u64, maximum: usize) -> Result<&[u8], UserMemoryError> {
        let bytes = self.read(address, maximum)?;
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(UserMemoryError::MissingTerminator)?;
        Ok(&bytes[..end])
    }
}

pub(crate) fn dispatch<S, M, U, const MAX_FILES: usize>(
    compat: &mut PosixCompat<S, M, MAX_FILES>,
    request: LinuxSyscallRequest,
    memory: &mut U,
) -> LinuxSyscallResponse
where
    S: SystemCall,
    M: SharedBuffers,
    U: LinuxUserMemory,
{
    let syscall = request.syscall();
    let args = request.arguments;
    match syscall {
        LinuxSyscall::Read => {
            let fd = match i32::try_from(args[0]) {
                Ok(fd) => fd,
                Err(_) => return LinuxSyscallResponse::error(LinuxErrno::BadFileDescriptor),
            };
            let length = match usize::try_from(args[2]) {
                Ok(length) => length,
                Err(_) => return LinuxSyscallResponse::error(LinuxErrno::BadAddress),
            };
            let output = match memory.writable(args[1], length) {
                Ok(output) => output,
                Err(_) => return LinuxSyscallResponse::error(LinuxErrno::BadAddress),
            };
            match compat.read(fd, output) {
                Ok(bytes) => LinuxSyscallResponse::success(bytes as i64),
                Err(error) => LinuxSyscallResponse::error(error.linux_errno()),
            }
        }
        LinuxSyscall::Write => {
            let fd = match i32::try_from(args[0]) {
                Ok(fd) => fd,
                Err(_) => return LinuxSyscallResponse::error(LinuxErrno::BadFileDescriptor),
            };
            let length = match usize::try_from(args[2]) {
                Ok(length) => length,
                Err(_) => return LinuxSyscallResponse::error(LinuxErrno::BadAddress),
            };
            let input = match memory.read(args[1], length) {
                Ok(input) => input,
                Err(_) => return LinuxSyscallResponse::error(LinuxErrno::BadAddress),
            };
            match compat.write(fd, input) {
                Ok(bytes) => LinuxSyscallResponse::success(bytes as i64),
                Err(error) => LinuxSyscallResponse::error(error.linux_errno()),
            }
        }
        LinuxSyscall::Open | LinuxSyscall::OpenAt => {
            if matches!(syscall, LinuxSyscall::OpenAt)
                && i32::try_from(args[0]).ok() != Some(AT_FDCWD)
            {
                return LinuxSyscallResponse::error(LinuxErrno::InvalidArgument);
            }
            let path_address = if matches!(syscall, LinuxSyscall::Open) {
                args[0]
            } else {
                args[1]
            };
            let path = match memory.c_string(path_address, MAX_LINUX_PATH_BYTES) {
                Ok(path) => path,
                Err(_) => return LinuxSyscallResponse::error(LinuxErrno::BadAddress),
            };
            let flags = if matches!(syscall, LinuxSyscall::Open) {
                args[1]
            } else {
                args[2]
            };
            let flags = match linux_open_flags(flags) {
                Ok(flags) => flags,
                Err(errno) => return LinuxSyscallResponse::error(errno),
            };
            match compat.open(path, flags) {
                Ok(fd) => LinuxSyscallResponse::success(fd as i64),
                Err(error) => LinuxSyscallResponse::error(error.linux_errno()),
            }
        }
        LinuxSyscall::Close => {
            let fd = match i32::try_from(args[0]) {
                Ok(fd) => fd,
                Err(_) => return LinuxSyscallResponse::error(LinuxErrno::BadFileDescriptor),
            };
            match compat.close(fd) {
                Ok(()) => LinuxSyscallResponse::success(0),
                Err(error) => LinuxSyscallResponse::error(error.linux_errno()),
            }
        }
        LinuxSyscall::Lseek => {
            let fd = match i32::try_from(args[0]) {
                Ok(fd) => fd,
                Err(_) => return LinuxSyscallResponse::error(LinuxErrno::BadFileDescriptor),
            };
            let offset = args[1] as i64;
            let seek = match args[2] {
                0 => u64::try_from(offset).map(SeekFrom::Start),
                1 => Ok(SeekFrom::Current(offset)),
                2 => Ok(SeekFrom::End(offset)),
                _ => return LinuxSyscallResponse::error(LinuxErrno::InvalidArgument),
            };
            let seek = match seek {
                Ok(seek) => seek,
                Err(_) => return LinuxSyscallResponse::error(LinuxErrno::InvalidArgument),
            };
            match compat.seek(fd, seek) {
                Ok(position) => match i64::try_from(position) {
                    Ok(position) => LinuxSyscallResponse::success(position),
                    Err(_) => LinuxSyscallResponse::error(LinuxErrno::InvalidArgument),
                },
                Err(error) => LinuxSyscallResponse::error(error.linux_errno()),
            }
        }
        LinuxSyscall::ClockGettime => {
            if args[0] != 1 {
                return LinuxSyscallResponse::error(LinuxErrno::InvalidArgument);
            }
            let nanoseconds = match compat.monotonic_time_nanoseconds() {
                Ok(nanoseconds) => nanoseconds,
                Err(error) => return LinuxSyscallResponse::error(error.linux_errno()),
            };
            let seconds = nanoseconds / 1_000_000_000;
            let remainder = nanoseconds % 1_000_000_000;
            let output = match memory.writable(args[1], 16) {
                Ok(output) => output,
                Err(_) => return LinuxSyscallResponse::error(LinuxErrno::BadAddress),
            };
            output[..8].copy_from_slice(&(seconds as i64).to_ne_bytes());
            output[8..].copy_from_slice(&(remainder as i64).to_ne_bytes());
            LinuxSyscallResponse::success(0)
        }
        LinuxSyscall::GetPid => {
            if request.process_id == 0 {
                LinuxSyscallResponse::error(LinuxErrno::InvalidArgument)
            } else {
                match i64::try_from(request.process_id) {
                    Ok(process_id) => LinuxSyscallResponse::success(process_id),
                    Err(_) => LinuxSyscallResponse::error(LinuxErrno::InvalidArgument),
                }
            }
        }
        LinuxSyscall::Exit => LinuxSyscallResponse::exit(),
        LinuxSyscall::Unknown(_) => LinuxSyscallResponse::error(LinuxErrno::FunctionNotImplemented),
    }
}

fn linux_open_flags(bits: u64) -> Result<OpenFlags, LinuxErrno> {
    const O_WRONLY: u64 = 1;
    const O_RDWR: u64 = 2;
    const O_CREAT: u64 = 64;
    const O_TRUNC: u64 = 512;
    const O_APPEND: u64 = 1024;
    const IGNORED_STATUS_FLAGS: u64 = (1 << 11)
        | (1 << 16)
        | (1 << 17)
        | (1 << 19)
        | (1 << 20)
        | (1 << 21)
        | (1 << 22);
    let known = O_WRONLY | O_RDWR | O_CREAT | O_TRUNC | O_APPEND | IGNORED_STATUS_FLAGS;
    if bits & !known != 0 || bits & O_WRONLY != 0 && bits & O_RDWR != 0 {
        return Err(LinuxErrno::InvalidArgument);
    }
    let mut translated = 0;
    if bits & O_WRONLY != 0 {
        translated |= OpenFlags::WRITE_ONLY.bits();
    } else if bits & O_RDWR != 0 {
        translated |= OpenFlags::READ_WRITE.bits();
    }
    if bits & O_CREAT != 0 {
        translated |= OpenFlags::CREATE.bits();
    }
    if bits & O_TRUNC != 0 {
        translated |= OpenFlags::TRUNCATE.bits();
    }
    if bits & O_APPEND != 0 {
        translated |= OpenFlags::APPEND.bits();
    }
    OpenFlags::from_bits(translated).ok_or(LinuxErrno::InvalidArgument)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_common_linux_vectors() {
        assert_eq!(
            LinuxSyscall::from_number(LinuxArchitecture::X86_64, 257),
            LinuxSyscall::OpenAt
        );
        assert_eq!(
            LinuxSyscall::from_number(LinuxArchitecture::Aarch64, 63),
            LinuxSyscall::Read
        );
        assert_eq!(
            LinuxSyscall::from_number(LinuxArchitecture::X86_64, 999),
            LinuxSyscall::Unknown(999)
        );
    }

    #[test]
    fn translates_linux_open_flags() {
        let flags = linux_open_flags(64 | 1024).expect("valid Linux flags");
        assert_eq!(
            flags,
            OpenFlags::CREATE.union(OpenFlags::APPEND)
        );
        assert_eq!(linux_open_flags(1 | 2), Err(LinuxErrno::InvalidArgument));
    }
}
