use crate::capability::{CapabilityHandle, CapabilityObject, CapabilitySpace, Rights};
use crate::partition::CorePartition;
use crate::quota::{QuotaDecision, QuotaResource};
use crate::scheduler::Scheduler;
use crate::task::{AddressSpaceId, CpuId, ThreadId};
use synos_ipc::{Envelope, Ring, RingError};
use synos_observability::{
    CapabilityDomain, CapabilityTrace, CapabilityTraceStage, CorrelationId, EventField,
    EventKind, field, next_correlation_id, trace,
};
use synos_status::{IntoStatus, Severity, Status, facility};

pub use synos_ipc::{ChannelId, SharedBuffer, SharedRegionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Message {
    pub correlation: CorrelationId,
    pub label: u64,
    pub buffer: Option<SharedBuffer>,
    pub words: [u64; 4],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityTransfer {
    pub delegated: CapabilityHandle,
    pub receiver: AddressSpaceId,
    pub rights: Rights,
}

impl Message {
    pub const EMPTY: Self = Self {
        correlation: CorrelationId::NONE,
        label: 0,
        buffer: None,
        words: [0; 4],
    };
}

impl From<Message> for Envelope {
    fn from(message: Message) -> Self {
        Self {
            correlation: message.correlation.raw(),
            label: message.label,
            buffer: message.buffer,
            words: message.words,
        }
    }
}

impl From<Envelope> for Message {
    fn from(envelope: Envelope) -> Self {
        Self {
            correlation: CorrelationId::from_raw(envelope.correlation),
            label: envelope.label,
            buffer: envelope.buffer,
            words: envelope.words,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpcError {
    Full,
    Empty,
    AccessDenied,
    CoreIsolated,
    RateLimited { retry_after_us: u64 },
}

impl IntoStatus for IpcError {
    fn status(self) -> Status {
        match self {
            Self::Full => Status::BUSY,
            Self::Empty => Status::new(Severity::Information, facility::KERNEL, 2, 0)
                .expect("valid IPC status"),
            Self::AccessDenied => Status::ACCESS_DENIED,
            Self::CoreIsolated | Self::RateLimited { .. } => Status::BUSY,
        }
    }
}

/// A bounded, non-blocking MPMC channel.
///
/// Messages carry a shared-region descriptor instead of copying payload bytes.
/// The sender and receiver address spaces must already map that region.
pub struct Channel<const CAPACITY: usize> {
    id: ChannelId,
    ring: Ring<CAPACITY>,
}

impl<const CAPACITY: usize> Channel<CAPACITY> {
    pub fn new(id: ChannelId) -> Self {
        assert!(CAPACITY >= 2);
        Self {
            id,
            ring: Ring::new(),
        }
    }

    pub const fn id(&self) -> ChannelId {
        self.id
    }

    pub fn try_send<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        buffer_authority: Option<CapabilityHandle>,
        message: Message,
    ) -> Result<(), IpcError> {
        self.try_send_at(
            capabilities,
            caller,
            endpoint,
            buffer_authority,
            0,
            message,
        )
    }

    pub fn try_send_at<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        buffer_authority: Option<CapabilityHandle>,
        now_us: u64,
        message: Message,
    ) -> Result<(), IpcError> {
        capabilities
            .authorize(
                caller,
                endpoint,
                CapabilityObject::IpcChannel(self.id),
                Rights::SEND,
            )
            .map_err(|_| IpcError::AccessDenied)?;

        if let Some(buffer) = message.buffer {
            let handle = buffer_authority.ok_or(IpcError::AccessDenied)?;
            capabilities
                .authorize_mapping(caller, handle, buffer.region, buffer.writable, false)
                .map_err(|_| IpcError::AccessDenied)?;
        }

        let decision = capabilities
            .consume_quota(
                caller,
                endpoint,
                QuotaResource::IpcMessages,
                now_us,
                1,
            )
            .map_err(|_| IpcError::AccessDenied)?;
        if let QuotaDecision::Throttled { retry_after_us } = decision {
            return Err(IpcError::RateLimited { retry_after_us })
        }
        if decision == QuotaDecision::Rejected {
            return Err(IpcError::RateLimited { retry_after_us: u64::MAX })
        }
        match self.enqueue(message) {
            Ok(()) => {
                if let Some(trace) = CapabilityTrace::new(
                    CapabilityDomain::Kernel,
                    CapabilityTraceStage::KernelIpc,
                    endpoint.raw(),
                    message.label as u16,
                ) {
                    trace.emit(synos_observability::Level::Trace)
                }
                Ok(())
            }
            Err(error) => {
                let _ = capabilities.refund_quota(
                    caller,
                    endpoint,
                    QuotaResource::IpcMessages,
                    1,
                );
                Err(error)
            }
        }
    }

    pub fn try_send_on<const MAX_CAPABILITIES: usize>(
        &self,
        partition: CorePartition,
        cpu: CpuId,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        buffer_authority: Option<CapabilityHandle>,
        message: Message,
    ) -> Result<(), IpcError> {
        self.try_send_on_at(
            partition,
            cpu,
            capabilities,
            caller,
            endpoint,
            buffer_authority,
            0,
            message,
        )
    }

    pub fn try_send_on_at<const MAX_CAPABILITIES: usize>(
        &self,
        partition: CorePartition,
        cpu: CpuId,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        buffer_authority: Option<CapabilityHandle>,
        now_us: u64,
        message: Message,
    ) -> Result<(), IpcError> {
        if !partition.accepts_ipc(cpu) {
            return Err(IpcError::CoreIsolated)
        }
        self.try_send_at(
            capabilities,
            caller,
            endpoint,
            buffer_authority,
            now_us,
            message,
        )
    }

    /// Send through a bounded queue while propagating a blocked sender's
    /// effective priority to the service thread that owns the endpoint.
    pub fn try_send_with_priority<const MAX_CAPABILITIES: usize>(
        &self,
        scheduler: &mut Scheduler,
        owner: ThreadId,
        waiter: ThreadId,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        buffer_authority: Option<CapabilityHandle>,
        message: Message,
    ) -> Result<(), IpcError> {
        match self.try_send(
            capabilities,
            caller,
            endpoint,
            buffer_authority,
            message,
        ) {
            Ok(()) => {
                scheduler.ipc_complete(self.id.raw(), waiter);
                Ok(())
            }
            Err(IpcError::Full) => {
                let _ = scheduler.ipc_wait(self.id.raw(), owner, waiter);
                Err(IpcError::Full)
            }
            Err(error) => Err(error),
        }
    }

    /// Atomically attenuate a capability and attach its handle to a zero-copy
    /// IPC message. The delegated handle is placed in word 3.
    #[allow(clippy::too_many_arguments)]
    pub fn try_send_delegated<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &mut CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        buffer_authority: Option<CapabilityHandle>,
        source: CapabilityHandle,
        receiver: AddressSpaceId,
        rights: Rights,
        message: Message,
    ) -> Result<CapabilityTransfer, IpcError> {
        self.try_send_delegated_at(
            capabilities,
            caller,
            endpoint,
            buffer_authority,
            source,
            receiver,
            rights,
            0,
            message,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn try_send_delegated_at<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &mut CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        buffer_authority: Option<CapabilityHandle>,
        source: CapabilityHandle,
        receiver: AddressSpaceId,
        rights: Rights,
        now_us: u64,
        mut message: Message,
    ) -> Result<CapabilityTransfer, IpcError> {
        capabilities
            .authorize(
                caller,
                endpoint,
                CapabilityObject::IpcChannel(self.id),
                Rights::SEND,
            )
            .map_err(|_| IpcError::AccessDenied)?;
        if let Some(buffer) = message.buffer {
            let handle = buffer_authority.ok_or(IpcError::AccessDenied)?;
            capabilities
                .authorize_mapping(caller, handle, buffer.region, buffer.writable, false)
                .map_err(|_| IpcError::AccessDenied)?
        }
        let delegated = capabilities
            .delegate(caller, source, receiver, rights)
            .map_err(|_| IpcError::AccessDenied)?;
        message.words[3] = delegated.raw();
        let decision = capabilities
            .consume_quota(
                caller,
                endpoint,
                QuotaResource::IpcMessages,
                now_us,
                1,
            )
            .map_err(|_| IpcError::AccessDenied)?;
        if decision != QuotaDecision::Allowed {
            let _ = capabilities.delete(receiver, delegated);
            return Err(match decision {
                QuotaDecision::Throttled { retry_after_us } => {
                    IpcError::RateLimited { retry_after_us }
                }
                QuotaDecision::Rejected => IpcError::RateLimited {
                    retry_after_us: u64::MAX,
                },
                QuotaDecision::Allowed => unreachable!(),
            })
        }
        if let Err(error) = self.enqueue(message) {
            let _ = capabilities.delete(receiver, delegated);
            let _ = capabilities.refund_quota(
                caller,
                endpoint,
                QuotaResource::IpcMessages,
                1,
            );
            return Err(error);
        }
        if let Some(trace) = CapabilityTrace::new(
            CapabilityDomain::Kernel,
            CapabilityTraceStage::KernelIpc,
            delegated.raw(),
            message.label as u16,
        ) {
            trace.emit(synos_observability::Level::Trace)
        }
        Ok(CapabilityTransfer {
            delegated,
            receiver,
            rights,
        })
    }

    /// Validate capabilities once, then use the mapped ring without syscalls.
    pub fn map_sender<'a, const MAX_CAPABILITIES: usize>(
        &'a self,
        capabilities: &'a CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        ring_memory: CapabilityHandle,
        ring_region: SharedRegionId,
    ) -> Result<MappedSender<'a, CAPACITY, MAX_CAPABILITIES>, IpcError> {
        capabilities
            .authorize(
                caller,
                endpoint,
                CapabilityObject::IpcChannel(self.id),
                Rights::SEND,
            )
            .map_err(|_| IpcError::AccessDenied)?;
        capabilities
            .authorize_mapping(caller, ring_memory, ring_region, true, false)
            .map_err(|_| IpcError::AccessDenied)?;
        Ok(MappedSender {
            channel: self,
            ring_region,
            capabilities,
            endpoint,
            caller,
        })
    }

    /// Validate capabilities once, then consume the read-only shared ring.
    pub fn map_receiver<'a, const MAX_CAPABILITIES: usize>(
        &'a self,
        capabilities: &'a CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        ring_memory: CapabilityHandle,
        ring_region: SharedRegionId,
    ) -> Result<MappedReceiver<'a, CAPACITY, MAX_CAPABILITIES>, IpcError> {
        capabilities
            .authorize(
                caller,
                endpoint,
                CapabilityObject::IpcChannel(self.id),
                Rights::RECEIVE,
            )
            .map_err(|_| IpcError::AccessDenied)?;
        capabilities
            .authorize_mapping(caller, ring_memory, ring_region, false, false)
            .map_err(|_| IpcError::AccessDenied)?;
        Ok(MappedReceiver {
            channel: self,
            ring_region,
            capabilities,
            endpoint,
            caller,
        })
    }

    fn enqueue(&self, mut message: Message) -> Result<(), IpcError> {
        if message.correlation.is_none() {
            message.correlation = next_correlation_id(1)
        }
        self.ring
            .try_send(message.into())
            .map_err(|error| match error {
                RingError::Full => IpcError::Full,
                RingError::Empty => unreachable!(),
            })?;
        trace!(
            EventKind::Ipc,
            EventField::unsigned(field::CHANNEL, self.id.raw() as u64),
            EventField::identifier(field::OPERATION, message.correlation.raw()),
        );
        Ok(())
    }

    pub fn try_receive<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
    ) -> Result<Message, IpcError> {
        self.try_receive_at(capabilities, caller, endpoint, 0)
    }

    pub fn try_receive_at<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        now_us: u64,
    ) -> Result<Message, IpcError> {
        capabilities
            .authorize(
                caller,
                endpoint,
                CapabilityObject::IpcChannel(self.id),
                Rights::RECEIVE,
            )
            .map_err(|_| IpcError::AccessDenied)?;

        let decision = capabilities
            .consume_quota(
                caller,
                endpoint,
                QuotaResource::IpcMessages,
                now_us,
                1,
            )
            .map_err(|_| IpcError::AccessDenied)?;
        if decision != QuotaDecision::Allowed {
            return Err(match decision {
                QuotaDecision::Throttled { retry_after_us } => {
                    IpcError::RateLimited { retry_after_us }
                }
                QuotaDecision::Rejected => IpcError::RateLimited {
                    retry_after_us: u64::MAX,
                },
                QuotaDecision::Allowed => unreachable!(),
            })
        }
        match self.dequeue() {
            Ok(message) => Ok(message),
            Err(error) => {
                let _ = capabilities.refund_quota(
                    caller,
                    endpoint,
                    QuotaResource::IpcMessages,
                    1,
                );
                Err(error)
            }
        }
    }

    pub fn try_receive_on<const MAX_CAPABILITIES: usize>(
        &self,
        partition: CorePartition,
        cpu: CpuId,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
    ) -> Result<Message, IpcError> {
        self.try_receive_on_at(partition, cpu, capabilities, caller, endpoint, 0)
    }

    pub fn try_receive_on_at<const MAX_CAPABILITIES: usize>(
        &self,
        partition: CorePartition,
        cpu: CpuId,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        now_us: u64,
    ) -> Result<Message, IpcError> {
        if !partition.accepts_ipc(cpu) {
            return Err(IpcError::CoreIsolated)
        }
        self.try_receive_at(capabilities, caller, endpoint, now_us)
    }

    fn dequeue(&self) -> Result<Message, IpcError> {
        self.ring
            .try_receive()
            .map(Message::from)
            .map_err(|error| match error {
                RingError::Empty => IpcError::Empty,
                RingError::Full => unreachable!(),
            })
    }

    pub fn pending(&self) -> usize {
        self.ring.pending()
    }
}

