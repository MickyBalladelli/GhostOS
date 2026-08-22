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

#[derive(Clone, Copy)]
struct ProcessSlot {
    caller: Option<AddressSpaceId>,
    identity: Option<FilesystemIdentity>,
}

impl ProcessSlot {
    const EMPTY: Self = Self {
        caller: None,
        identity: None,
    };
}

/// Runtime ABI dispatcher owned by the kernel.
pub struct Dispatcher<T, const MAX_PROCESSES: usize = MAX_FILESYSTEM_PROCESSES> {
    filesystem: T,
    processes: [ProcessSlot; MAX_PROCESSES],
    clock: u64,
}

impl<T: FilesystemIpc, const MAX_PROCESSES: usize> Dispatcher<T, MAX_PROCESSES> {
    pub const fn new(filesystem: T) -> Self {
        Self {
            filesystem,
            processes: [ProcessSlot::EMPTY; MAX_PROCESSES],
            clock: 0,
        }
    }

    pub const fn filesystem(&self) -> &T {
        &self.filesystem
    }

    pub fn filesystem_mut(&mut self) -> &mut T {
        &mut self.filesystem
    }

    pub const fn clock(&self) -> u64 {
        self.clock
    }

    pub fn advance_clock(&mut self, elapsed: u64) {
        self.clock = self.clock.saturating_add(elapsed)
    }

    /// Associates a kernel address space with the daemon-issued process
    /// authority. The authority is attenuated by the daemon before this call.
    pub fn register_filesystem_process(
        &mut self,
        caller: AddressSpaceId,
        identity: FilesystemIdentity,
    ) -> Result<(), RuntimeDispatchError> {
        if let Some(slot) = self
            .processes
            .iter_mut()
            .find(|slot| slot.caller == Some(caller))
        {
            slot.identity = Some(identity);
            return Ok(())
        }
        let slot = self
            .processes
            .iter_mut()
            .find(|slot| slot.caller.is_none())
            .ok_or(RuntimeDispatchError::ProcessNotRegistered)?;
        slot.caller = Some(caller);
        slot.identity = Some(identity);
        Ok(())
    }

