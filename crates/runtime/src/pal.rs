//! Capability-backed primitives used by the native `std::sys::ghostos` PAL.

use ghostos_ipc::SharedBuffer;
use ghostos_status::Status;

use crate::{
    Capability, Error, Operation, Request, Response, Runtime, SystemCall, ThreadStart, WaitWord,
};

pub const MAX_PAL_PATH_BYTES: usize = 4096;
pub const MAX_PAL_ARGUMENT_BYTES: usize = 64 * 1024;

pub const GHOSTOS_TOOLCHAIN_ROOT: &str = "/system/toolchains/stage-2";
pub const GHOSTOS_REGISTRY_ROOT: &str = "/system/registries";
pub const GHOSTOS_SOURCE_ROOT: &str = "/system/sources";
pub const GHOSTOS_BUILD_ROOT: &str = "/system/builds";
pub const GHOSTOS_TEMP_ROOT: &str = "/system/tmp";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MemoryProtection {
    None = 0,
    Read = 1,
    ReadWrite = 3,
    ReadExecute = 5,
}

impl MemoryProtection {
    pub const fn writable(self) -> bool {
        matches!(self, Self::ReadWrite)
    }

    pub const fn executable(self) -> bool {
        matches!(self, Self::ReadExecute)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CapabilityObjectKind {
    Unknown = 0,
    Memory = 1,
    AddressSpace = 2,
    Thread = 3,
    Ipc = 4,
    File = 5,
    Pipe = 6,
    Terminal = 7,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityDescription {
    pub kind: CapabilityObjectKind,
    pub rights: u16,
    pub object_id: u64,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessHandle {
    capability: Capability,
}

impl ProcessHandle {
    pub const fn from_capability(capability: Capability) -> Self {
        Self { capability }
    }

    pub const fn capability(self) -> Capability {
        self.capability
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Terminal {
    capability: Capability,
}

impl Terminal {
    pub const fn from_capability(capability: Capability) -> Self {
        Self { capability }
    }

    pub const fn capability(self) -> Capability {
        self.capability
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pipe {
    pub reader: Capability,
    pub writer: Capability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessExitStatus {
    pub code: i32,
    pub reason: ProcessExitReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProcessExitReason {
    Clean = 1,
    Cancelled = 2,
    Fault = 3,
    Killed = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessStatus {
    pub state: ProcessState,
    pub exit: Option<ProcessExitStatus>,
    pub memory_bytes: u64,
    pub cpu_time_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ProcessState {
    Unknown = 0,
    Running = 1,
    Exited = 2,
    Cancelled = 3,
    Faulted = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferPage {
    pub bytes: usize,
    pub next: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct TlsKey(u32);

impl TlsKey {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeMutex {
    pub word: WaitWord,
}

impl RuntimeMutex {
    pub const fn new(word: WaitWord) -> Self {
        Self { word }
    }

    /// The caller performs the atomic compare-and-exchange on the shared word,
    /// then parks here if it observed the locked value.
    pub fn wait<S: SystemCall>(
        self,
        runtime: &Runtime<S>,
        locked_value: u32,
        timeout_nanoseconds: Option<u64>,
    ) -> Result<bool, Error> {
        runtime.wait(self.word, locked_value, timeout_nanoseconds)
    }

    pub fn wake_one<S: SystemCall>(self, runtime: &Runtime<S>) -> Result<u32, Error> {
        runtime.wake(self.word, 1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeCondvar {
    pub word: WaitWord,
}

impl RuntimeCondvar {
    pub const fn new(word: WaitWord) -> Self {
        Self { word }
    }

    pub fn wait<S: SystemCall>(
        self,
        runtime: &Runtime<S>,
        sequence: u32,
        timeout_nanoseconds: Option<u64>,
    ) -> Result<bool, Error> {
        runtime.wait(self.word, sequence, timeout_nanoseconds)
    }

    pub fn notify<S: SystemCall>(self, runtime: &Runtime<S>, count: u32) -> Result<u32, Error> {
        runtime.wake(self.word, count)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DynamicLoadingPolicy {
    StaticOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanicModel {
    Abort,
}

pub const PANIC_MODEL: PanicModel = PanicModel::Abort;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BacktraceFrame {
    pub instruction_pointer: u64,
    pub stack_pointer: u64,
    pub module: u64,
}

pub trait BacktraceProvider {
    type Error;

    fn frame(&mut self, index: usize) -> Result<Option<BacktraceFrame>, Self::Error>;
}

impl<S: SystemCall> Runtime<S> {
    pub fn random_fill(&self, output: SharedBuffer) -> Result<usize, Error> {
        let response = self.execute(Request::new(Operation::RandomGet).with_buffer(output))?;
        bounded_length(response, output)
    }

    pub fn terminal_read(&self, terminal: Terminal, output: SharedBuffer) -> Result<usize, Error> {
        let response = self.execute(
            Request::new(Operation::TerminalRead)
                .with_capability(terminal.capability)
                .with_buffer(output),
        )?;
        bounded_length(response, output)
    }

    pub fn terminal_write(&self, terminal: Terminal, input: SharedBuffer) -> Result<usize, Error> {
        let response = self.execute(
            Request::new(Operation::TerminalWrite)
                .with_capability(terminal.capability)
                .with_buffer(input),
        )?;
        bounded_length(response, input)
    }

    pub fn spawn_process(&self, start: ThreadStart) -> Result<ProcessHandle, Error> {
        let mut request = Request::new(Operation::ProcessSpawn).with_capability(start.image);
        request.arguments = [
            start.entry_offset,
            start.stack.raw(),
            start.stack_top_offset,
            start.argument,
            0,
            0,
        ];
        let response = self.execute(request)?;
        let capability = Capability::from_raw(response.values[0]).ok_or(Error::InvalidResponse)?;
        Ok(ProcessHandle { capability })
    }

    pub fn exec_process(&self, process: ProcessHandle, start: ThreadStart) -> Result<(), Error> {
        let mut request = Request::new(Operation::ProcessExec).with_capability(process.capability);
        request.arguments = [
            start.image.raw(),
            start.entry_offset,
            start.stack.raw(),
            start.stack_top_offset,
            start.argument,
            0,
        ];
        self.execute(request)?;
        Ok(())
    }

    pub fn wait_process(
        &self,
        process: ProcessHandle,
        timeout_nanoseconds: Option<u64>,
    ) -> Result<Option<ProcessExitStatus>, Error> {
        let mut request = Request::new(Operation::ProcessWait).with_capability(process.capability);
        request.arguments[0] = timeout_nanoseconds.unwrap_or(u64::MAX);
        let (response, status) = self.execute_raw(request)?;
        if status == Status::PENDING {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(Error::Status(status));
        }
        Ok(Some(decode_exit(response)?))
    }

    pub fn cancel_process(&self, process: ProcessHandle) -> Result<(), Error> {
        self.execute(Request::new(Operation::ProcessCancel).with_capability(process.capability))?;
        Ok(())
    }

    pub fn exit_process(&self, code: i32) -> Result<(), Error> {
        let mut request = Request::new(Operation::ProcessExit);
        request.arguments[0] = code as i64 as u64;
        self.execute(request)?;
        Ok(())
    }

    pub fn process_status(&self, process: ProcessHandle) -> Result<ProcessStatus, Error> {
        let response = self
            .execute(Request::new(Operation::ProcessStatus).with_capability(process.capability))?;
        let state = match response.values[0] {
            1 => ProcessState::Running,
            2 => ProcessState::Exited,
            3 => ProcessState::Cancelled,
            4 => ProcessState::Faulted,
            0 => ProcessState::Unknown,
            _ => return Err(Error::InvalidResponse),
        };
        let exit = if response.values[1] == 0 {
            None
        } else {
            Some(decode_exit(response)?)
        };
        Ok(ProcessStatus {
            state,
            exit,
            memory_bytes: response.values[2],
            cpu_time_us: response.values[3],
        })
    }

    pub fn protect_memory(
        &self,
        memory: Capability,
        address: u64,
        length: u64,
        protection: MemoryProtection,
    ) -> Result<(), Error> {
        let mut request = Request::new(Operation::MemoryProtect).with_capability(memory);
        request.arguments = [address, length, protection as u64, 0, 0, 0];
        self.execute(request)?;
        Ok(())
    }

    pub fn describe_capability(
        &self,
        capability: Capability,
    ) -> Result<CapabilityDescription, Error> {
        let response =
            self.execute(Request::new(Operation::CapabilityQuery).with_capability(capability))?;
        let kind = match response.values[0] as u8 {
            1 => CapabilityObjectKind::Memory,
            2 => CapabilityObjectKind::AddressSpace,
            3 => CapabilityObjectKind::Thread,
            4 => CapabilityObjectKind::Ipc,
            5 => CapabilityObjectKind::File,
            6 => CapabilityObjectKind::Pipe,
            7 => CapabilityObjectKind::Terminal,
            0 => CapabilityObjectKind::Unknown,
            _ => return Err(Error::InvalidResponse),
        };
        Ok(CapabilityDescription {
            kind,
            rights: u16::try_from(response.values[1]).map_err(|_| Error::InvalidResponse)?,
            object_id: response.values[2],
            flags: u32::try_from(response.values[3]).map_err(|_| Error::InvalidResponse)?,
        })
    }

    pub fn create_pipe(&self) -> Result<Pipe, Error> {
        let response = self.execute(Request::new(Operation::PipeCreate))?;
        Ok(Pipe {
            reader: Capability::from_raw(response.values[0]).ok_or(Error::InvalidResponse)?,
            writer: Capability::from_raw(response.values[1]).ok_or(Error::InvalidResponse)?,
        })
    }

    pub fn read_pipe(&self, reader: Capability, output: SharedBuffer) -> Result<usize, Error> {
        let response = self.execute(
            Request::new(Operation::PipeRead)
                .with_capability(reader)
                .with_buffer(output),
        )?;
        bounded_length(response, output)
    }

    pub fn write_pipe(&self, writer: Capability, input: SharedBuffer) -> Result<usize, Error> {
        let response = self.execute(
            Request::new(Operation::PipeWrite)
                .with_capability(writer)
                .with_buffer(input),
        )?;
        bounded_length(response, input)
    }

    pub fn close_pipe(&self, pipe: Capability) -> Result<(), Error> {
        self.execute(Request::new(Operation::PipeClose).with_capability(pipe))?;
        Ok(())
    }

    pub fn tls_get(&self, key: TlsKey) -> Result<u64, Error> {
        let mut request = Request::new(Operation::ThreadTlsGet);
        request.arguments[0] = key.raw() as u64;
        Ok(self.execute(request)?.values[0])
    }

    pub fn tls_set(&self, key: TlsKey, value: u64) -> Result<(), Error> {
        let mut request = Request::new(Operation::ThreadTlsSet);
        request.arguments = [key.raw() as u64, value, 0, 0, 0, 0];
        self.execute(request)?;
        Ok(())
    }

    pub fn read_arguments(
        &self,
        output: SharedBuffer,
        continuation: u64,
    ) -> Result<BufferPage, Error> {
        read_buffer_page(self, Operation::ArgumentsRead, output, continuation)
    }

    pub fn read_environment(
        &self,
        output: SharedBuffer,
        continuation: u64,
    ) -> Result<BufferPage, Error> {
        read_buffer_page(self, Operation::EnvironmentRead, output, continuation)
    }
}

fn read_buffer_page<S: SystemCall>(
    runtime: &Runtime<S>,
    operation: Operation,
    output: SharedBuffer,
    continuation: u64,
) -> Result<BufferPage, Error> {
    let mut request = Request::new(operation).with_buffer(output);
    request.arguments[4] = continuation;
    let response = runtime.execute(request)?;
    let bytes = usize::try_from(response.values[0]).map_err(|_| Error::InvalidResponse)?;
    if bytes > output.length as usize || response.values[1] == u64::MAX {
        return Err(Error::InvalidResponse);
    }
    Ok(BufferPage {
        bytes,
        next: response.values[1],
    })
}

fn bounded_length(response: Response, buffer: SharedBuffer) -> Result<usize, Error> {
    let length = usize::try_from(response.values[0]).map_err(|_| Error::InvalidResponse)?;
    if length > buffer.length as usize {
        Err(Error::InvalidResponse)
    } else {
        Ok(length)
    }
}

fn decode_exit(response: Response) -> Result<ProcessExitStatus, Error> {
    let reason = match response.values[1] {
        1 => ProcessExitReason::Clean,
        2 => ProcessExitReason::Cancelled,
        3 => ProcessExitReason::Fault,
        4 => ProcessExitReason::Killed,
        _ => return Err(Error::InvalidResponse),
    };
    Ok(ProcessExitStatus {
        code: response.values[0] as i32,
        reason,
    })
}