/// Direct producer view over a capability-mapped shared-memory ring.
///
/// Setup enters the kernel once. Steady-state sends are atomic memory
/// operations and may only reference the ring's own mapped region.
pub struct MappedSender<'a, const CAPACITY: usize, const MAX_CAPABILITIES: usize> {
    channel: &'a Channel<CAPACITY>,
    ring_region: SharedRegionId,
    capabilities: &'a CapabilitySpace<MAX_CAPABILITIES>,
    endpoint: CapabilityHandle,
    caller: AddressSpaceId,
}

impl<const CAPACITY: usize, const MAX_CAPABILITIES: usize>
    MappedSender<'_, CAPACITY, MAX_CAPABILITIES>
{
    pub fn try_send(&self, message: Message) -> Result<(), IpcError> {
        self.try_send_at(0, message)
    }

    pub fn try_send_at(&self, now_us: u64, message: Message) -> Result<(), IpcError> {
        if message
            .buffer
            .is_some_and(|buffer| buffer.region != self.ring_region)
        {
            return Err(IpcError::AccessDenied);
        }
        let decision = self
            .capabilities
            .consume_quota(
                self.caller,
                self.endpoint,
                QuotaResource::IpcMessages,
                now_us,
                1,
            )
            .map_err(|_| IpcError::AccessDenied)?;
        if decision != QuotaDecision::Allowed {
            return Err(match decision {
                QuotaDecision::Throttled { retry_after_us } => {
                    IpcError::RateLimited { retry_after_us }
                }
                QuotaDecision::Rejected => IpcError::RateLimited {
                    retry_after_us: u64::MAX,
                },
                QuotaDecision::Allowed => unreachable!(),
            })
        }
        match self.channel.enqueue(message) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = self.capabilities.refund_quota(
                    self.caller,
                    self.endpoint,
                    QuotaResource::IpcMessages,
                    1,
                );
                Err(error)
            }
        }
    }

    pub fn try_send_on(
        &self,
        partition: CorePartition,
        cpu: CpuId,
        message: Message,
    ) -> Result<(), IpcError> {
        if !partition.accepts_ipc(cpu) {
            return Err(IpcError::CoreIsolated)
        }
        self.try_send(message)
    }

    pub fn try_send_with_priority(
        &self,
        scheduler: &mut Scheduler,
        owner: ThreadId,
        waiter: ThreadId,
        message: Message,
    ) -> Result<(), IpcError> {
        match self.try_send(message) {
            Ok(()) => {
                scheduler.ipc_complete(self.channel.id.raw(), waiter);
                Ok(())
            }
            Err(IpcError::Full) => {
                let _ = scheduler.ipc_wait(self.channel.id.raw(), owner, waiter);
                Err(IpcError::Full)
            }
            Err(error) => Err(error),
        }
    }

    pub const fn region(&self) -> SharedRegionId {
        self.ring_region
    }
}