    pub fn unregister_filesystem_process(
        &mut self,
        caller: AddressSpaceId,
    ) -> Result<(), RuntimeDispatchError> {
        let slot = self
            .processes
            .iter_mut()
            .find(|slot| slot.caller == Some(caller))
            .ok_or(RuntimeDispatchError::ProcessNotRegistered)?;
        *slot = ProcessSlot::EMPTY;
        Ok(())
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
        if request.abi_version != ghostos_abi::ABI_SCHEMA_VERSION {
            return Response {
                status: RuntimeDispatchError::AbiMismatch.status().raw(),
                flags: 0,
                values: [0; 4],
            }
        }
        let Some(operation) = Operation::from_raw(request.operation) else {
            return self.dispatch(caller, request);
        };
        if !matches!(
            operation,
            Operation::ClockNow
                | Operation::RealtimeNow
                | Operation::Yield
                | Operation::SynFsOpen
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
                | Operation::SynFsDelete
        ) {
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
        if request.abi_version != ghostos_abi::ABI_SCHEMA_VERSION {
            return Err(RuntimeDispatchError::AbiMismatch)
        }
        let operation = Operation::from_raw(request.operation)
            .ok_or(RuntimeDispatchError::InvalidRequest)?;
        if request.reserved != 0 {
            return Err(RuntimeDispatchError::InvalidRequest)
        }

        match operation {
            Operation::ClockNow => {
                self.require_empty_request(request, false)?;
                Ok(Response {
                    status: Status::NORMAL.raw(),
                    flags: 0,
                    values: [self.clock, 0, 0, 0],
                })
            }
            Operation::RealtimeNow => {
                self.require_empty_request(request, false)?;
                let now_ns = crate::time::realtime_now_ns()
                    .ok_or(RuntimeDispatchError::TransportFailure)?;
                Ok(Response {
                    status: Status::NORMAL.raw(),
                    flags: 0,
                    values: [now_ns, 0, 0, 0],
                })
            }
            Operation::Yield => {
                self.require_empty_request(request, false)?;
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
        let slot = self
            .processes
            .iter()
            .find(|slot| slot.caller == Some(caller))
            .and_then(|slot| slot.identity)
            .ok_or(RuntimeDispatchError::ProcessNotRegistered)?;
        let buffer = self.buffer(operation, request)?;
        let fs_operation = match operation {
            Operation::SynFsOpen => FsdOperation::Open,
            Operation::SynFsClose => FsdOperation::Close,
            Operation::SynFsRead => FsdOperation::Read,
            Operation::SynFsWrite => FsdOperation::Write,
            Operation::SynFsMap => FsdOperation::Map,
            Operation::SynFsUnmap => FsdOperation::Unmap,
            Operation::SynFsMetadata => FsdOperation::Metadata,
            Operation::SynFsList => FsdOperation::List,
            Operation::SynFsMkdir => FsdOperation::Mkdir,
            Operation::SynFsRmdir => FsdOperation::Rmdir,
            Operation::SynFsLink => FsdOperation::Link,
            Operation::SynFsLinks => FsdOperation::Links,
            Operation::SynFsDelete => FsdOperation::Delete,
            _ => return Err(RuntimeDispatchError::InvalidRequest),
        };

        let capability = match operation {
            Operation::SynFsOpen
            | Operation::SynFsList
            | Operation::SynFsMkdir
            | Operation::SynFsRmdir
            | Operation::SynFsLinks
            | Operation::SynFsDelete => {
                if request.capability != 0 {
                    return Err(RuntimeDispatchError::InvalidRequest)
                }
                slot.authority
            }
            _ => FsdCapability::from_raw(request.capability)
                .ok_or(RuntimeDispatchError::InvalidCapability)?,
        };
        let flags = match operation {
            Operation::SynFsOpen => {
                if request.flags & !0x21f != 0 {
                    return Err(RuntimeDispatchError::InvalidRequest)
                }
                FsdFlags::from_bits(request.flags)
            }
            Operation::SynFsMkdir => {
                if request.flags & !(1 << 8) != 0 {
                    return Err(RuntimeDispatchError::InvalidRequest)
                }
                FsdFlags::from_bits(request.flags)
            }
            Operation::SynFsMap => {
                if request.flags & !FsdFlags::WRITE.bits() != 0 {
                    return Err(RuntimeDispatchError::InvalidRequest)
                }
                FsdFlags::from_bits(request.flags)
            }
            _ if request.flags == 0 => FsdFlags::from_bits(0),
            _ => return Err(RuntimeDispatchError::InvalidRequest),
        };
        if operation == Operation::SynFsOpen
            || operation == Operation::SynFsMkdir
            || operation == Operation::SynFsRmdir
            || operation == Operation::SynFsLink
            || operation == Operation::SynFsLinks
            || operation == Operation::SynFsDelete
        {
            if request.arguments[4] != 0 || request.arguments[5] != 0 {
                return Err(RuntimeDispatchError::InvalidRequest)
            }
        } else if operation == Operation::SynFsRead
            || operation == Operation::SynFsWrite
            || operation == Operation::SynFsList
        {
            if request.arguments[5] != 0 {
                return Err(RuntimeDispatchError::InvalidRequest)
            }
        } else if operation == Operation::SynFsMap {
            if request.arguments[5] == 0 || request.arguments[5] >> 48 != 0 {
                return Err(RuntimeDispatchError::InvalidRequest)
            }
        } else if request.arguments != [0; 6] {
            return Err(RuntimeDispatchError::InvalidRequest)
        }

        let fs_request = ghostos_fsd::Request::new(fs_operation, slot.process)
            .with_flags(flags)
            .with_capability(capability)
            .with_offset(request.arguments[4])
            .with_length(request.arguments[5]);
        let response = self.filesystem.transact(caller, fs_request, buffer);
        let response = Response {
            status: response.status.raw(),
            flags: 0,
            values: response.values,
        };
        self.validate_filesystem_response(operation, buffer, response)
    }

    fn require_empty_request(
        &self,
        request: Request,
        capability_allowed: bool,
    ) -> Result<(), RuntimeDispatchError> {
        if (!capability_allowed && request.capability != 0)
            || request.flags != 0
            || request.arguments != [0; 6]
        {
            return Err(RuntimeDispatchError::InvalidRequest)
        }
        Ok(())
    }

    fn buffer(
        &self,
        operation: Operation,
        request: Request,
    ) -> Result<Option<SharedBuffer>, RuntimeDispatchError> {
        let needs_buffer = matches!(
            operation,
            Operation::SynFsOpen
            | Operation::SynFsRead
            | Operation::SynFsWrite
            | Operation::SynFsList
            | Operation::SynFsMkdir
            | Operation::SynFsRmdir
            | Operation::SynFsLink
            | Operation::SynFsLinks
            | Operation::SynFsDelete
        );
        let has_descriptor = request.arguments[0] != 0
            || request.arguments[1] != 0
            || request.arguments[2] != 0
            || request.arguments[3] != 0;
        if !needs_buffer {
            if has_descriptor {
                return Err(RuntimeDispatchError::InvalidBuffer)
            }
            return Ok(None)
        }
        let region = SharedRegionId::new(
            u32::try_from(request.arguments[0]).map_err(|_| RuntimeDispatchError::InvalidBuffer)?,
        )
        .ok_or(RuntimeDispatchError::InvalidBuffer)?;
        let offset = u32::try_from(request.arguments[1])
            .map_err(|_| RuntimeDispatchError::InvalidBuffer)?;
        let length = u32::try_from(request.arguments[2])
            .map_err(|_| RuntimeDispatchError::InvalidBuffer)?;
        if request.arguments[3] > 1
            || offset.checked_add(length).is_none()
            || length as usize > ghostos_fsd::MAX_IPC_BUFFER_BYTES
        {
            return Err(RuntimeDispatchError::InvalidBuffer)
        }
        let writable = request.arguments[3] != 0;
        let expected_writable = operation == Operation::SynFsRead
            || operation == Operation::SynFsList
            || operation == Operation::SynFsLinks;
        if writable != expected_writable {
            return Err(RuntimeDispatchError::InvalidBuffer)
        }
        Ok(Some(SharedBuffer {
            region,
            offset,
            length,
            writable,
        }))
    }

    fn validate_filesystem_response(
        &self,
        operation: Operation,
        buffer: Option<SharedBuffer>,
        response: Response,
    ) -> Result<Response, RuntimeDispatchError> {
        let Some(status) = Status::from_raw(response.status) else {
            return Err(RuntimeDispatchError::TransportFailure)
        };
        if !status.is_success() {
            return Ok(response)
        }
        match operation {
            Operation::SynFsOpen => {
                if FsdCapability::from_raw(response.values[0]).is_none() {
                    return Err(RuntimeDispatchError::InvalidCapability)
                }
            }
            Operation::SynFsMap => {
                if FsdCapability::from_raw(response.values[0]).is_none()
                    || response.values[1] % 4096 != 0
                    || response.values[2] == 0
                    || response.values[2] % 4096 != 0
                    || response.values[3] > 1
                {
                    return Err(RuntimeDispatchError::InvalidCapability)
                }
            }
            Operation::SynFsRead | Operation::SynFsWrite => {
                let length = buffer
                    .map(|buffer| buffer.length as u64)
                    .ok_or(RuntimeDispatchError::InvalidBuffer)?;
                if response.values[0] > length {
                    return Err(RuntimeDispatchError::TransportFailure)
                }
            }
            Operation::SynFsList => {
                let length = buffer
                    .map(|buffer| buffer.length as u64)
                    .ok_or(RuntimeDispatchError::InvalidBuffer)?;
                if response.values[0] > length {
                    return Err(RuntimeDispatchError::TransportFailure)
                }
            }
            Operation::SynFsLinks => {
                let length = buffer
                    .map(|buffer| buffer.length as u64)
                    .ok_or(RuntimeDispatchError::InvalidBuffer)?;
                if response.values[0] > length {
                    return Err(RuntimeDispatchError::TransportFailure)
                }
            }
            Operation::SynFsClose
            | Operation::SynFsUnmap
            | Operation::SynFsMetadata
            | Operation::SynFsMkdir
            | Operation::SynFsLink => {}
            Operation::SynFsRmdir => {
                if response.values[3] > 1 {
                    return Err(RuntimeDispatchError::TransportFailure)
                }
            }
            Operation::SynFsDelete => {
                if response.values[0] > u64::from(u32::MAX)
                    || response.values[1] > 3
                    || response.values[2] > u64::from(u32::MAX)
                    || response.values[3] > 1
                {
                    return Err(RuntimeDispatchError::TransportFailure)
                }
            }
            _ => return Err(RuntimeDispatchError::InvalidRequest),
        }
        Ok(response)
    }
}
