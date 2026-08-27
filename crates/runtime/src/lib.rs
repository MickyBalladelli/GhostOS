#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

mod abi;
mod fs;
mod ipc;
mod pal;
pub mod sys;
mod thread;

use ghostos_status::Status;

pub use abi::{
    ABI_REVISION, ABI_SCHEMA_VERSION, Capability, GateFn, NativeGate, Operation, Request, Response,
    SystemCall,
};
pub use ghostos_path_pattern::{Pattern, PatternError};
pub use fs::{
    decode_boot_fs_buffer, decode_runtime_fs_buffer, seed_directory_path, BootFsBuffer,
    DirectoryPage, DirectoryRemovalMetadata, File, FileMapping, LinkMetadata, Metadata, OpenOptions,
    PathBuffer, DIRECTORY_RECORD_HEADER_BYTES, LIST_CONTINUATION_ARGUMENT, LIST_PATH_LENGTH_ARGUMENT,
};
pub use ipc::{IpcAccess, IpcMapping};
pub use pal::{
    BacktraceFrame, BacktraceProvider, BufferPage, CapabilityDescription, CapabilityObjectKind,
    DynamicLoadingPolicy, MemoryProtection, PanicModel, Pipe, ProcessExitReason,
    ProcessExitStatus, ProcessHandle, ProcessState, ProcessStatus, RuntimeCondvar, RuntimeMutex,
    Terminal, TlsKey, PANIC_MODEL, MAX_PAL_ARGUMENT_BYTES, MAX_PAL_PATH_BYTES,
    GHOSTOS_BUILD_ROOT, GHOSTOS_REGISTRY_ROOT, GHOSTOS_SOURCE_ROOT, GHOSTOS_TEMP_ROOT,
    GHOSTOS_TOOLCHAIN_ROOT,
};
pub use thread::{Thread, ThreadStart, WaitWord};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidResponse,
    Status(Status),
}

#[cfg(test)]
mod tests;

pub struct Runtime<S> {
    system: S,
}

impl<S: SystemCall> Runtime<S> {
    pub const fn new(system: S) -> Self {
        Self { system }
    }

    pub const fn system(&self) -> &S {
        &self.system
    }

    pub fn clock_now(&self) -> Result<u64, Error> {
        self.monotonic_now()
    }

    pub fn monotonic_now(&self) -> Result<u64, Error> {
        let response = self.execute(Request::new(Operation::ClockNow))?;
        Ok(response.values[0])
    }

    pub fn realtime_now(&self) -> Result<u64, Error> {
        let response = self.execute(Request::new(Operation::RealtimeNow))?;
        Ok(response.values[0])
    }

    pub fn sleep_until(&self, deadline_us: u64) -> Result<(), Error> {
        let mut request = Request::new(Operation::SleepUntil);
        request.arguments[0] = deadline_us;
        self.execute(request)?;
        Ok(())
    }

    pub fn sleep_for(&self, duration_us: u64) -> Result<(), Error> {
        let deadline = self.clock_now()?.saturating_add(duration_us);
        self.sleep_until(deadline)
    }

    pub fn map(
        &self,
        capability: Capability,
        offset: u64,
        length: u64,
        writable: bool,
    ) -> Result<u64, Error> {
        let mut request = Request::new(Operation::MemoryMap).with_capability(capability);
        request.arguments[0] = offset;
        request.arguments[1] = length;
        request.arguments[2] = writable as u64;
        let response = self.execute(request)?;
        Ok(response.values[0])
    }

    pub fn unmap(&self, capability: Capability, address: u64, length: u64) -> Result<(), Error> {
        let mut request = Request::new(Operation::MemoryUnmap).with_capability(capability);
        request.arguments[0] = address;
        request.arguments[1] = length;
        self.execute(request)?;
        Ok(())
    }

    pub(crate) fn execute(&self, request: Request) -> Result<Response, Error> {
        let (response, status) = self.execute_raw(request)?;
        if status.is_success() {
            Ok(response)
        } else {
            Err(Error::Status(status))
        }
    }

    pub(crate) fn execute_raw(&self, request: Request) -> Result<(Response, Status), Error> {
        let response = self.system.call(request);
        let status = Status::from_raw(response.status).ok_or(Error::InvalidResponse)?;
        Ok((response, status))
    }
}
