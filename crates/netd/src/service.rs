use synos_ipc::{Ring, RingError, SharedBuffer};
use synos_status::{IntoStatus, Severity, Status, facility};

use crate::memory::{MemoryError, SharedMemory};
use crate::protocol::{ProtocolError, SocketOperation, SocketRequest, socket_response};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct SocketCapability(u64);

impl SocketCapability {
    const fn from_parts(slot: usize, generation: u32) -> Self {
        Self(((generation as u64) << 32) | slot as u64)
    }

    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw >> 32 == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    const fn slot(self) -> usize {
        self.0 as u32 as usize
    }

    const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct SocketRights(u16);

impl SocketRights {
    pub const NONE: Self = Self(0);
    pub const LISTEN: Self = Self(1 << 0);
    pub const CONNECT: Self = Self(1 << 1);
    pub const SEND: Self = Self(1 << 2);
    pub const RECEIVE: Self = Self(1 << 3);
    pub const CLOSE: Self = Self(1 << 4);
    pub const INSPECT: Self = Self(1 << 5);
    pub const ALL: Self = Self((1 << 6) - 1);

    pub const fn from_bits(bits: u16) -> Option<Self> {
        if bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SocketState {
    Closed = 0,
    Listening = 1,
    Connecting = 2,
    Established = 3,
    Closing = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceError {
    AccessDenied,
    Backend,
    CompletionFull,
    InvalidBuffer,
    InvalidCapability,
    InvalidOperation,
    InvalidRights,
    NoSocketSpace,
    WouldBlock,
}

impl IntoStatus for ServiceError {
    fn status(self) -> Status {
        match self {
            Self::AccessDenied => Status::ACCESS_DENIED,
            Self::CompletionFull | Self::WouldBlock => Status::BUSY,
            Self::NoSocketSpace => Status::NO_SPACE,
            Self::InvalidBuffer
            | Self::InvalidCapability
            | Self::InvalidOperation
            | Self::InvalidRights => Status::INVALID_ARGUMENT,
            Self::Backend => Status::new(Severity::Error, facility::NETWORK, 1, 0)
                .expect("valid network status"),
        }
    }
}

impl From<MemoryError> for ServiceError {
    fn from(_: MemoryError) -> Self {
        Self::InvalidBuffer
    }
}

impl From<ProtocolError> for ServiceError {
    fn from(error: ProtocolError) -> Self {
        match error {
            ProtocolError::InvalidCapability => Self::InvalidCapability,
            ProtocolError::InvalidRights => Self::InvalidRights,
            ProtocolError::InvalidOperation
            | ProtocolError::InvalidSchema
            | ProtocolError::InvalidVersion => Self::InvalidOperation,
        }
    }
}

pub trait SocketBackend {
    type Handle: Copy;

    fn open_tcp(&mut self) -> Result<Self::Handle, ServiceError>;
    fn listen(&mut self, handle: Self::Handle, port: u16) -> Result<(), ServiceError>;
    fn connect_ipv4(
        &mut self,
        handle: Self::Handle,
        address: [u8; 4],
        port: u16,
    ) -> Result<(), ServiceError>;
    fn send(&mut self, handle: Self::Handle, bytes: &[u8]) -> Result<usize, ServiceError>;
    fn receive(
        &mut self,
        handle: Self::Handle,
        bytes: &mut [u8],
    ) -> Result<usize, ServiceError>;
    fn close(&mut self, handle: Self::Handle);
    fn state(&self, handle: Self::Handle) -> SocketState;
}

#[derive(Clone, Copy)]
struct SocketSlot<H: Copy> {
    generation: u32,
    occupied: bool,
    owner: u64,
    rights: SocketRights,
    handle: Option<H>,
}

impl<H: Copy> SocketSlot<H> {
    const EMPTY: Self = Self {
        generation: 0,
        occupied: false,
        owner: 0,
        rights: SocketRights::NONE,
        handle: None,
    };
}

struct SocketTable<H: Copy, const CAPACITY: usize> {
    slots: [SocketSlot<H>; CAPACITY],
}

impl<H: Copy, const CAPACITY: usize> SocketTable<H, CAPACITY> {
    const fn new() -> Self {
        Self {
            slots: [SocketSlot::EMPTY; CAPACITY],
        }
    }

    fn insert(
        &mut self,
        owner: u64,
        rights: SocketRights,
        handle: H,
    ) -> Result<SocketCapability, ServiceError> {
        let (index, slot) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| !slot.occupied)
            .ok_or(ServiceError::NoSocketSpace)?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.occupied = true;
        slot.owner = owner;
        slot.rights = rights;
        slot.handle = Some(handle);
        Ok(SocketCapability::from_parts(index, slot.generation))
    }

    fn authorize(
        &self,
        owner: u64,
        capability: SocketCapability,
        required: SocketRights,
    ) -> Result<H, ServiceError> {
        let slot = self
            .slots
            .get(capability.slot())
            .ok_or(ServiceError::InvalidCapability)?;
        if !slot.occupied || slot.generation != capability.generation() {
            return Err(ServiceError::InvalidCapability)
        }
        if slot.owner != owner || !slot.rights.contains(required) {
            return Err(ServiceError::AccessDenied)
        }
        slot.handle.ok_or(ServiceError::InvalidCapability)
    }

    fn remove(
        &mut self,
        owner: u64,
        capability: SocketCapability,
    ) -> Result<H, ServiceError> {
        let slot = self
            .slots
            .get_mut(capability.slot())
            .ok_or(ServiceError::InvalidCapability)?;
        if !slot.occupied || slot.generation != capability.generation() {
            return Err(ServiceError::InvalidCapability)
        }
        if slot.owner != owner || !slot.rights.contains(SocketRights::CLOSE) {
            return Err(ServiceError::AccessDenied)
        }
        let handle = slot.handle.take().ok_or(ServiceError::InvalidCapability)?;
        slot.occupied = false;
        slot.owner = 0;
        slot.rights = SocketRights::NONE;
        Ok(handle)
    }
}

pub struct ClientChannel<'a, const RING_CAPACITY: usize> {
    pub principal: u64,
    pub allowed_rights: SocketRights,
    pub requests: &'a Ring<RING_CAPACITY>,
    pub completions: &'a Ring<RING_CAPACITY>,
}

/// Capability gate in front of a user-space socket backend.
pub struct NetworkDaemon<B: SocketBackend, const SOCKET_CAPACITY: usize> {
    backend: B,
    sockets: SocketTable<B::Handle, SOCKET_CAPACITY>,
}

impl<B: SocketBackend, const SOCKET_CAPACITY: usize> NetworkDaemon<B, SOCKET_CAPACITY> {
    pub const fn new(backend: B) -> Self {
        Self {
            backend,
            sockets: SocketTable::new(),
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Handles at most one request so the scheduler controls daemon latency.
    pub fn process_one<M: SharedMemory, const RING_CAPACITY: usize>(
        &mut self,
        channel: &ClientChannel<'_, RING_CAPACITY>,
        memory: &mut M,
    ) -> Result<bool, ServiceError> {
        if channel.completions.pending() >= RING_CAPACITY {
            return Err(ServiceError::CompletionFull)
        }
        let envelope = match channel.requests.try_receive() {
            Ok(envelope) => envelope,
            Err(RingError::Empty) => return Ok(false),
            Err(RingError::Full) => unreachable!(),
        };
        let correlation = envelope.correlation;
        let buffer = envelope.buffer;
        let outcome = SocketRequest::decode(envelope)
            .map_err(ServiceError::from)
            .and_then(|request| self.execute(channel, memory, request));
        let (status, capability, value, response_buffer) = match outcome {
            Ok(result) => (
                Status::NORMAL,
                result.capability,
                result.value,
                result.buffer,
            ),
            Err(error) => (error.status(), None, 0, buffer),
        };
        channel
            .completions
            .try_send(socket_response(
                correlation,
                status,
                capability,
                value,
                response_buffer,
            ))
            .map_err(|_| ServiceError::CompletionFull)?;
        Ok(true)
    }

    fn execute<M: SharedMemory, const RING_CAPACITY: usize>(
        &mut self,
        channel: &ClientChannel<'_, RING_CAPACITY>,
        memory: &mut M,
        request: SocketRequest,
    ) -> Result<OperationResult, ServiceError> {
        match request.operation {
            SocketOperation::OpenTcp => {
                let rights_bits = u16::try_from(request.argument0)
                    .map_err(|_| ServiceError::InvalidRights)?;
                let rights = SocketRights::from_bits(rights_bits)
                    .ok_or(ServiceError::InvalidRights)?;
                if rights.is_empty() || !channel.allowed_rights.contains(rights) {
                    return Err(ServiceError::AccessDenied)
                }
                let handle = self.backend.open_tcp()?;
                match self.sockets.insert(channel.principal, rights, handle) {
                    Ok(capability) => Ok(OperationResult::capability(capability)),
                    Err(error) => {
                        self.backend.close(handle);
                        Err(error)
                    }
                }
            }
            SocketOperation::Listen => {
                let (capability, handle) =
                    self.authorize(channel, request, SocketRights::LISTEN)?;
                let port = port(request.argument0)?;
                self.backend.listen(handle, port)?;
                Ok(OperationResult::capability(capability))
            }
            SocketOperation::ConnectIpv4 => {
                let (capability, handle) =
                    self.authorize(channel, request, SocketRights::CONNECT)?;
                let address = u32::try_from(request.argument0)
                    .map_err(|_| ServiceError::InvalidOperation)?
                    .to_be_bytes();
                let port = port(request.argument1)?;
                self.backend.connect_ipv4(handle, address, port)?;
                Ok(OperationResult::capability(capability))
            }
            SocketOperation::Send => {
                let (capability, handle) =
                    self.authorize(channel, request, SocketRights::SEND)?;
                let descriptor = readable_buffer(request.buffer)?;
                let written = memory
                    .read(descriptor, |bytes| self.backend.send(handle, bytes))??;
                Ok(OperationResult::transfer(capability, written, descriptor))
            }
            SocketOperation::Receive => {
                let (capability, handle) =
                    self.authorize(channel, request, SocketRights::RECEIVE)?;
                let descriptor = writable_buffer(request.buffer)?;
                let received = memory
                    .write(descriptor, |bytes| self.backend.receive(handle, bytes))??;
                Ok(OperationResult::transfer(capability, received, descriptor))
            }
            SocketOperation::Close => {
                let capability = request.capability.ok_or(ServiceError::InvalidCapability)?;
                let handle = self.sockets.remove(channel.principal, capability)?;
                self.backend.close(handle);
                Ok(OperationResult::capability(capability))
            }
            SocketOperation::State => {
                let (capability, handle) =
                    self.authorize(channel, request, SocketRights::INSPECT)?;
                Ok(OperationResult {
                    capability: Some(capability),
                    value: self.backend.state(handle) as u64,
                    buffer: None,
                })
            }
        }
    }

    fn authorize<const RING_CAPACITY: usize>(
        &self,
        channel: &ClientChannel<'_, RING_CAPACITY>,
        request: SocketRequest,
        rights: SocketRights,
    ) -> Result<(SocketCapability, B::Handle), ServiceError> {
        let capability = request.capability.ok_or(ServiceError::InvalidCapability)?;
        let handle = self
            .sockets
            .authorize(channel.principal, capability, rights)?;
        Ok((capability, handle))
    }
}

struct OperationResult {
    capability: Option<SocketCapability>,
    value: u64,
    buffer: Option<SharedBuffer>,
}

impl OperationResult {
    const fn capability(capability: SocketCapability) -> Self {
        Self {
            capability: Some(capability),
            value: 0,
            buffer: None,
        }
    }

    const fn transfer(
        capability: SocketCapability,
        length: usize,
        buffer: SharedBuffer,
    ) -> Self {
        Self {
            capability: Some(capability),
            value: length as u64,
            buffer: Some(buffer),
        }
    }
}

fn port(raw: u64) -> Result<u16, ServiceError> {
    let port = u16::try_from(raw).map_err(|_| ServiceError::InvalidOperation)?;
    if port == 0 {
        Err(ServiceError::InvalidOperation)
    } else {
        Ok(port)
    }
}

fn readable_buffer(buffer: Option<SharedBuffer>) -> Result<SharedBuffer, ServiceError> {
    let buffer = buffer.ok_or(ServiceError::InvalidBuffer)?;
    if buffer.length == 0 || buffer.writable {
        Err(ServiceError::InvalidBuffer)
    } else {
        Ok(buffer)
    }
}

fn writable_buffer(buffer: Option<SharedBuffer>) -> Result<SharedBuffer, ServiceError> {
    let buffer = buffer.ok_or(ServiceError::InvalidBuffer)?;
    if buffer.length == 0 || !buffer.writable {
        Err(ServiceError::InvalidBuffer)
    } else {
        Ok(buffer)
    }
}
