use synos_ipc::{Ring, RingError, SharedBuffer};
use synos_observability::{
    CapabilityDomain, CapabilityTrace, CapabilityTraceStage, Level, ProfileDomain, ProfileSample,
    record_profile_sample,
};
use synos_status::{IntoStatus, Severity, Status, facility};

use crate::memory::{MemoryError, SharedMemory};
use crate::protocol::{ProtocolError, SocketOperation, SocketRequest, socket_response};
use crate::{CapabilityRight, Firewall, FirewallError};

pub const DEFAULT_NETWORK_TENANT_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkRatePolicy {
    pub tenant: u64,
    pub burst_bytes: u64,
    pub bytes_per_second: u64,
}

impl NetworkRatePolicy {
    pub const fn new(tenant: u64, burst_bytes: u64, bytes_per_second: u64) -> Self {
        Self {
            tenant,
            burst_bytes,
            bytes_per_second,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkShapeDecision {
    Allowed,
    Throttled { retry_after_ms: u64 },
}

#[derive(Clone, Copy)]
struct NetworkTenantBucket {
    policy: NetworkRatePolicy,
    tokens: u64,
    last_ms: u64,
}

/// Fixed-size per-tenant egress shaping. Empty entries remain unrestricted for
/// compatibility with existing socket clients; configured tenants share no
/// token bucket with their neighbors.
pub struct NetworkShaper<const TENANTS: usize = DEFAULT_NETWORK_TENANT_CAPACITY> {
    buckets: [Option<NetworkTenantBucket>; TENANTS],
}

impl<const TENANTS: usize> NetworkShaper<TENANTS> {
    pub const fn new() -> Self {
        Self { buckets: [None; TENANTS] }
    }

    pub fn configure(&mut self, policy: NetworkRatePolicy) -> Result<(), ServiceError> {
        if policy.tenant == 0 || policy.burst_bytes == 0 || policy.bytes_per_second == 0 {
            return Err(ServiceError::InvalidOperation)
        }
        if let Some(bucket) = self
            .buckets
            .iter_mut()
            .flatten()
            .find(|bucket| bucket.policy.tenant == policy.tenant)
        {
            bucket.policy = policy;
            bucket.tokens = policy.burst_bytes;
            bucket.last_ms = 0;
            return Ok(())
        }
        let slot = self
            .buckets
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ServiceError::NoSocketSpace)?;
        *slot = Some(NetworkTenantBucket {
            policy,
            tokens: policy.burst_bytes,
            last_ms: 0,
        });
        Ok(())
    }

    pub fn admit(
        &mut self,
        tenant: u64,
        now_ms: u64,
        bytes: u64,
    ) -> NetworkShapeDecision {
        let Some(bucket) = self
            .buckets
            .iter_mut()
            .flatten()
            .find(|bucket| bucket.policy.tenant == tenant)
        else {
            return NetworkShapeDecision::Allowed
        };
        let elapsed = now_ms.saturating_sub(bucket.last_ms);
        let refill = (elapsed as u128)
            .saturating_mul(bucket.policy.bytes_per_second as u128)
            .checked_div(1_000)
            .unwrap_or(u128::MAX)
            .min(u64::MAX as u128) as u64;
        bucket.tokens = bucket
            .tokens
            .saturating_add(refill)
            .min(bucket.policy.burst_bytes);
        bucket.last_ms = now_ms;
        if bytes <= bucket.tokens {
            bucket.tokens -= bytes;
            return NetworkShapeDecision::Allowed
        }
        let deficit = bytes.saturating_sub(bucket.tokens);
        let retry_after_ms = deficit
            .saturating_mul(1_000)
            .saturating_add(bucket.policy.bytes_per_second - 1)
            / bucket.policy.bytes_per_second;
        NetworkShapeDecision::Throttled { retry_after_ms }
    }

    pub fn refund(&mut self, tenant: u64, bytes: u64) {
        if let Some(bucket) = self
            .buckets
            .iter_mut()
            .flatten()
            .find(|bucket| bucket.policy.tenant == tenant)
        {
            bucket.tokens = bucket.tokens.saturating_add(bytes).min(bucket.policy.burst_bytes)
        }
    }
}

impl<const TENANTS: usize> Default for NetworkShaper<TENANTS> {
    fn default() -> Self {
        Self::new()
    }
}

pub type DefaultFirewall = Firewall;

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

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
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
    Firewall(FirewallError),
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
                .unwrap_or(Status::INVALID_ARGUMENT),
            Self::Firewall(FirewallError::ExpiredCapability | FirewallError::InvalidCapability) => {
                Status::ACCESS_DENIED
            }
            Self::Firewall(_) => Status::INVALID_ARGUMENT,
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
        let capability = SocketCapability::from_parts(index, slot.generation);
        if let Some(trace) = CapabilityTrace::new(
            CapabilityDomain::Network,
            CapabilityTraceStage::Created,
            capability.raw(),
            SocketOperation::OpenTcp.raw(),
        ) {
            trace.emit(Level::Info)
        }
        Ok(capability)
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
        if let Some(trace) = CapabilityTrace::new(
            CapabilityDomain::Network,
            CapabilityTraceStage::DaemonAuthorized,
            capability.raw(),
            required.bits(),
        ) {
            trace.emit(Level::Trace)
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
        if let Some(trace) = CapabilityTrace::new(
            CapabilityDomain::Network,
            CapabilityTraceStage::Revoked,
            capability.raw(),
            SocketOperation::Close.raw(),
        ) {
            trace.emit(Level::Info)
        }
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
    firewall: DefaultFirewall,
    shaper: NetworkShaper,
}

impl<B: SocketBackend, const SOCKET_CAPACITY: usize> NetworkDaemon<B, SOCKET_CAPACITY> {
    pub fn new(backend: B) -> Self {
        let mut policy = crate::FirewallPolicy::new();
        crate::install_core_network_rules(&mut policy)
            .expect("core network firewall rules fit the default policy");
        Self {
            backend,
            sockets: SocketTable::new(),
            firewall: DefaultFirewall::new(policy),
            shaper: NetworkShaper::new(),
        }
    }

    pub fn with_firewall(backend: B, firewall: DefaultFirewall) -> Self {
        Self {
            backend,
            sockets: SocketTable::new(),
            firewall,
            shaper: NetworkShaper::new(),
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn firewall(&self) -> &DefaultFirewall {
        &self.firewall
    }

    pub fn firewall_mut(&mut self) -> &mut DefaultFirewall {
        &mut self.firewall
    }

    pub fn network_shaper(&self) -> &NetworkShaper {
        &self.shaper
    }

    pub fn network_shaper_mut(&mut self) -> &mut NetworkShaper {
        &mut self.shaper
    }

    /// Processes at most `budget` requests. The caller regains control after
    /// the budget or the first backend error, so network work cannot monopolize
    /// the service thread or starve other scheduler tasks.
    pub fn process_budget<M: SharedMemory, const RING_CAPACITY: usize>(
        &mut self,
        channel: &ClientChannel<'_, RING_CAPACITY>,
        memory: &mut M,
        budget: usize,
    ) -> Result<usize, ServiceError> {
        self.process_budget_at(channel, memory, budget, 0)
    }

    pub fn process_budget_at<M: SharedMemory, const RING_CAPACITY: usize>(
        &mut self,
        channel: &ClientChannel<'_, RING_CAPACITY>,
        memory: &mut M,
        budget: usize,
        now_ms: u64,
    ) -> Result<usize, ServiceError> {
        let mut processed = 0;
        for _ in 0..budget {
            match self.process_one_at(channel, memory, now_ms)? {
                true => processed += 1,
                false => break,
            }
        }
        Ok(processed)
    }

    /// Handles at most one request so the scheduler controls daemon latency.
    pub fn process_one<M: SharedMemory, const RING_CAPACITY: usize>(
        &mut self,
        channel: &ClientChannel<'_, RING_CAPACITY>,
        memory: &mut M,
    ) -> Result<bool, ServiceError> {
        self.process_one_at(channel, memory, 0)
    }

    pub fn process_one_at<M: SharedMemory, const RING_CAPACITY: usize>(
        &mut self,
        channel: &ClientChannel<'_, RING_CAPACITY>,
        memory: &mut M,
        now_ms: u64,
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
            .and_then(|request| self.execute(channel, memory, request, now_ms));
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
        now_ms: u64,
    ) -> Result<OperationResult, ServiceError> {
        record_profile_sample(ProfileSample::single(
            ProfileDomain::Networking,
            request.operation.raw() as u64,
            0,
            0x5001,
        ));
        if let Some(capability) = request.capability {
            if let Some(trace) = CapabilityTrace::new(
                CapabilityDomain::Network,
                CapabilityTraceStage::KernelIpc,
                capability.raw(),
                request.operation.raw(),
            ) {
                trace.emit(Level::Trace)
            }
        }
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
                self.firewall
                    .authorize_socket_endpoint(
                        CapabilityRight::Listen,
                        [0; 4],
                        port,
                        channel.allowed_rights.contains(SocketRights::LISTEN),
                    )
                    .map_err(ServiceError::Firewall)?;
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
                self.firewall
                    .authorize_socket_endpoint(
                        CapabilityRight::Connect,
                        address,
                        port,
                        channel.allowed_rights.contains(SocketRights::CONNECT),
                    )
                    .map_err(ServiceError::Firewall)?;
                self.backend.connect_ipv4(handle, address, port)?;
                Ok(OperationResult::capability(capability))
            }
            SocketOperation::Send => {
                let (capability, handle) =
                    self.authorize(channel, request, SocketRights::SEND)?;
                let descriptor = readable_buffer(request.buffer)?;
                match self.shaper.admit(
                    channel.principal,
                    now_ms,
                    descriptor.length as u64,
                ) {
                    NetworkShapeDecision::Allowed => {}
                    NetworkShapeDecision::Throttled { .. } => return Err(ServiceError::WouldBlock),
                }
                let written = match memory.read(descriptor, |bytes| self.backend.send(handle, bytes)) {
                    Ok(Ok(written)) => written,
                    Ok(Err(error)) => {
                        self.shaper.refund(channel.principal, descriptor.length as u64);
                        return Err(error)
                    }
                    Err(error) => {
                        self.shaper.refund(channel.principal, descriptor.length as u64);
                        return Err(error.into())
                    }
                };
                self.shaper.refund(
                    channel.principal,
                    (descriptor.length as usize).saturating_sub(written) as u64,
                );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_tenant_cannot_burst_past_its_egress_bucket() {
        let mut shaper = NetworkShaper::<1>::new();
        shaper.configure(NetworkRatePolicy::new(7, 10, 10)).unwrap();
        assert_eq!(shaper.admit(7, 0, 10), NetworkShapeDecision::Allowed);
        assert!(matches!(
            shaper.admit(7, 0, 1),
            NetworkShapeDecision::Throttled { .. }
        ));
        assert_eq!(shaper.admit(7, 1_000, 10), NetworkShapeDecision::Allowed);
    }
}
