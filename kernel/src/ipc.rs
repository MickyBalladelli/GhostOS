use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::capability::{CapabilityHandle, CapabilityObject, CapabilitySpace, Rights};
use crate::partition::CorePartition;
use crate::quota::{QuotaDecision, QuotaResource};
use crate::scheduler::{Scheduler, SchedulerError};
use crate::task::{AddressSpaceId, CpuId, ThreadId};
use ghostos_ipc::{Envelope, Ring, RingError};
use ghostos_observability::{
    CapabilityDomain, CapabilityTraceStage, CorrelationId, EventField, EventKind,
    ProfileDomain, ProfileSample, emit_capability_trace, field, next_correlation_id,
    record_profile_sample, trace,
};
use ghostos_status::{IntoStatus, Severity, Status, facility};

pub use ghostos_ipc::{ChannelId, SharedBuffer, SharedRegionId};

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
    Closed,
    Deadlock,
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
            Self::Closed => Status::NOT_FOUND,
            Self::Deadlock => Status::BUSY,
            Self::AccessDenied => Status::ACCESS_DENIED,
            Self::CoreIsolated | Self::RateLimited { .. } => Status::BUSY,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointCloseReport {
    pub revoked_capabilities: usize,
    pub discarded_messages: usize,
    pub channel_closed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointCleanupReport {
    pub endpoints: usize,
    pub revoked_capabilities: usize,
    pub discarded_messages: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelDiagnostics {
    pub channel: ChannelId,
    pub closed: bool,
    pub pending: usize,
    pub high_watermark: usize,
    pub enqueued: u64,
    pub dequeued: u64,
    pub full_events: u64,
    pub rate_limited_events: u64,
    pub last_enqueue_us: u64,
    pub last_dequeue_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StuckChannelReport {
    pub channel: ChannelId,
    pub pending: usize,
    pub stalled_for_us: u64,
    pub full_events: u64,
    pub last_enqueued_correlation: u128,
    pub last_enqueued_label: u64,
}

/// A bounded, non-blocking MPMC channel.
///
/// Messages carry a shared-region descriptor instead of copying payload bytes.
/// The sender and receiver address spaces must already map that region.
pub struct Channel<const CAPACITY: usize> {
    id: ChannelId,
    ring: Ring<CAPACITY>,
    closed: AtomicBool,
    high_watermark: AtomicUsize,
    enqueued: AtomicU64,
    dequeued: AtomicU64,
    full_events: AtomicU64,
    rate_limited_events: AtomicU64,
    last_enqueue_us: AtomicU64,
    last_dequeue_us: AtomicU64,
    last_correlation_low: AtomicU64,
    last_correlation_high: AtomicU64,
    last_label: AtomicU64,
}

impl<const CAPACITY: usize> Channel<CAPACITY> {
    pub fn new(id: ChannelId) -> Self {
        assert!(CAPACITY >= 2);
        Self {
            id,
            ring: Ring::new(),
            closed: AtomicBool::new(false),
            high_watermark: AtomicUsize::new(0),
            enqueued: AtomicU64::new(0),
            dequeued: AtomicU64::new(0),
            full_events: AtomicU64::new(0),
            rate_limited_events: AtomicU64::new(0),
            last_enqueue_us: AtomicU64::new(0),
            last_dequeue_us: AtomicU64::new(0),
            last_correlation_low: AtomicU64::new(0),
            last_correlation_high: AtomicU64::new(0),
            last_label: AtomicU64::new(0),
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
        self.debug_check_ownership(capabilities, caller, endpoint, Rights::SEND);

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
            return Err(self.rate_limited(retry_after_us))
        }
        if decision == QuotaDecision::Rejected {
            return Err(self.rate_limited(u64::MAX))
        }
        match self.enqueue(message, now_us) {
            Ok(()) => {
                record_profile_sample(ProfileSample::single(
                    ProfileDomain::Ipc,
                    now_us,
                    0,
                    0x2001,
                ));
                emit_capability_trace(
                    ghostos_observability::Level::Trace,
                    CapabilityDomain::Kernel,
                    CapabilityTraceStage::KernelIpc,
                    endpoint.raw(),
                    message.label as u16,
                );
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
                match scheduler.ipc_wait(self.id.raw(), owner, waiter) {
                    Ok(()) => Err(IpcError::Full),
                    Err(SchedulerError::IpcDeadlock) => Err(IpcError::Deadlock),
                    Err(_) => Err(IpcError::Full),
                }
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
        self.debug_check_ownership(capabilities, caller, endpoint, Rights::SEND);
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
                    self.rate_limited(retry_after_us)
                }
                QuotaDecision::Rejected => self.rate_limited(u64::MAX),
                QuotaDecision::Allowed => unreachable!(),
            })
        }
        if let Err(error) = self.enqueue(message, now_us) {
            let _ = capabilities.delete(receiver, delegated);
            let _ = capabilities.refund_quota(
                caller,
                endpoint,
                QuotaResource::IpcMessages,
                1,
            );
            return Err(error);
        }
        emit_capability_trace(
            ghostos_observability::Level::Trace,
            CapabilityDomain::Kernel,
            CapabilityTraceStage::KernelIpc,
            delegated.raw(),
            message.label as u16,
        );
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

    fn enqueue(&self, mut message: Message, now_us: u64) -> Result<(), IpcError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(IpcError::Closed)
        }
        if message.correlation.is_none() {
            message.correlation = next_correlation_id(1)
        }
        if let Err(error) = self.ring.try_send(message.into()) {
            match error {
                RingError::Full => {
                    self.full_events.fetch_add(1, Ordering::Relaxed);
                    trace!(
                        EventKind::Ipc,
                        EventField::unsigned(field::CHANNEL, self.id.raw() as u64),
                        EventField::unsigned(field::OPERATION, 3),
                    );
                    return Err(IpcError::Full)
                }
                RingError::Empty => unreachable!(),
            }
        }
        self.enqueued.fetch_add(1, Ordering::Relaxed);
        self.last_enqueue_us.store(now_us, Ordering::Release);
        self.last_correlation_low
            .store(message.correlation.raw() as u64, Ordering::Relaxed);
        self.last_correlation_high
            .store((message.correlation.raw() >> 64) as u64, Ordering::Relaxed);
        self.last_label.store(message.label, Ordering::Release);
        self.high_watermark.fetch_max(self.pending(), Ordering::Relaxed);
        trace!(
            EventKind::Ipc,
            EventField::unsigned(field::CHANNEL, self.id.raw() as u64),
            EventField::identifier(field::OPERATION, message.correlation.raw()),
        );
        crate::invariants::debug_assert_valid(self.check_queue_invariants());
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
        self.debug_check_ownership(capabilities, caller, endpoint, Rights::RECEIVE);

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
                    self.rate_limited(retry_after_us)
                }
                QuotaDecision::Rejected => self.rate_limited(u64::MAX),
                QuotaDecision::Allowed => unreachable!(),
            })
        }
        match self.dequeue(now_us) {
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

    fn dequeue(&self, now_us: u64) -> Result<Message, IpcError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(IpcError::Closed)
        }
        let result = self
            .ring
            .try_receive()
            .map(Message::from)
            .map_err(|error| match error {
                RingError::Empty => IpcError::Empty,
                RingError::Full => unreachable!(),
            });
        if let Ok(message) = result {
            self.dequeued.fetch_add(1, Ordering::Relaxed);
            self.last_dequeue_us.store(now_us, Ordering::Release);
            trace!(
                EventKind::Ipc,
                EventField::unsigned(field::CHANNEL, self.id.raw() as u64),
                EventField::identifier(field::OPERATION, message.correlation.raw()),
            )
        }
        crate::invariants::debug_assert_valid(self.check_queue_invariants());
        result
    }

    fn rate_limited(&self, retry_after_us: u64) -> IpcError {
        self.rate_limited_events.fetch_add(1, Ordering::Relaxed);
        trace!(
            EventKind::Ipc,
            EventField::unsigned(field::CHANNEL, self.id.raw() as u64),
            EventField::unsigned(field::OPERATION, 4),
            EventField::unsigned(field::LENGTH, retry_after_us),
        );
        IpcError::RateLimited { retry_after_us }
    }

    /// Delete one endpoint capability. Closing a receive endpoint closes the
    /// whole channel and drains requests that can no longer be delivered.
    pub fn close_endpoint<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &mut CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
    ) -> Result<EndpointCloseReport, IpcError> {
        let info = capabilities
            .inspect(caller, endpoint)
            .map_err(|_| IpcError::AccessDenied)?;
        if info.object != CapabilityObject::IpcChannel(self.id)
            || (!info.rights.contains(Rights::SEND)
                && !info.rights.contains(Rights::RECEIVE))
        {
            return Err(IpcError::AccessDenied)
        }
        let closes_channel = info.rights.contains(Rights::RECEIVE);
        let revoked_capabilities = capabilities
            .delete(caller, endpoint)
            .map_err(|_| IpcError::AccessDenied)?;
        let mut discarded_messages = 0;
        if closes_channel && !self.closed.swap(true, Ordering::AcqRel) {
            while self.ring.try_receive().is_ok() {
                discarded_messages += 1
            }
            trace!(
                EventKind::Ipc,
                EventField::unsigned(field::CHANNEL, self.id.raw() as u64),
                EventField::unsigned(field::OPERATION, 5),
                EventField::unsigned(field::LENGTH, discarded_messages as u64),
            )
        }
        Ok(EndpointCloseReport {
            revoked_capabilities,
            discarded_messages,
            channel_closed: self.closed.load(Ordering::Acquire),
        })
    }

    /// Remove every endpoint owned by a terminated address space.
    pub fn cleanup_owner<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &mut CapabilitySpace<MAX_CAPABILITIES>,
        owner: AddressSpaceId,
    ) -> EndpointCleanupReport {
        let mut handles = [None; MAX_CAPABILITIES];
        let mut count = 0;
        for (handle, info) in capabilities.entries() {
            if info.owner == owner && info.object == CapabilityObject::IpcChannel(self.id) {
                handles[count] = Some(handle);
                count += 1
            }
        }
        let mut report = EndpointCleanupReport {
            endpoints: 0,
            revoked_capabilities: 0,
            discarded_messages: 0,
        };
        for handle in handles[..count].iter().flatten().copied() {
            if let Ok(closed) = self.close_endpoint(capabilities, owner, handle) {
                report.endpoints += 1;
                report.revoked_capabilities = report
                    .revoked_capabilities
                    .saturating_add(closed.revoked_capabilities);
                report.discarded_messages = report
                    .discarded_messages
                    .saturating_add(closed.discarded_messages)
            }
        }
        report
    }

    pub fn diagnostics(&self) -> ChannelDiagnostics {
        ChannelDiagnostics {
            channel: self.id,
            closed: self.closed.load(Ordering::Acquire),
            pending: self.pending(),
            high_watermark: self.high_watermark.load(Ordering::Relaxed),
            enqueued: self.enqueued.load(Ordering::Relaxed),
            dequeued: self.dequeued.load(Ordering::Relaxed),
            full_events: self.full_events.load(Ordering::Relaxed),
            rate_limited_events: self.rate_limited_events.load(Ordering::Relaxed),
            last_enqueue_us: self.last_enqueue_us.load(Ordering::Acquire),
            last_dequeue_us: self.last_dequeue_us.load(Ordering::Acquire),
        }
    }

    pub fn stuck_report(
        &self,
        now_us: u64,
        threshold_us: u64,
    ) -> Option<StuckChannelReport> {
        if threshold_us == 0 || self.closed.load(Ordering::Acquire) {
            return None
        }
        let diagnostics = self.diagnostics();
        if diagnostics.pending == 0 {
            return None
        }
        let last_progress = if diagnostics.last_dequeue_us == 0 {
            diagnostics.last_enqueue_us
        } else {
            diagnostics.last_dequeue_us
        };
        let stalled_for_us = now_us.saturating_sub(last_progress);
        (stalled_for_us >= threshold_us).then_some(StuckChannelReport {
            channel: self.id,
            pending: diagnostics.pending,
            stalled_for_us,
            full_events: diagnostics.full_events,
            last_enqueued_correlation: self.last_correlation_low.load(Ordering::Relaxed) as u128
                | ((self.last_correlation_high.load(Ordering::Relaxed) as u128) << 64),
            last_enqueued_label: self.last_label.load(Ordering::Acquire),
        })
    }

    pub fn pending(&self) -> usize {
        self.ring.pending()
    }

    /// Check queue bounds and the capability owner without exposing queue data.
    pub fn check_invariants<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        required: Rights,
    ) -> Result<(), crate::invariants::InvariantFailure> {
        self.check_queue_invariants()?;
        if capabilities
                .authorize(
                    caller,
                    endpoint,
                    CapabilityObject::IpcChannel(self.id),
                    required,
                )
                .is_err()
        {
            return Err(crate::invariants::InvariantFailure::new(
                crate::invariants::InvariantId::IpcOwnership,
            ))
        }
        Ok(())
    }

    fn check_queue_invariants(&self) -> Result<(), crate::invariants::InvariantFailure> {
        if self.id.raw() == 0 || self.pending() > CAPACITY {
            return Err(crate::invariants::InvariantFailure::new(
                crate::invariants::InvariantId::IpcOwnership,
            ))
        }
        Ok(())
    }

    fn debug_check_ownership<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        required: Rights,
    ) {
        crate::invariants::debug_assert_valid(self.check_invariants(
            capabilities,
            caller,
            endpoint,
            required,
        ))
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
        self.channel.debug_check_ownership(
            self.capabilities,
            self.caller,
            self.endpoint,
            Rights::SEND,
        );
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
                    self.channel.rate_limited(retry_after_us)
                }
                QuotaDecision::Rejected => self.channel.rate_limited(u64::MAX),
                QuotaDecision::Allowed => unreachable!(),
            })
        }
        match self.channel.enqueue(message, now_us) {
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
                match scheduler.ipc_wait(self.channel.id.raw(), owner, waiter) {
                    Ok(()) => Err(IpcError::Full),
                    Err(SchedulerError::IpcDeadlock) => Err(IpcError::Deadlock),
                    Err(_) => Err(IpcError::Full),
                }
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
        self.channel.debug_check_ownership(
            self.capabilities,
            self.caller,
            self.endpoint,
            Rights::RECEIVE,
        );
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
                    self.channel.rate_limited(retry_after_us)
                }
                QuotaDecision::Rejected => self.channel.rate_limited(u64::MAX),
                QuotaDecision::Allowed => unreachable!(),
            })
        }
        match self.channel.dequeue(now_us) {
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
