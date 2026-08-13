#![no_std]
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use synos_status::Status;
pub use synos_system_model::ContentId;
pub use synos_service_scale::{
    EffectReceipt, HandoffReceipt, HandoffToken, InstanceId, InstanceState, JoinReceipt, RequestId,
    ObservabilityScale, RouteDecision, ServiceKind, SessionId, SessionState, ScaleError,
    ScaleSnapshot,
};

pub mod profiling;
pub mod slo;
pub mod throughput;
pub mod cache;
pub mod scaling;
pub mod cardinality;

pub use cardinality::{
    AuditLabelAggregate, AuditLabelAggregator, AtomicCardinality, CardinalityDecision,
    CardinalityEntry, CardinalitySnapshot, CardinalityTable, MAX_AUDIT_LABEL_CARDINALITY,
    MAX_METRIC_AGGREGATES, MAX_METRIC_CARDINALITY, MAX_TENANT_DIAGNOSTICS,
    MAX_TRACE_LABEL_CARDINALITY, TenantDiagnostic, TenantDiagnosticError,
    TenantDiagnosticKind, TenantDiagnosticOutcome, TenantDiagnostics,
};

pub use cache::{
    CacheEvent, CacheKind, CachePolicy, CachePolicyError, CachePolicyRegistry,
    CachePolicyReport, CACHE_RATE_SCALE, MAX_CACHE_POLICIES,
};

pub use profiling::{
    drain_profile, record_profile_sample, redacted_host_id, validate_profile_export,
    ProfileAggregator, ProfileDomain, ProfileError, ProfileFrame, ProfileMetadata, ProfileRing,
    ProfileSample, ProfileStack, ProfileRetention, RetainedProfile, GLOBAL_PROFILE_CAPACITY,
    MAX_PROFILE_FRAMES, MAX_PROFILE_STACKS, PROFILE_HEADER_BYTES, PROFILE_MAGIC,
    PROFILE_STACK_RECORD_BYTES, PROFILE_VERSION, SYSTEM_PROFILE,
};
pub use throughput::{BatchController, BatchDecision, ProducerPolicy};
pub use scaling::{
    AffinitySet, ScalePath, ScalePolicy, SCALE_CPU_TIERS,
};
pub use slo::{
    SloDefinition, SloError, SloKind, SloMeasurement, SloObservation, SloReport,
    SloReportStatus, BUDGET_SCALE, DEFAULT_MAX_AGE_US, DEFAULT_OBJECTIVE_PER_MILLION,
    DEFAULT_WINDOW_US, SLO_COUNT,
};

pub const MAX_EVENT_FIELDS: usize = 4;
pub const JOURNAL_RECORD_SIZE: usize = 128;
pub const GLOBAL_TRACE_CAPACITY: usize = 256;
pub const GLOBAL_AUDIT_CAPACITY: usize = 256;
pub const MAX_METRIC_SAMPLES: usize = 256;
pub const MAX_ALERTS: usize = 128;
pub const MAX_TELEMETRY_BATCH: usize = 32;
pub const MAX_OPERATIONAL_HEALTH: usize = 64;
pub const DEFAULT_RECOVERY_AUDIT_CAPACITY: usize = 256;
pub const AUDIT_EXPORT_HEADER_BYTES: usize = 64;
pub const SEALED_AUDIT_RECORD_BYTES: usize = 200;
pub const AUDIT_EXPORT_MAGIC: &[u8; 8] = b"SNAUDIT1";
pub const AUDIT_EXPORT_VERSION: u16 = 1;

const JOURNAL_MAGIC: u32 = u32::from_le_bytes(*b"SLOG");
const JOURNAL_VERSION: u8 = 1;
const SLOT_BUSY: u64 = 1 << 63;

pub mod field {
    pub const MESSAGE: u16 = 1;
    pub const STATUS: u16 = 2;
    pub const CAPABILITY: u16 = 3;
    pub const CALLER: u16 = 4;
    pub const OWNER: u16 = 5;
    pub const RIGHTS: u16 = 6;
    pub const OBJECT: u16 = 7;
    pub const AUTH_ACTION: u16 = 8;
    pub const IDENTITY: u16 = 9;
    pub const ADDRESS: u16 = 10;
    pub const LENGTH: u16 = 11;
    pub const CHANNEL: u16 = 12;
    pub const OPERATION: u16 = 13;
    pub const CLUSTER: u16 = 14;
    pub const TRANSPORT: u16 = 15;
    pub const WORKLOAD: u16 = 16;
    pub const TRACE: u16 = 17;
    pub const CAPABILITY_DOMAIN: u16 = 18;
    pub const CAPABILITY_STAGE: u16 = 19;
    pub const HEALTH: u16 = 20;
    pub const QUEUE_DEPTH: u16 = 21;
    pub const QUEUE_CAPACITY: u16 = 22;
    pub const DROPPED_PACKETS: u16 = 23;
    pub const RETRIES: u16 = 24;
    pub const DEGRADED_MODE: u16 = 25;
    pub const PING_TARGET: u16 = 26;
    pub const PING_INTERFACE: u16 = 27;
    pub const PING_SOURCE: u16 = 28;
    pub const PING_COUNT: u16 = 29;
    pub const PING_TIMEOUT: u16 = 30;
    pub const PING_RESULT: u16 = 31;
    pub const NUMA_KIND: u16 = 32;
    pub const NUMA_REQUESTED_NODE: u16 = 33;
    pub const NUMA_SELECTED_NODE: u16 = 34;
    pub const NUMA_LOCALITY: u16 = 35;
    pub const REMOTE_MEMORY_BYTES: u16 = 36;
    pub const TEMPERATURE_DECI_KELVIN: u16 = 37;
    pub const THROTTLE_PERCENT: u16 = 38;
    pub const THERMAL_ACTION: u16 = 39;
    pub const THERMAL_EVENT: u16 = 40;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CapabilityDomain {
    Kernel = 1,
    Filesystem = 2,
    Network = 3,
    Process = 4,
    Storage = 5,
    Cluster = 6,
    Compiler = 7,
}

impl CapabilityDomain {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Kernel),
            2 => Some(Self::Filesystem),
            3 => Some(Self::Network),
            4 => Some(Self::Process),
            5 => Some(Self::Storage),
            6 => Some(Self::Cluster),
            7 => Some(Self::Compiler),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Kernel => "kernel",
            Self::Filesystem => "filesystem",
            Self::Network => "network",
            Self::Process => "process",
            Self::Storage => "storage",
            Self::Cluster => "cluster",
            Self::Compiler => "compiler",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CapabilityTraceStage {
    Created = 1,
    KernelIpc = 2,
    DaemonAuthorized = 3,
    ShellOutput = 4,
    AuditRecorded = 5,
    Revoked = 6,
}

impl CapabilityTraceStage {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Created),
            2 => Some(Self::KernelIpc),
            3 => Some(Self::DaemonAuthorized),
            4 => Some(Self::ShellOutput),
            5 => Some(Self::AuditRecorded),
            6 => Some(Self::Revoked),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::KernelIpc => "kernel-ipc",
            Self::DaemonAuthorized => "daemon-authorized",
            Self::ShellOutput => "shell-output",
            Self::AuditRecorded => "audit-recorded",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct TelemetryDimensions {
    pub cluster: u128,
    pub node: u32,
    pub transport: u8,
    pub workload: u64,
    pub operation: u16,
}

impl TelemetryDimensions {
    pub const fn new(cluster: u128, node: u32, transport: u8, workload: u64, operation: u16) -> Self {
        Self { cluster, node, transport, workload, operation }
    }

