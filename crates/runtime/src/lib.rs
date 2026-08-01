#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

mod abi;
mod fs;
mod ipc;
pub mod sys;
mod thread;

use synos_status::Status;

pub use abi::{Capability, GateFn, NativeGate, Operation, Request, Response, SystemCall};
pub use fs::{DirectoryPage, File, Metadata, OpenOptions, DIRECTORY_RECORD_HEADER_BYTES};
pub use ipc::{IpcAccess, IpcMapping};
pub use thread::{Thread, ThreadStart, WaitWord};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidResponse,
    Status(Status),
}

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
        let response = self.execute(Request::new(Operation::ClockNow))?;
        Ok(response.values[0])
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
        let response = self.system.call(request);
        let status = Status::from_raw(response.status).ok_or(Error::InvalidResponse)?;
        if status.is_success() {
            Ok(response)
        } else {
            Err(Error::Status(status))
        }
    }
}
