//! Kernel-side dispatch for the native GhostOS runtime ABI.
//!
//! The kernel checks the fixed ABI request and shared-buffer descriptor, then
//! hands filesystem work to the filesystem IPC transport. The transport is
//! responsible for sending the request over the capability-protected channel
//! and resolving the descriptor in the daemon's address space.

use crate::task::AddressSpaceId;
use crate::ipc::{Channel, Message};
use crate::{CapabilityHandle, CapabilitySpace};
use ghostos_fsd::{Capability as FsdCapability, Flags as FsdFlags, Operation as FsdOperation};
use ghostos_ipc::{ChannelId, Envelope, SharedBuffer, SharedRegionId};
use ghostos_runtime::{Operation, Request, Response};
use ghostos_status::Status;
use core::ffi::c_uint;

pub const MAX_FILESYSTEM_PROCESSES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilesystemIdentity {
    pub process: ghostos_fsd::ProcessId,
    pub authority: FsdCapability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeDispatchError {
    AbiMismatch,
    InvalidRequest,
    InvalidCapability,
    InvalidBuffer,
    ProcessNotRegistered,
    Capacity,
    TransportFailure,
}

impl RuntimeDispatchError {
    const fn status(self) -> Status {
        match self {
            Self::AbiMismatch => Status::PROTOCOL_MISMATCH,
            Self::InvalidRequest | Self::InvalidBuffer => Status::INVALID_ARGUMENT,
            Self::InvalidCapability | Self::ProcessNotRegistered => Status::ACCESS_DENIED,
            Self::Capacity => Status::NO_SPACE,
            Self::TransportFailure => Status::BUSY,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CProcessSlot {
    caller: u32,
    used: u32,
    process: u64,
    authority: u64,
}

#[repr(C)]
struct CRuntimeState {
    clock: u64,
    processes: [CProcessSlot; MAX_FILESYSTEM_PROCESSES],
}

const _: [(); 1544] = [(); core::mem::size_of::<CRuntimeState>()];

#[repr(C)]
#[derive(Clone, Copy)]
struct CRuntimeIdentity {
    process: u64,
    authority: u64,
}

#[repr(C)]
struct CRuntimeFilesystemRequest {
    operation: u32,
    flags: u32,
    process: u64,
    capability: u64,
    offset: u64,
    length: u64,
    region: u32,
    buffer_offset: u32,
    buffer_length: u32,
    writable: u32,
    has_buffer: u32,
    reserved: u32,
}

const _: [(); 64] = [(); core::mem::size_of::<CRuntimeFilesystemRequest>()];

unsafe extern "C" {
    fn ghostos_runtime_clock(state: *const CRuntimeState) -> u64;
    fn ghostos_runtime_advance_clock(state: *mut CRuntimeState, elapsed: u64);
    fn ghostos_runtime_register(
        state: *mut CRuntimeState,
        caller: u32,
        identity: CRuntimeIdentity,
        capacity: usize,
    ) -> c_uint;
    fn ghostos_runtime_unregister(state: *mut CRuntimeState, caller: u32, capacity: usize) -> c_uint;
    fn ghostos_runtime_prepare_filesystem_request(
        state: *const CRuntimeState,
        caller: u32,
        request: *const Request,
        capacity: usize,
        prepared: *mut CRuntimeFilesystemRequest,
    ) -> c_uint;
    fn ghostos_runtime_validate_request(request: *const Request) -> c_uint;
    fn ghostos_runtime_operation_delegated(operation: u16) -> bool;
    fn ghostos_runtime_validate_empty_request(request: *const Request, capability_allowed: bool) -> c_uint;
    fn ghostos_runtime_validate_filesystem_response(
        operation: u16,
        status: u32,
        values: *const u64,
        has_buffer: bool,
        buffer_length: u32,
    ) -> c_uint;
}

fn c_runtime_error(result: c_uint) -> Result<(), RuntimeDispatchError> {
    match result {
        0 => Ok(()),
        1 => Err(RuntimeDispatchError::AbiMismatch),
        2 => Err(RuntimeDispatchError::InvalidRequest),
        3 => Err(RuntimeDispatchError::ProcessNotRegistered),
        4 => Err(RuntimeDispatchError::Capacity),
        5 => Err(RuntimeDispatchError::InvalidCapability),
        6 => Err(RuntimeDispatchError::InvalidBuffer),
        7 => Err(RuntimeDispatchError::TransportFailure),
        _ => Err(RuntimeDispatchError::TransportFailure),
    }
}

/// The kernel IPC endpoint used by the runtime dispatcher.
///
/// An implementation normally sends one message to `ghostos-fsd`, waits for its
/// response, and validates the shared mapping in both address spaces. Keeping
/// this as a transport lets the kernel stay independent from the daemon's
/// scheduler while making the ABI translation explicit and testable.
pub trait FilesystemIpc {
    fn transact(
        &mut self,
        caller: AddressSpaceId,
        request: ghostos_fsd::Request,
        buffer: Option<SharedBuffer>,
    ) -> ghostos_fsd::Response;
}

pub trait FilesystemService {
    fn dispatch(
        &mut self,
        request: ghostos_fsd::Request,
        buffer: Option<&mut [u8]>,
    ) -> ghostos_fsd::Response;
}

impl<
        const MAX_BLOCKS: usize,
        const MAX_PROCESSES: usize,
        const MAX_OPEN_FILES: usize,
        const MAX_SNAPSHOTS: usize,
        const MAX_MOUNTS: usize,
        const SCRATCH_BYTES: usize,
    > FilesystemService
    for ghostos_fsd::Daemon<
        MAX_BLOCKS,
        MAX_PROCESSES,
        MAX_OPEN_FILES,
        MAX_SNAPSHOTS,
        MAX_MOUNTS,
        SCRATCH_BYTES,
    >
{
    fn dispatch(
        &mut self,
        request: ghostos_fsd::Request,
        buffer: Option<&mut [u8]>,
    ) -> ghostos_fsd::Response {
        ghostos_fsd::Daemon::dispatch(self, request, buffer)
    }
}

pub trait SharedBufferResolver {
    fn resolve(
        &mut self,
        owner: AddressSpaceId,
        buffer: SharedBuffer,
    ) -> Result<&mut [u8], RuntimeDispatchError>;
}

/// Kernel extension point for compiler/runtime operations that need a
/// scheduler, process table, entropy source, terminal, or VM manager. The
/// fixed request remains capability-checked by the service implementation;
/// the filesystem dispatcher stays independent from those subsystems.
pub trait RuntimeOperationService {
    fn dispatch(
        &mut self,
        caller: AddressSpaceId,
        operation: Operation,
        request: Request,
    ) -> Result<Response, RuntimeDispatchError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilesystemEndpoints {
    pub request_channel: ChannelId,
    pub response_channel: ChannelId,
    pub kernel_request: CapabilityHandle,
    pub daemon_request: CapabilityHandle,
    pub daemon_response: CapabilityHandle,
    pub kernel_response: CapabilityHandle,
    pub buffer_authority: CapabilityHandle,
}

/// A synchronous adapter over two capability-protected kernel IPC channels.
///
/// The request is put on the kernel-to-daemon ring, consumed by the daemon
/// endpoint, executed with a mapped shared buffer, and returned on the
/// daemon-to-kernel ring. A real scheduler can replace the inline service
/// call with a wake-and-resume operation without changing the ABI dispatcher.
pub struct ChannelFilesystemIpc<
    Service,
    Memory,
    const REQUEST_CAPACITY: usize = 8,
    const RESPONSE_CAPACITY: usize = 8,
    const MAX_CAPABILITIES: usize = { crate::MAX_CAPABILITIES },
> {
    request_channel: Channel<REQUEST_CAPACITY>,
    response_channel: Channel<RESPONSE_CAPACITY>,
    capabilities: CapabilitySpace<MAX_CAPABILITIES>,
    endpoints: FilesystemEndpoints,
    service: Service,
    memory: Memory,
    daemon: AddressSpaceId,
}

impl<
        Service: FilesystemService,
        Memory: SharedBufferResolver,
        const REQUEST_CAPACITY: usize,
        const RESPONSE_CAPACITY: usize,
        const MAX_CAPABILITIES: usize,
    > ChannelFilesystemIpc<
        Service,
        Memory,
        REQUEST_CAPACITY,
        RESPONSE_CAPACITY,
        MAX_CAPABILITIES,
    >
{
    pub fn new(
        capabilities: CapabilitySpace<MAX_CAPABILITIES>,
        endpoints: FilesystemEndpoints,
        service: Service,
        memory: Memory,
        daemon: AddressSpaceId,
    ) -> Self {
        Self {
            request_channel: Channel::new(
                endpoints.request_channel,
            ),
            response_channel: Channel::new(
                endpoints.response_channel,
            ),
            capabilities,
            endpoints,
            service,
            memory,
            daemon,
        }
    }

    pub fn service(&self) -> &Service {
        &self.service
    }

    pub fn service_mut(&mut self) -> &mut Service {
        &mut self.service
    }

    pub fn memory(&self) -> &Memory {
        &self.memory
    }

    pub fn memory_mut(&mut self) -> &mut Memory {
        &mut self.memory
    }
}

impl<
        Service: FilesystemService,
        Memory: SharedBufferResolver,
        const REQUEST_CAPACITY: usize,
        const RESPONSE_CAPACITY: usize,
        const MAX_CAPABILITIES: usize,
    > FilesystemIpc
    for ChannelFilesystemIpc<
        Service,
        Memory,
        REQUEST_CAPACITY,
        RESPONSE_CAPACITY,
        MAX_CAPABILITIES,
    >
{
    fn transact(
        &mut self,
        _caller: AddressSpaceId,
        request: ghostos_fsd::Request,
        buffer: Option<SharedBuffer>,
    ) -> ghostos_fsd::Response {
        let request_message = Message::from(request.to_envelope(buffer));
        if self
            .request_channel
            .try_send(
                &self.capabilities,
                AddressSpaceId::KERNEL,
                self.endpoints.kernel_request,
                Some(self.endpoints.buffer_authority),
                request_message,
            )
            .is_err()
        {
            return ghostos_fsd::Response::error(Status::BUSY)
        }

        let incoming = match self.request_channel.try_receive(
            &self.capabilities,
            self.daemon,
            self.endpoints.daemon_request,
        ) {
            Ok(message) => message,
            Err(_) => return ghostos_fsd::Response::error(Status::BUSY),
        };
        let (request, descriptor) = match ghostos_fsd::Request::from_envelope(incoming.into()) {
            Ok(value) => value,
            Err(_) => return ghostos_fsd::Response::error(Status::INVALID_ARGUMENT),
        };
        let response = match descriptor {
            Some(descriptor) => match self.memory.resolve(self.daemon, descriptor) {
                Ok(buffer) => self.service.dispatch(request, Some(buffer)),
                Err(_) => ghostos_fsd::Response::error(Status::ACCESS_DENIED),
            },
            None => self.service.dispatch(request, None),
        };

        let response_message = Message::from(response.to_envelope(incoming.correlation.raw()));
        if self
            .response_channel
            .try_send(
                &self.capabilities,
                self.daemon,
                self.endpoints.daemon_response,
                None,
                response_message,
            )
            .is_err()
        {
            return ghostos_fsd::Response::error(Status::BUSY)
        }
        let outgoing = match self.response_channel.try_receive(
            &self.capabilities,
            AddressSpaceId::KERNEL,
            self.endpoints.kernel_response,
        ) {
            Ok(message) => message,
            Err(_) => return ghostos_fsd::Response::error(Status::BUSY),
        };
        let envelope: Envelope = outgoing.into();
        if envelope.label != ghostos_fsd::RESPONSE_LABEL
            || envelope.correlation != incoming.correlation.raw()
        {
            return ghostos_fsd::Response::error(Status::INVALID_ARGUMENT)
        }
        let Ok(status_raw) = u32::try_from(envelope.words[0]) else {
            return ghostos_fsd::Response::error(Status::INVALID_ARGUMENT)
        };
        let Some(status) = Status::from_raw(status_raw) else {
            return ghostos_fsd::Response::error(Status::INVALID_ARGUMENT)
        };
        ghostos_fsd::Response {
            status,
            values: [
                envelope.words[1],
                envelope.words[2],
                envelope.words[3],
                0,
            ],
        }
    }
}

/// Runtime ABI dispatcher owned by the kernel.
pub struct Dispatcher<T, const MAX_PROCESSES: usize = MAX_FILESYSTEM_PROCESSES> {
    filesystem: T,
    state: CRuntimeState,
}

impl<T: FilesystemIpc, const MAX_PROCESSES: usize> Dispatcher<T, MAX_PROCESSES> {
    pub const fn new(filesystem: T) -> Self {
        Self {
            filesystem,
            state: CRuntimeState {
                clock: 0,
                processes: [CProcessSlot { caller: 0, used: 0, process: 0, authority: 0 }; MAX_FILESYSTEM_PROCESSES],
            },
        }
    }

    pub const fn filesystem(&self) -> &T {
        &self.filesystem
    }

    pub fn filesystem_mut(&mut self) -> &mut T {
        &mut self.filesystem
    }

    pub fn clock(&self) -> u64 {
        unsafe { ghostos_runtime_clock(&self.state) }
    }

    pub fn advance_clock(&mut self, elapsed: u64) {
        unsafe { ghostos_runtime_advance_clock(&mut self.state, elapsed) }
    }

    /// Associates a kernel address space with the daemon-issued process
    /// authority. The authority is attenuated by the daemon before this call.
    pub fn register_filesystem_process(
        &mut self,
        caller: AddressSpaceId,
        identity: FilesystemIdentity,
    ) -> Result<(), RuntimeDispatchError> {
        let result = unsafe {
            ghostos_runtime_register(
                &mut self.state,
                caller.raw(),
                CRuntimeIdentity { process: identity.process.raw(), authority: identity.authority.raw() },
                MAX_PROCESSES,
            )
        };
        match result {
            4 => Err(RuntimeDispatchError::ProcessNotRegistered),
            _ => c_runtime_error(result),
        }
    }

    pub fn unregister_filesystem_process(
        &mut self,
        caller: AddressSpaceId,
    ) -> Result<(), RuntimeDispatchError> {
        let result = unsafe { ghostos_runtime_unregister(&mut self.state, caller.raw(), MAX_PROCESSES) };
        c_runtime_error(result)
    }

    /// Dispatch one native runtime request on behalf of `caller`.
    pub fn dispatch(&mut self, caller: AddressSpaceId, request: Request) -> Response {
        match self.dispatch_checked(caller, request) {
            Ok(response) => response,
            Err(error) => Response {
                status: error.status().raw(),
                flags: 0,
                values: [0; 4],
            },
        }
    }

    /// Dispatch the extended PAL operations through the kernel subsystem that
    /// owns them. Basic clock, wait, and GhostFS calls keep their existing path.
    pub fn dispatch_with_operations<O: RuntimeOperationService>(
        &mut self,
        caller: AddressSpaceId,
        request: Request,
        operations: &mut O,
    ) -> Response {
        if let Err(error) = c_runtime_error(unsafe { ghostos_runtime_validate_request(&request) }) {
            return Response {
                status: error.status().raw(),
                flags: 0,
                values: [0; 4],
            }
        }
        let Some(operation) = Operation::from_raw(request.operation) else {
            return self.dispatch(caller, request);
        };
        if unsafe { ghostos_runtime_operation_delegated(operation as u16) } {
            match operations.dispatch(caller, operation, request) {
                Ok(response) => response,
                Err(error) => Response {
                    status: error.status().raw(),
                    flags: 0,
                    values: [0; 4],
                },
            }
        } else {
            self.dispatch(caller, request)
        }
    }

    fn dispatch_checked(
        &mut self,
        caller: AddressSpaceId,
        request: Request,
    ) -> Result<Response, RuntimeDispatchError> {
        c_runtime_error(unsafe { ghostos_runtime_validate_request(&request) })?;
        let operation = Operation::from_raw(request.operation)
            .ok_or(RuntimeDispatchError::InvalidRequest)?;

        match operation {
            Operation::ClockNow => {
                c_runtime_error(unsafe { ghostos_runtime_validate_empty_request(&request, false) })?;
                Ok(Response {
                    status: Status::NORMAL.raw(),
                    flags: 0,
                    values: [self.clock(), 0, 0, 0],
                })
            }
            Operation::RealtimeNow => {
                c_runtime_error(unsafe { ghostos_runtime_validate_empty_request(&request, false) })?;
                let now_ns = crate::time::realtime_now_ns()
                    .ok_or(RuntimeDispatchError::TransportFailure)?;
                Ok(Response {
                    status: Status::NORMAL.raw(),
                    flags: 0,
                    values: [now_ns, 0, 0, 0],
                })
            }
            Operation::Yield => {
                c_runtime_error(unsafe { ghostos_runtime_validate_empty_request(&request, false) })?;
                Ok(Response {
                    status: Status::NORMAL.raw(),
                    flags: 0,
                    values: [0; 4],
                })
            }
            Operation::SynFsOpen
            | Operation::SynFsClose
            | Operation::SynFsRead
            | Operation::SynFsWrite
            | Operation::SynFsMap
            | Operation::SynFsUnmap
            | Operation::SynFsMetadata
            | Operation::SynFsList
            | Operation::SynFsMkdir
            | Operation::SynFsRmdir
            | Operation::SynFsLink
            | Operation::SynFsLinks
            | Operation::SynFsDelete => self.dispatch_filesystem(caller, operation, request),
            _ => Err(RuntimeDispatchError::InvalidRequest),
        }
    }

    fn dispatch_filesystem(
        &mut self,
        caller: AddressSpaceId,
        operation: Operation,
        request: Request,
    ) -> Result<Response, RuntimeDispatchError> {
        let mut prepared = CRuntimeFilesystemRequest {
            operation: 0, flags: 0, process: 0, capability: 0, offset: 0, length: 0,
            region: 0, buffer_offset: 0, buffer_length: 0, writable: 0, has_buffer: 0,
            reserved: 0,
        };
        c_runtime_error(unsafe {
            ghostos_runtime_prepare_filesystem_request(
                &self.state, caller.raw(), &request, MAX_PROCESSES, &mut prepared,
            )
        })?;
        let buffer = if prepared.has_buffer != 0 {
            Some(SharedBuffer {
                region: SharedRegionId::new(prepared.region).ok_or(RuntimeDispatchError::InvalidBuffer)?,
                offset: prepared.buffer_offset,
                length: prepared.buffer_length,
                writable: prepared.writable != 0,
            })
        } else {
            None
        };
        let fs_operation = FsdOperation::from_raw(prepared.operation as u16)
            .ok_or(RuntimeDispatchError::InvalidRequest)?;
        let fs_request = ghostos_fsd::Request::new(
            fs_operation,
            ghostos_fsd::ProcessId::from_valid_raw(prepared.process),
        )
            .with_flags(FsdFlags::from_bits(prepared.flags as u16))
            .with_capability(FsdCapability::from_valid_raw(prepared.capability))
            .with_offset(prepared.offset)
            .with_length(prepared.length);
        let response = self.filesystem.transact(caller, fs_request, buffer);
        let response = Response {
            status: response.status.raw(),
            flags: 0,
            values: response.values,
        };
        self.validate_filesystem_response(operation, buffer, response)
    }

    fn validate_filesystem_response(
        &self,
        operation: Operation,
        buffer: Option<SharedBuffer>,
        response: Response,
    ) -> Result<Response, RuntimeDispatchError> {
        if Status::from_raw(response.status).is_none() {
            return Err(RuntimeDispatchError::TransportFailure)
        }
        c_runtime_error(unsafe {
            ghostos_runtime_validate_filesystem_response(
                operation as u16,
                response.status,
                response.values.as_ptr(),
                buffer.is_some(),
                buffer.map(|buffer| buffer.length).unwrap_or(0),
            )
        })?;
        Ok(response)
    }
}