/// Direct consumer view over the read-only side of a shared-memory ring.
pub struct MappedReceiver<'a, const CAPACITY: usize, const MAX_CAPABILITIES: usize> {
    channel: &'a Channel<CAPACITY>,
    ring_region: SharedRegionId,
    capabilities: &'a CapabilitySpace<MAX_CAPABILITIES>,
    endpoint: CapabilityHandle,
    caller: AddressSpaceId,
}

impl<const CAPACITY: usize, const MAX_CAPABILITIES: usize>
    MappedReceiver<'_, CAPACITY, MAX_CAPABILITIES>
{
    pub fn try_receive(&self) -> Result<Message, IpcError> {
        self.try_receive_at(0)
    }

    pub fn try_receive_at(&self, now_us: u64) -> Result<Message, IpcError> {
        let decision = self
            .capabilities
            .consume_quota(
                self.caller,
                self.endpoint,
                QuotaResource::IpcMessages,
                now_us,
                1,
            )
            .map_err(|_| IpcError::AccessDenied)?;
        if decision != QuotaDecision::Allowed {
            return Err(match decision {
                QuotaDecision::Throttled { retry_after_us } => {
                    IpcError::RateLimited { retry_after_us }
                }
                QuotaDecision::Rejected => IpcError::RateLimited {
                    retry_after_us: u64::MAX,
                },
                QuotaDecision::Allowed => unreachable!(),
            })
        }
        match self.channel.dequeue() {
            Ok(message) => Ok(message),
            Err(error) => {
                let _ = self.capabilities.refund_quota(
                    self.caller,
                    self.endpoint,
                    QuotaResource::IpcMessages,
                    1,
                );
                Err(error)
            }
        }
    }

    pub const fn region(&self) -> SharedRegionId {
        self.ring_region
    }
}