    /// Stable low-cardinality bucket for values that exceed a series limit.
    pub const fn other() -> Self {
        Self {
            cluster: 0,
            node: 1,
            transport: 0,
            workload: 0,
            operation: 0,
        }
    }

    pub fn apply(self, event: TraceEvent) -> TraceEvent {
        event
            .on_node(self.node)
            .with_field(EventField::identifier(field::CLUSTER, self.cluster))
            .with_field(EventField::unsigned(field::TRANSPORT, self.transport as u64))
            .with_field(EventField::unsigned(field::WORKLOAD, self.workload))
            .with_field(EventField::unsigned(field::OPERATION, self.operation as u64))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HealthTransport {
    Http = 1,
    Grpc = 2,
    Sdk = 3,
    RemoteTerminal = 4,
    Mesh = 5,
    Cluster = 6,
    Dhcp = 7,
    Packet = 8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HealthState {
    Healthy = 1,
    Degraded = 2,
    Failed = 3,
}

impl HealthState {
    pub const fn status(self) -> Status {
        match self {
            Self::Healthy => Status::NORMAL,
            Self::Degraded => Status::CLUSTER_DEGRADED,
            Self::Failed => Status::NODE_UNSAFE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationalHealth {
    pub sampled_at_us: u64,
    pub node: u32,
    pub transport: HealthTransport,
    pub state: HealthState,
    pub queue_depth: u32,
    pub queue_capacity: u32,
    pub dropped_packets: u64,
    pub retries: u64,
    pub degraded_mode: bool,
}

impl OperationalHealth {
    pub const fn new(
        sampled_at_us: u64,
        node: u32,
        transport: HealthTransport,
        state: HealthState,
        queue_depth: u32,
        queue_capacity: u32,
        dropped_packets: u64,
        retries: u64,
        degraded_mode: bool,
    ) -> Option<Self> {
        if sampled_at_us == 0 || node == 0 || queue_depth > queue_capacity {
            None
        } else {
            Some(Self {
                sampled_at_us,
                node,
                transport,
                state,
                queue_depth,
                queue_capacity,
                dropped_packets,
                retries,
                degraded_mode,
            })
        }
    }

    pub const fn is_degraded(self) -> bool {
        self.degraded_mode || !matches!(self.state, HealthState::Healthy)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthError {
    Capacity,
    Duplicate,
    Invalid,
}

#[derive(Clone, Copy)]
pub struct HealthReport<const CAPACITY: usize = MAX_OPERATIONAL_HEALTH> {
    sampled_at_us: u64,
    samples: [Option<OperationalHealth>; CAPACITY],
}

impl<const CAPACITY: usize> HealthReport<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            sampled_at_us: 0,
            samples: [None; CAPACITY],
        }
    }

    pub const fn sampled_at_us(&self) -> u64 {
        self.sampled_at_us
    }

    pub fn set_sampled_at_us(&mut self, sampled_at_us: u64) {
        self.sampled_at_us = sampled_at_us
    }

    pub fn samples(&self) -> impl Iterator<Item = OperationalHealth> + '_ {
        self.samples.iter().flatten().copied()
    }

    pub fn push(&mut self, sample: OperationalHealth) -> Result<(), HealthError> {
        if sample.sampled_at_us == 0
            || sample.node == 0
            || sample.queue_depth > sample.queue_capacity
        {
            return Err(HealthError::Invalid)
        }
        if self
            .samples()
            .any(|entry| entry.node == sample.node && entry.transport == sample.transport)
        {
            return Err(HealthError::Duplicate)
        }
        let slot = self
            .samples
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(HealthError::Capacity)?;
        *slot = Some(sample);
        self.sampled_at_us = self.sampled_at_us.max(sample.sampled_at_us);
        Ok(())
    }

    pub fn healthy_count(&self) -> usize {
        self.samples()
            .filter(|sample| matches!(sample.state, HealthState::Healthy) && !sample.degraded_mode)
            .count()
    }

    pub fn degraded_count(&self) -> usize {
        self.samples()
            .filter(|sample| sample.is_degraded() && !matches!(sample.state, HealthState::Failed))
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.samples()
            .filter(|sample| matches!(sample.state, HealthState::Failed))
            .count()
    }

    pub fn queue_depth(&self) -> u64 {
        self.samples()
            .map(|sample| sample.queue_depth as u64)
            .sum()
    }

    pub fn queue_capacity(&self) -> u64 {
        self.samples()
            .map(|sample| sample.queue_capacity as u64)
            .sum()
    }

    pub fn dropped_packets(&self) -> u64 {
        self.samples()
            .map(|sample| sample.dropped_packets)
            .sum()
    }

    pub fn retries(&self) -> u64 {
        self.samples().map(|sample| sample.retries).sum()
    }

    pub fn degraded_mode(&self) -> bool {
        self.samples().any(OperationalHealth::is_degraded)
    }

    pub fn status(&self) -> Status {
        if self.failed_count() != 0 {
            Status::NODE_UNSAFE
        } else if self.degraded_mode() {
            Status::CLUSTER_DEGRADED
        } else {
            Status::NORMAL
        }
    }

    pub fn retain_node(&mut self, node: u32) {
        for entry in &mut self.samples {
            if entry.is_some_and(|sample| sample.node != node) {
                *entry = None
            }
        }
    }

    pub fn clear(&mut self) {
        *self = Self::new()
    }

    /// Emit two bounded audit records so queue and retry counters remain exact.
    pub fn emit_audit(&self) {
        for sample in self.samples() {
            emit_health_audit(sample)
        }
    }
}

impl<const CAPACITY: usize> Default for HealthReport<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

pub fn emit_health_audit(sample: OperationalHealth) {
    let level = match sample.state {
        HealthState::Healthy => Level::Info,
        HealthState::Degraded => Level::Warn,
        HealthState::Failed => Level::Error,
    };
    emit_audit(
        TraceEvent::new(level, EventKind::Audit)
            .at(sample.sampled_at_us)
            .on_node(sample.node)
            .with_field(EventField::unsigned(field::TRANSPORT, sample.transport as u64))
            .with_field(EventField::unsigned(field::HEALTH, sample.state as u64))
            .with_field(EventField::unsigned(field::QUEUE_DEPTH, sample.queue_depth as u64))
            .with_field(EventField::unsigned(field::QUEUE_CAPACITY, sample.queue_capacity as u64)),
    );
    emit_audit(
        TraceEvent::new(level, EventKind::Audit)
            .at(sample.sampled_at_us)
            .on_node(sample.node)
            .with_field(EventField::unsigned(field::TRANSPORT, sample.transport as u64))
            .with_field(EventField::unsigned(field::DROPPED_PACKETS, sample.dropped_packets))
            .with_field(EventField::unsigned(field::RETRIES, sample.retries))
            .with_field(EventField::boolean(field::DEGRADED_MODE, sample.degraded_mode)),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct CorrelationId(u128);

impl CorrelationId {
    pub const NONE: Self = Self(0);

    pub const fn from_raw(raw: u128) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u128 {
        self.0
    }

    pub const fn is_none(self) -> bool {
        self.0 == 0
    }
}

static CORRELATION_BOOT: AtomicU32 = AtomicU32::new(1);
static CORRELATION_COUNTER: AtomicU64 = AtomicU64::new(1);

pub fn set_correlation_boot_id(boot_id: u32) {
    CORRELATION_BOOT.store(boot_id.max(1), Ordering::Release)
}

pub fn next_correlation_id(node: u32) -> CorrelationId {
    let counter = CORRELATION_COUNTER.fetch_add(1, Ordering::Relaxed).max(1);
    CorrelationId(
        ((node.max(1) as u128) << 96)
            | ((CORRELATION_BOOT.load(Ordering::Acquire) as u128) << 64)
            | counter as u128,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u8)]
pub enum Level {
    Trace = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
    Critical = 4,
}

impl Level {
    const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Trace),
            1 => Some(Self::Info),
            2 => Some(Self::Warn),
            3 => Some(Self::Error),
            4 => Some(Self::Critical),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EventKind {
    Boot = 1,
    Kernel = 2,
    Ipc = 3,
    RemoteMemory = 4,
    Authentication = 5,
    Capability = 6,
    Audit = 7,
    Operator = 8,
    Thermal = 9,
}

impl EventKind {
    const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Boot),
            2 => Some(Self::Kernel),
            3 => Some(Self::Ipc),
            4 => Some(Self::RemoteMemory),
            5 => Some(Self::Authentication),
            6 => Some(Self::Capability),
            7 => Some(Self::Audit),
            8 => Some(Self::Operator),
            9 => Some(Self::Thermal),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FieldKind {
    Unsigned = 1,
    Signed = 2,
    Boolean = 3,
    Identifier = 4,
    Status = 5,
}

impl FieldKind {
    const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Unsigned),
            2 => Some(Self::Signed),
            3 => Some(Self::Boolean),
            4 => Some(Self::Identifier),
            5 => Some(Self::Status),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventField {
    pub key: u16,
    pub kind: FieldKind,
    value: u128,
}

impl EventField {
    pub const fn unsigned(key: u16, value: u64) -> Self {
        Self {
            key,
            kind: FieldKind::Unsigned,
            value: value as u128,
        }
    }

    pub const fn signed(key: u16, value: i64) -> Self {
        Self {
            key,
            kind: FieldKind::Signed,
            value: value as u64 as u128,
        }
    }

    pub const fn boolean(key: u16, value: bool) -> Self {
        Self {
            key,
            kind: FieldKind::Boolean,
            value: value as u128,
        }
    }

    pub const fn identifier(key: u16, value: u128) -> Self {
        Self {
            key,
            kind: FieldKind::Identifier,
            value,
        }
    }

    pub const fn status(value: Status) -> Self {
        Self {
            key: field::STATUS,
            kind: FieldKind::Status,
            value: value.raw() as u128,
        }
    }

    pub const fn as_u128(self) -> u128 {
        self.value
    }

    pub const fn as_u64(self) -> u64 {
        self.value as u64
    }
}

const EMPTY_FIELD: EventField = EventField::unsigned(0, 0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceEvent {
    pub timestamp: u64,
    pub correlation: CorrelationId,
    pub node: u32,
    pub level: Level,
    pub kind: EventKind,
    fields: [EventField; MAX_EVENT_FIELDS],
    field_count: u8,
}

impl TraceEvent {
    pub const fn new(level: Level, kind: EventKind) -> Self {
        Self {
            timestamp: 0,
            correlation: CorrelationId::NONE,
            node: 1,
            level,
            kind,
            fields: [EMPTY_FIELD; MAX_EVENT_FIELDS],
            field_count: 0,
        }
    }

    pub const fn at(mut self, timestamp: u64) -> Self {
        self.timestamp = timestamp;
        self
    }

    pub const fn on_node(mut self, node: u32) -> Self {
        self.node = if node == 0 { 1 } else { node };
        self
    }

    pub const fn correlated(mut self, correlation: CorrelationId) -> Self {
        self.correlation = correlation;
        self
    }

    pub const fn with_field(mut self, field: EventField) -> Self {
        if (self.field_count as usize) < MAX_EVENT_FIELDS {
            self.fields[self.field_count as usize] = field;
            self.field_count += 1
        }
        self
    }

    pub fn fields(&self) -> impl Iterator<Item = EventField> + '_ {
        self.fields[..self.field_count as usize].iter().copied()
    }

    pub fn field(&self, key: u16) -> Option<EventField> {
        self.fields().find(|entry| entry.key == key)
    }
}

/// Common, bounded vocabulary for following one capability across trust
/// boundaries. The capability value is opaque; only the handle/token value is
/// carried so audit records never contain resource names or payload bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityTrace {
    pub domain: CapabilityDomain,
    pub stage: CapabilityTraceStage,
    pub capability: u64,
    pub operation: u16,
}

impl CapabilityTrace {
    pub const fn new(
        domain: CapabilityDomain,
        stage: CapabilityTraceStage,
        capability: u64,
        operation: u16,
    ) -> Option<Self> {
        if capability == 0 {
            None
        } else {
            Some(Self {
                domain,
                stage,
                capability,
                operation,
            })
        }
    }

    pub fn from_event(event: TraceEvent) -> Option<Self> {
        if event.kind != EventKind::Audit {
            return None;
        }
        Some(Self {
            domain: CapabilityDomain::from_raw(
                event.field(field::CAPABILITY_DOMAIN)?.as_u64() as u8,
            )?,
            stage: CapabilityTraceStage::from_raw(
                event.field(field::CAPABILITY_STAGE)?.as_u64() as u8,
            )?,
            capability: event.field(field::CAPABILITY)?.as_u64(),
            operation: event.field(field::OPERATION)?.as_u64() as u16,
        })
    }

    pub fn event(self, level: Level) -> TraceEvent {
        TraceEvent::new(level, EventKind::Audit)
            .with_field(EventField::unsigned(field::CAPABILITY, self.capability))
            .with_field(EventField::unsigned(
                field::CAPABILITY_DOMAIN,
                self.domain as u64,
            ))
            .with_field(EventField::unsigned(
                field::CAPABILITY_STAGE,
                self.stage as u64,
            ))
            .with_field(EventField::unsigned(field::OPERATION, self.operation as u64))
    }

    pub fn emit(self, level: Level) {
        emit_capability_trace(
            level,
            self.domain,
            self.stage,
            self.capability,
            self.operation,
        )
    }
}

#[inline(never)]
pub fn emit_capability_trace(
    level: Level,
    domain: CapabilityDomain,
    stage: CapabilityTraceStage,
    capability: u64,
    operation: u16,
) {
    if let Some(trace) = CapabilityTrace::new(domain, stage, capability, operation) {
        emit_audit(trace.event(level))
    }
}

struct AtomicField {
    metadata: AtomicU32,
    low: AtomicU64,
    high: AtomicU64,
}

impl AtomicField {
    const fn new() -> Self {
        Self {
            metadata: AtomicU32::new(0),
            low: AtomicU64::new(0),
            high: AtomicU64::new(0),
        }
    }

    fn write(&self, field: EventField) {
        self.low.store(field.value as u64, Ordering::Relaxed);
        self.high
            .store((field.value >> 64) as u64, Ordering::Relaxed);
        self.metadata.store(
            field.key as u32 | ((field.kind as u32) << 16),
            Ordering::Relaxed,
        )
    }

    fn read(&self) -> EventField {
        let metadata = self.metadata.load(Ordering::Relaxed);
        EventField {
            key: metadata as u16,
            kind: FieldKind::from_raw((metadata >> 16) as u8).unwrap_or(FieldKind::Unsigned),
            value: self.low.load(Ordering::Relaxed) as u128
                | ((self.high.load(Ordering::Relaxed) as u128) << 64),
        }
    }
}

struct AtomicEvent {
    timestamp: AtomicU64,
    correlation_low: AtomicU64,
    correlation_high: AtomicU64,
    node: AtomicU32,
    level: AtomicU8,
    kind: AtomicU8,
    field_count: AtomicU8,
    fields: [AtomicField; MAX_EVENT_FIELDS],
}

impl AtomicEvent {
    const fn new() -> Self {
        Self {
            timestamp: AtomicU64::new(0),
            correlation_low: AtomicU64::new(0),
            correlation_high: AtomicU64::new(0),
            // An event is not readable until `published` is set after every
            // field is written. Zero is therefore a valid BSS initializer.
            node: AtomicU32::new(0),
            level: AtomicU8::new(0),
            kind: AtomicU8::new(0),
            field_count: AtomicU8::new(0),
            fields: [const { AtomicField::new() }; MAX_EVENT_FIELDS],
        }
    }

    fn write(&self, event: TraceEvent) {
        self.timestamp.store(event.timestamp, Ordering::Relaxed);
        self.correlation_low
            .store(event.correlation.raw() as u64, Ordering::Relaxed);
        self.correlation_high
            .store((event.correlation.raw() >> 64) as u64, Ordering::Relaxed);
        self.node.store(event.node, Ordering::Relaxed);
        self.level.store(event.level as u8, Ordering::Relaxed);
        self.kind.store(event.kind as u8, Ordering::Relaxed);
        for (destination, source) in self.fields.iter().zip(event.fields) {
            destination.write(source)
        }
        self.field_count.store(event.field_count, Ordering::Relaxed)
    }

    fn read(&self) -> TraceEvent {
        let mut fields = [EMPTY_FIELD; MAX_EVENT_FIELDS];
        for (destination, source) in fields.iter_mut().zip(&self.fields) {
            *destination = source.read()
        }
        TraceEvent {
            timestamp: self.timestamp.load(Ordering::Relaxed),
            correlation: CorrelationId::from_raw(
                self.correlation_low.load(Ordering::Relaxed) as u128
                    | ((self.correlation_high.load(Ordering::Relaxed) as u128) << 64),
            ),
            node: self.node.load(Ordering::Relaxed),
            level: Level::from_raw(self.level.load(Ordering::Relaxed)).unwrap_or(Level::Trace),
            kind: EventKind::from_raw(self.kind.load(Ordering::Relaxed))
                .unwrap_or(EventKind::Kernel),
            fields,
            field_count: self
                .field_count
                .load(Ordering::Relaxed)
                .min(MAX_EVENT_FIELDS as u8),
        }
    }
}

struct TraceSlot {
    published: AtomicU64,
    event: AtomicEvent,
}

impl TraceSlot {
    const fn new() -> Self {
        Self {
            // Zero is also the empty marker for a newly-created ring. A
            // reader only examines slots below `read_position`, so no
            // uninitialized slot can be mistaken for a published event.
            // Keeping this zero-valued lets global rings live in BSS.
            published: AtomicU64::new(0),
            event: AtomicEvent::new(),
        }
    }
}

/// Lock-free, fixed-size MPMC trace queue. Producers overwrite the oldest
/// record when consumers fall behind, so kernel tracing never blocks.
pub struct TraceRing<const CAPACITY: usize> {
    write_position: AtomicU64,
    read_position: AtomicU64,
    dropped: AtomicU64,
    labels: cardinality::AtomicCardinality<MAX_TRACE_LABEL_CARDINALITY>,
    slots: [TraceSlot; CAPACITY],
}

impl<const CAPACITY: usize> TraceRing<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY >= 2);
        Self {
            write_position: AtomicU64::new(0),
            read_position: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            labels: cardinality::AtomicCardinality::new(),
            slots: [const { TraceSlot::new() }; CAPACITY],
        }
    }

    pub fn push(&self, event: TraceEvent) {
        self.labels.observe_event(event);
        let ticket = self.write_position.fetch_add(1, Ordering::AcqRel);
        let minimum = ticket.saturating_add(1).saturating_sub(CAPACITY as u64);
        let mut read = self.read_position.load(Ordering::Acquire);
        while read < minimum {
            match self.read_position.compare_exchange_weak(
                read,
                minimum,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.dropped.fetch_add(minimum - read, Ordering::Relaxed);
                    break;
                }
                Err(observed) => read = observed,
            }
        }

        let slot = &self.slots[ticket as usize % CAPACITY];
        slot.published.store(ticket | SLOT_BUSY, Ordering::Release);
        slot.event.write(event);
        slot.published.store(ticket, Ordering::Release)
    }

    pub fn try_pop(&self) -> Option<TraceEvent> {
        loop {
            let position = self.read_position.load(Ordering::Acquire);
            if position >= self.write_position.load(Ordering::Acquire) {
                return None;
            }
            let slot = &self.slots[position as usize % CAPACITY];
            let published = slot.published.load(Ordering::Acquire);
            if published & SLOT_BUSY != 0 {
                return None;
            }
            if published != position {
                if published > position {
                    let _ = self.read_position.compare_exchange_weak(
                        position,
                        published,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    continue;
                }
                return None;
            }
            if self
                .read_position
                .compare_exchange_weak(position, position + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                continue;
            }
            let event = slot.event.read();
            if slot.published.load(Ordering::Acquire) == position {
                return Some(event);
            }
            let _ = self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn pending(&self) -> usize {
        self.write_position
            .load(Ordering::Acquire)
            .saturating_sub(self.read_position.load(Ordering::Acquire))
            .min(CAPACITY as u64) as usize
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Acquire)
    }

    /// Cardinality is measured independently from ring occupancy. A full
    /// ring drops records; a full label table folds new label values here.
    pub fn label_cardinality(&self) -> CardinalitySnapshot {
        self.labels.snapshot()
    }

    pub fn export_label_aggregates(
        &self,
        destination: &mut [Option<CardinalityEntry>],
    ) -> usize {
        self.labels.export(destination)
    }

    /// Copy the newest records without consuming the ring. This is used by
    /// crash capture, which must not depend on the log service draining it.
    pub fn copy_recent(&self, destination: &mut [Option<TraceEvent>]) -> usize {
        if destination.is_empty() {
            return 0
        }
        let end = self.write_position.load(Ordering::Acquire);
        let start = end.saturating_sub(destination.len().min(CAPACITY) as u64);
        let mut count = 0;
        for position in start..end {
            if count == destination.len() {
                break
            }
            let slot = &self.slots[position as usize % CAPACITY];
            if slot.published.load(Ordering::Acquire) != position {
                continue
            }
            let event = slot.event.read();
            if slot.published.load(Ordering::Acquire) != position {
                continue
            }
            destination[count] = Some(event);
            count += 1
        }
        count
    }
}

impl<const CAPACITY: usize> Default for TraceRing<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

pub static SYSTEM_TRACE: TraceRing<GLOBAL_TRACE_CAPACITY> = TraceRing::new();
pub static SECURITY_AUDIT: TraceRing<GLOBAL_AUDIT_CAPACITY> = TraceRing::new();
static LOGICAL_CLOCK: AtomicU64 = AtomicU64::new(1);

pub fn emit(mut event: TraceEvent) {
    if event.timestamp == 0 {
        event.timestamp = LOGICAL_CLOCK.fetch_add(1, Ordering::Relaxed)
    }
    if event.correlation.is_none() {
        event.correlation = next_correlation_id(event.node)
    }
    SYSTEM_TRACE.push(event)
}

pub fn emit_audit(mut event: TraceEvent) {
    event.kind = EventKind::Audit;
    if event.timestamp == 0 {
        event.timestamp = LOGICAL_CLOCK.fetch_add(1, Ordering::Relaxed)
    }
    if event.correlation.is_none() {
        event.correlation = next_correlation_id(event.node)
    }
    SECURITY_AUDIT.push(event);
    SYSTEM_TRACE.push(event)
}

#[macro_export]
macro_rules! trace {
    ($kind:expr $(, $field:expr)* $(,)?) => {{
        let event = $crate::TraceEvent::new($crate::Level::Trace, $kind)
            $(.with_field($field))*;
        $crate::emit(event)
    }};
}

#[macro_export]
macro_rules! info {
    ($kind:expr $(, $field:expr)* $(,)?) => {{
        let event = $crate::TraceEvent::new($crate::Level::Info, $kind)
            $(.with_field($field))*;
        $crate::emit(event)
    }};
}

#[macro_export]
macro_rules! warn {
    ($kind:expr $(, $field:expr)* $(,)?) => {{
        let event = $crate::TraceEvent::new($crate::Level::Warn, $kind)
            $(.with_field($field))*;
        $crate::emit(event)
    }};
}

#[macro_export]
macro_rules! error {
    ($kind:expr $(, $field:expr)* $(,)?) => {{
        let event = $crate::TraceEvent::new($crate::Level::Error, $kind)
            $(.with_field($field))*;
        $crate::emit(event)
    }};
}

#[macro_export]
macro_rules! audit_event {
    ($level:expr $(, $field:expr)* $(,)?) => {{
        let event = $crate::TraceEvent::new($level, $crate::EventKind::Audit)
            $(.with_field($field))*;
        $crate::emit_audit(event)
    }};
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    BufferTooSmall,
    InvalidField,
    InvalidMagic,
    InvalidVersion,
    Checksum,
}

pub fn encode_record(event: TraceEvent, destination: &mut [u8]) -> Result<usize, CodecError> {
    if destination.len() < JOURNAL_RECORD_SIZE {
        return Err(CodecError::BufferTooSmall);
    }
    destination[..JOURNAL_RECORD_SIZE].fill(0);
    destination[..4].copy_from_slice(&JOURNAL_MAGIC.to_le_bytes());
    destination[4] = JOURNAL_VERSION;
    destination[5] = event.level as u8;
    destination[6] = event.kind as u8;
    destination[7] = event.field_count;
    destination[8..16].copy_from_slice(&event.timestamp.to_le_bytes());
    destination[16..32].copy_from_slice(&event.correlation.raw().to_le_bytes());
    destination[32..36].copy_from_slice(&event.node.to_le_bytes());
    for (index, value) in event.fields().enumerate() {
        let offset = 40 + index * 20;
        destination[offset..offset + 2].copy_from_slice(&value.key.to_le_bytes());
        destination[offset + 2] = value.kind as u8;
        destination[offset + 4..offset + 20].copy_from_slice(&value.value.to_le_bytes())
    }
    let checksum = checksum(&destination[..120]);
    destination[120..128].copy_from_slice(&checksum.to_le_bytes());
    Ok(JOURNAL_RECORD_SIZE)
}

pub fn decode_record(source: &[u8]) -> Result<TraceEvent, CodecError> {
    if source.len() < JOURNAL_RECORD_SIZE {
        return Err(CodecError::BufferTooSmall);
    }
    if u32::from_le_bytes(record_bytes(source, 0)?) != JOURNAL_MAGIC {
        return Err(CodecError::InvalidMagic);
    }
    if source[4] != JOURNAL_VERSION {
        return Err(CodecError::InvalidVersion);
    }
    if u64::from_le_bytes(record_bytes(source, 120)?)
        != checksum(&source[..120])
    {
        return Err(CodecError::Checksum);
    }
    let level = Level::from_raw(source[5]).ok_or(CodecError::InvalidField)?;
    let kind = EventKind::from_raw(source[6]).ok_or(CodecError::InvalidField)?;
    let field_count = source[7] as usize;
    if field_count > MAX_EVENT_FIELDS {
        return Err(CodecError::InvalidField);
    }
    let mut event = TraceEvent::new(level, kind)
        .at(u64::from_le_bytes(record_bytes(source, 8)?))
        .correlated(CorrelationId::from_raw(u128::from_le_bytes(
            record_bytes(source, 16)?,
        )))
        .on_node(u32::from_le_bytes(record_bytes(source, 32)?));
    for index in 0..field_count {
        let offset = 40 + index * 20;
        let key = u16::from_le_bytes(record_bytes(source, offset)?);
        let kind = FieldKind::from_raw(source[offset + 2]).ok_or(CodecError::InvalidField)?;
        let value = u128::from_le_bytes(record_bytes(source, offset + 4)?);
        event = event.with_field(EventField { key, kind, value })
    }
    Ok(event)
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AuditKey([u8; 32]);

impl AuditKey {
    pub const fn new(bytes: [u8; 32]) -> Option<Self> {
        let mut nonzero = false;
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != 0 {
                nonzero = true;
                break
            }
            index += 1;
        }
        if nonzero { Some(Self(bytes)) } else { None }
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn id(self) -> [u8; 32] {
        *ContentId::hash(&self.0).as_bytes()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SealedAuditRecord {
    pub sequence: u64,
    pub event: TraceEvent,
    pub previous: [u8; 32],
    pub tag: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditJournalError {
    Capacity,
    InvalidKey,
    InvalidEvent,
    BufferTooSmall { required: usize },
    Corrupt,
    KeyMismatch,
    Tampered,
}

impl AuditJournalError {
    pub const fn status(self) -> Status {
        match self {
            Self::Capacity | Self::BufferTooSmall { .. } => Status::NO_SPACE,
            Self::InvalidKey | Self::InvalidEvent => Status::INVALID_ARGUMENT,
            Self::Corrupt | Self::KeyMismatch | Self::Tampered => Status::CORRUPT,
        }
    }
}

/// Fixed-size, append-only audit evidence for recovery mode.
///
/// The normal log ring may drop old records. This journal never overwrites a
/// record. Once full, it reports capacity so the caller can raise a durable
/// audit alarm instead of silently losing security evidence.
#[derive(Clone, Copy)]
pub struct AuditJournal<const CAPACITY: usize = DEFAULT_RECOVERY_AUDIT_CAPACITY> {
    key: AuditKey,
    records: [Option<SealedAuditRecord>; CAPACITY],
    labels: AuditLabelAggregator<MAX_AUDIT_LABEL_CARDINALITY>,
    next_sequence: u64,
    dropped: u64,
}

impl<const CAPACITY: usize> AuditJournal<CAPACITY> {
    pub const fn encoded_len() -> usize {
        AUDIT_EXPORT_HEADER_BYTES + CAPACITY * SEALED_AUDIT_RECORD_BYTES
    }

    pub fn new(key: AuditKey) -> Result<Self, AuditJournalError> {
        if key.0.iter().all(|byte| *byte == 0) {
            return Err(AuditJournalError::InvalidKey)
        }
        Ok(Self {
            key,
            records: [None; CAPACITY],
            labels: AuditLabelAggregator::new(),
            next_sequence: 1,
            dropped: 0,
        })
    }

    pub fn key_id(&self) -> [u8; 32] {
        self.key.id()
    }

    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn records(&self) -> impl Iterator<Item = SealedAuditRecord> + '_ {
        self.records.iter().flatten().copied()
    }

    pub fn events(&self) -> impl Iterator<Item = TraceEvent> + '_ {
        self.records().map(|record| record.event)
    }

    pub fn label_aggregates(&self) -> impl Iterator<Item = AuditLabelAggregate> + '_ {
        self.labels.aggregates()
    }

    pub const fn label_cardinality(&self) -> CardinalitySnapshot {
        self.labels.snapshot()
    }

    pub fn append(&mut self, event: TraceEvent) -> Result<u64, AuditJournalError> {
        if event.kind != EventKind::Audit || event.timestamp == 0 {
            return Err(AuditJournalError::InvalidEvent)
        }
        self.labels.observe(event);
        let previous = self
            .records
            .iter()
            .flatten()
            .last()
            .map(|record| record.tag)
            .unwrap_or([0; 32]);
        let Some(slot) = self.records.iter_mut().find(|entry| entry.is_none()) else {
            self.dropped = self.dropped.saturating_add(1);
            return Err(AuditJournalError::Capacity)
        };
        let sequence = self.next_sequence;
        let record = SealedAuditRecord {
            sequence,
            event,
            previous,
            tag: audit_tag(self.key, sequence, event, previous),
        };
        *slot = Some(record);
        self.next_sequence = self.next_sequence.saturating_add(1);
        Ok(sequence)
    }

    pub fn verify(&self) -> Result<(), AuditJournalError> {
        let mut previous = [0; 32];
        let mut expected = 1_u64;
        for record in self.records.iter().flatten() {
            if record.sequence != expected
                || record.previous != previous
                || record.event.kind != EventKind::Audit
                || record.event.timestamp == 0
                || record.tag != audit_tag(self.key, record.sequence, record.event, record.previous)
            {
                return Err(AuditJournalError::Tampered)
            }
            previous = record.tag;
            expected = expected.saturating_add(1);
        }
        if self.next_sequence != expected {
            return Err(AuditJournalError::Corrupt)
        }
        Ok(())
    }

    pub fn export(&self, destination: &mut [u8]) -> Result<usize, AuditJournalError> {
        self.verify()?;
        let required = Self::encoded_len();
        if destination.len() < required {
            return Err(AuditJournalError::BufferTooSmall { required })
        }
        destination[..required].fill(0);
        destination[..8].copy_from_slice(AUDIT_EXPORT_MAGIC);
        destination[8..10].copy_from_slice(&AUDIT_EXPORT_VERSION.to_le_bytes());
        destination[10..12].copy_from_slice(&(AUDIT_EXPORT_HEADER_BYTES as u16).to_le_bytes());
        destination[12..16].copy_from_slice(&(self.records().count() as u32).to_le_bytes());
        destination[16..24].copy_from_slice(&self.next_sequence.to_le_bytes());
        destination[24..32].copy_from_slice(&self.dropped.to_le_bytes());
        destination[32..64].copy_from_slice(&self.key_id());
        for (index, record) in self.records().enumerate() {
            let offset = AUDIT_EXPORT_HEADER_BYTES + index * SEALED_AUDIT_RECORD_BYTES;
            destination[offset..offset + 8].copy_from_slice(&record.sequence.to_le_bytes());
            encode_record(
                record.event,
                &mut destination[offset + 8..offset + 8 + JOURNAL_RECORD_SIZE],
            ).map_err(|_| AuditJournalError::Corrupt)?;
            destination[offset + 136..offset + 168].copy_from_slice(&record.previous);
            destination[offset + 168..offset + 200].copy_from_slice(&record.tag);
        }
        Ok(required)
    }

    pub fn decode(input: &[u8], key: AuditKey) -> Result<Self, AuditJournalError> {
        let required = Self::encoded_len();
        if input.len() != required
            || &input[..8] != AUDIT_EXPORT_MAGIC
            || u16::from_le_bytes([input[8], input[9]]) != AUDIT_EXPORT_VERSION
            || u16::from_le_bytes([input[10], input[11]]) != AUDIT_EXPORT_HEADER_BYTES as u16
        {
            return Err(AuditJournalError::Corrupt)
        }
        if input[32..64] != key.id() {
            return Err(AuditJournalError::KeyMismatch)
        }
        let count = u32::from_le_bytes(input[12..16].try_into().map_err(|_| AuditJournalError::Corrupt)?) as usize;
        if count > CAPACITY {
            return Err(AuditJournalError::Corrupt)
        }
        let next_sequence = u64::from_le_bytes(input[16..24].try_into().map_err(|_| AuditJournalError::Corrupt)?);
        if next_sequence != (count as u64).saturating_add(1) {
            return Err(AuditJournalError::Corrupt)
        }
        let mut journal = Self::new(key)?;
        journal.next_sequence = next_sequence;
        journal.dropped = u64::from_le_bytes(input[24..32].try_into().map_err(|_| AuditJournalError::Corrupt)?);
        for index in 0..count {
            let offset = AUDIT_EXPORT_HEADER_BYTES + index * SEALED_AUDIT_RECORD_BYTES;
            let event = decode_record(&input[offset + 8..offset + 136])
                .map_err(|_| AuditJournalError::Corrupt)?;
            let mut previous = [0; 32];
            previous.copy_from_slice(&input[offset + 136..offset + 168]);
            let mut tag = [0; 32];
            tag.copy_from_slice(&input[offset + 168..offset + 200]);
            journal.records[index] = Some(SealedAuditRecord {
                sequence: u64::from_le_bytes(
                    input[offset..offset + 8]
                        .try_into()
                        .map_err(|_| AuditJournalError::Corrupt)?,
                ),
                event,
                previous,
                tag,
            });
            journal.labels.observe(event);
        }
        if input[AUDIT_EXPORT_HEADER_BYTES + count * SEALED_AUDIT_RECORD_BYTES..]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(AuditJournalError::Corrupt)
        }
        journal.verify()?;
        Ok(journal)
    }

    pub fn recover(input: &[u8], key: AuditKey) -> Result<Self, AuditJournalError> {
        Self::decode(input, key)
    }

    pub fn replay(
        &self,
        query: AuditQuery,
        mut visitor: impl FnMut(TraceEvent),
    ) -> Result<usize, AuditJournalError> {
        self.verify()?;
        let mut count = 0;
        for event in self.events().filter(|event| query.matches(*event)) {
            visitor(event);
            count += 1;
        }
        Ok(count)
    }
}

fn audit_tag(key: AuditKey, sequence: u64, event: TraceEvent, previous: [u8; 32]) -> [u8; 32] {
    let mut material = [0; 32 + 8 + JOURNAL_RECORD_SIZE + 32];
    material[..32].copy_from_slice(&key.0);
    material[32..40].copy_from_slice(&sequence.to_le_bytes());
    encode_record(event, &mut material[40..40 + JOURNAL_RECORD_SIZE])
        .expect("fixed audit record buffer");
    material[40 + JOURNAL_RECORD_SIZE..].copy_from_slice(&previous);
    *ContentId::hash(&material).as_bytes()
}

fn record_bytes<const SIZE: usize>(source: &[u8], start: usize) -> Result<[u8; SIZE], CodecError> {
    source
        .get(start..start.checked_add(SIZE).ok_or(CodecError::BufferTooSmall)?)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(CodecError::BufferTooSmall)
}

const fn checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        index += 1
    }
    hash
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct AuditQuery {
    pub since: Option<u64>,
    pub before: Option<u64>,
    pub capability: Option<u64>,
    pub node: Option<u32>,
    pub status: Option<u32>,
}

impl AuditQuery {
    pub fn matches(self, event: TraceEvent) -> bool {
        if event.kind != EventKind::Audit {
            return false;
        }
        if self.since.is_some_and(|value| event.timestamp < value)
            || self.before.is_some_and(|value| event.timestamp >= value)
            || self.node.is_some_and(|value| event.node != value)
        {
            return false;
        }
        if self.capability.is_some_and(|value| {
            event
                .field(field::CAPABILITY)
                .is_none_or(|field| field.as_u64() != value)
        }) {
            return false;
        }
        if self.status.is_some_and(|value| {
            event
                .field(field::STATUS)
                .is_none_or(|field| field.as_u64() as u32 != value)
        }) {
            return false;
        }
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryError {
    Codec(CodecError),
    InvalidArgument,
    TrailingBytes,
}

pub fn analyze_audit(
    journal: &[u8],
    query: AuditQuery,
    mut visitor: impl FnMut(TraceEvent),
) -> Result<usize, QueryError> {
    let mut chunks = journal.chunks_exact(JOURNAL_RECORD_SIZE);
    let mut matched = 0;
    for record in &mut chunks {
        let event = decode_record(record).map_err(QueryError::Codec)?;
        if query.matches(event) {
            visitor(event);
            matched += 1
        }
    }
    if !chunks.remainder().is_empty() {
        return Err(QueryError::TrailingBytes);
    }
    Ok(matched)
}

/// Parser for `analyze/audit /since=N /before=N /capability=N /node=N /status=N`.
pub fn parse_audit_command<'a>(
    arguments: impl IntoIterator<Item = &'a str>,
) -> Result<AuditQuery, QueryError> {
    let mut query = AuditQuery::default();
    for argument in arguments {
        let raw = argument
            .strip_prefix('/')
            .or_else(|| argument.strip_prefix("--"))
            .ok_or(QueryError::InvalidArgument)?;
        let (name, raw_value) = raw.split_once('=').ok_or(QueryError::InvalidArgument)?;
        match name {
            _ if name.eq_ignore_ascii_case("since") => query.since = Some(parse_u64(raw_value)?),
            _ if name.eq_ignore_ascii_case("before") => query.before = Some(parse_u64(raw_value)?),
            _ if name.eq_ignore_ascii_case("capability") => {
                query.capability = Some(parse_u64(raw_value)?)
            }
            _ if name.eq_ignore_ascii_case("node") => {
                query.node = Some(
                    u32::try_from(parse_u64(raw_value)?)
                        .map_err(|_| QueryError::InvalidArgument)?,
                )
            }
            _ if name.eq_ignore_ascii_case("status") => {
                query.status = Some(
                    u32::try_from(parse_u64(raw_value)?)
                        .map_err(|_| QueryError::InvalidArgument)?,
                )
            }
            _ => return Err(QueryError::InvalidArgument),
        }
    }
    Ok(query)
}

fn parse_u64(value: &str) -> Result<u64, QueryError> {
    if let Some(hex) = value.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).map_err(|_| QueryError::InvalidArgument)
    } else {
        value
            .parse::<u64>()
            .map_err(|_| QueryError::InvalidArgument)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MetricKind {
    Counter = 1,
    Gauge = 2,
    Histogram = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricSample {
    pub name: u16,
    pub kind: MetricKind,
    pub timestamp: u64,
    pub value: u64,
    pub dimensions: TelemetryDimensions,
}

impl MetricSample {
    pub const fn new(
        name: u16,
        kind: MetricKind,
        timestamp: u64,
        value: u64,
        dimensions: TelemetryDimensions,
    ) -> Self {
        Self { name, kind, timestamp, value, dimensions }
    }

    fn same_series(self, other: Self) -> bool {
        self.name == other.name
            && self.kind == other.kind
            && self.dimensions == other.dimensions
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricError {
    Capacity,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricRecordOutcome {
    NewSeries,
    UpdatedSeries,
    Aggregated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricAggregate {
    pub name: u16,
    pub kind: MetricKind,
    pub dimensions: TelemetryDimensions,
    pub observations: u64,
    pub value: u64,
}

/// Bounded metric series store. Counters are accumulated; gauges and histogram
/// observations replace the last value for the same dimensions.
pub struct MetricRegistry<const CAPACITY: usize = MAX_METRIC_SAMPLES> {
    samples: [Option<MetricSample>; CAPACITY],
    aggregates: [Option<MetricAggregate>; MAX_METRIC_AGGREGATES],
    cardinality: CardinalityTable<MAX_METRIC_CARDINALITY>,
    dropped: u64,
}

impl<const CAPACITY: usize> MetricRegistry<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY >= 1);
        Self {
            samples: [None; CAPACITY],
            aggregates: [None; MAX_METRIC_AGGREGATES],
            cardinality: CardinalityTable::new(),
            dropped: 0,
        }
    }

    pub fn record(&mut self, sample: MetricSample) -> Result<(), MetricError> {
        if sample.name == 0 || sample.timestamp == 0 || sample.dimensions.node == 0 {
            return Err(MetricError::Invalid);
        }
        self.cardinality.observe(sample.name as u64, metric_series_value(sample));
        if let Some(existing) = self.samples.iter_mut().flatten().find(|entry| entry.same_series(sample)) {
            if sample.kind == MetricKind::Counter {
                existing.value = existing.value.saturating_add(sample.value);
            } else {
                *existing = sample;
            }
            existing.timestamp = sample.timestamp;
            return Ok(())
        }
        let slot = self.samples.iter_mut().find(|entry| entry.is_none());
        if let Some(slot) = slot {
            *slot = Some(sample);
            Ok(())
        } else {
            self.dropped = self.dropped.saturating_add(1);
            Err(MetricError::Capacity)
        }
    }

    /// Record a sample without letting a new series make ingestion fail.
    /// Once the exact series budget is full, samples are folded into a stable
    /// `other` dimension and the fold count remains inspectable.
    pub fn record_bounded(
        &mut self,
        sample: MetricSample,
    ) -> Result<MetricRecordOutcome, MetricError> {
        if sample.name == 0 || sample.timestamp == 0 || sample.dimensions.node == 0 {
            return Err(MetricError::Invalid)
        }
        let cardinality = self
            .cardinality
            .observe(sample.name as u64, metric_series_value(sample));
        if let Some(existing) = self
            .samples
            .iter_mut()
            .flatten()
            .find(|entry| entry.same_series(sample))
        {
            if sample.kind == MetricKind::Counter {
                existing.value = existing.value.saturating_add(sample.value);
            } else {
                *existing = sample;
            }
            existing.timestamp = sample.timestamp;
            return Ok(MetricRecordOutcome::UpdatedSeries)
        }
        if cardinality == CardinalityDecision::Aggregated {
            return Ok(self.aggregate_metric(sample))
        }
        if let Some(slot) = self.samples.iter_mut().find(|entry| entry.is_none()) {
            *slot = Some(sample);
            return Ok(MetricRecordOutcome::NewSeries)
        }

        Ok(self.aggregate_metric(sample))
    }

    fn aggregate_metric(&mut self, sample: MetricSample) -> MetricRecordOutcome {
        let dimensions = TelemetryDimensions::other();
        if let Some(existing) = self
            .aggregates
            .iter_mut()
            .flatten()
            .find(|entry| entry.name == sample.name && entry.kind == sample.kind)
        {
            existing.observations = existing.observations.saturating_add(1);
            existing.value = aggregate_metric_value(existing.kind, existing.value, sample.value);
            return MetricRecordOutcome::Aggregated
        }
        if let Some(slot) = self.aggregates.iter_mut().find(|entry| entry.is_none()) {
            *slot = Some(MetricAggregate {
                name: sample.name,
                kind: sample.kind,
                dimensions,
                observations: 1,
                value: sample.value,
            });
            return MetricRecordOutcome::Aggregated
        }
        self.dropped = self.dropped.saturating_add(1);
        MetricRecordOutcome::Aggregated
    }

    pub fn samples(&self) -> impl Iterator<Item = MetricSample> + '_ {
        self.samples.iter().flatten().copied()
    }

    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn aggregates(&self) -> impl Iterator<Item = MetricAggregate> + '_ {
        self.aggregates.iter().flatten().copied()
    }

    pub const fn cardinality(&self) -> CardinalitySnapshot {
        self.cardinality.snapshot()
    }

    pub fn export(&self, destination: &mut [Option<MetricSample>]) -> usize {
        let mut count = 0;
        for sample in self.samples() {
            if let Some(slot) = destination.get_mut(count) {
                *slot = Some(sample);
                count += 1;
            } else {
                break
            }
        }
        count
    }
}

fn metric_series_value(sample: MetricSample) -> u128 {
    sample.dimensions.cluster
        ^ ((sample.dimensions.node as u128) << 32)
        ^ ((sample.dimensions.transport as u128) << 64)
        ^ ((sample.dimensions.workload as u128) << 72)
        ^ ((sample.dimensions.operation as u128) << 16)
}

fn aggregate_metric_value(kind: MetricKind, current: u64, value: u64) -> u64 {
    match kind {
        MetricKind::Counter | MetricKind::Histogram => current.saturating_add(value),
        MetricKind::Gauge => value,
    }
}

impl<const CAPACITY: usize> Default for MetricRegistry<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AlertLevel {
    Info = 1,
    Warning = 2,
    Critical = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Alert {
    pub sequence: u64,
    pub timestamp: u64,
    pub level: AlertLevel,
    pub code: u16,
    pub dimensions: TelemetryDimensions,
    pub correlation: CorrelationId,
}

pub struct AlertRegistry<const CAPACITY: usize = MAX_ALERTS> {
    alerts: [Option<Alert>; CAPACITY],
    next_sequence: u64,
    dropped: u64,
}

impl<const CAPACITY: usize> AlertRegistry<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY >= 1);
        Self { alerts: [None; CAPACITY], next_sequence: 1, dropped: 0 }
    }

    pub fn push(&mut self, mut alert: Alert) -> Result<u64, MetricError> {
        if alert.timestamp == 0 || alert.code == 0 || alert.dimensions.node == 0 {
            return Err(MetricError::Invalid);
        }
        alert.sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        if let Some(slot) = self.alerts.iter_mut().find(|entry| entry.is_none()) {
            *slot = Some(alert);
            return Ok(alert.sequence)
        }
        self.dropped = self.dropped.saturating_add(1);
        Err(MetricError::Capacity)
    }

    pub fn alerts(&self) -> impl Iterator<Item = Alert> + '_ {
        self.alerts.iter().flatten().copied()
    }

    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn drain(&mut self, destination: &mut [Option<Alert>]) -> usize {
        let mut count = 0;
        for alert in self.alerts.iter_mut() {
            if let Some(value) = alert.take() {
                if let Some(slot) = destination.get_mut(count) {
                    *slot = Some(value);
                    count += 1;
                } else {
                    *alert = Some(value);
                    break
                }
            }
        }
        count
    }
}

impl<const CAPACITY: usize> Default for AlertRegistry<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

pub fn drain_trace(destination: &mut [Option<TraceEvent>]) -> usize {
    drain_ring(&SYSTEM_TRACE, destination)
}

pub fn drain_logs(destination: &mut [Option<TraceEvent>]) -> usize {
    drain_trace(destination)
}

pub fn drain_audit(destination: &mut [Option<TraceEvent>]) -> usize {
    drain_ring(&SECURITY_AUDIT, destination)
}

fn drain_ring<const CAPACITY: usize>(
    ring: &TraceRing<CAPACITY>,
    destination: &mut [Option<TraceEvent>],
) -> usize {
    if destination.is_empty() {
        return 0;
    }
    let mut count = 0;
    while count < destination.len() {
        let Some(event) = ring.try_pop() else { break };
        if let Some(slot) = destination.get_mut(count) {
            *slot = Some(event);
            count += 1;
        }
    }
    count
}
