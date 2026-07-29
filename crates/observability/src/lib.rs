#![no_std]
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use synos_status::Status;

pub const MAX_EVENT_FIELDS: usize = 4;
pub const JOURNAL_RECORD_SIZE: usize = 128;
pub const GLOBAL_TRACE_CAPACITY: usize = 256;
pub const GLOBAL_AUDIT_CAPACITY: usize = 256;

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
            node: AtomicU32::new(1),
            level: AtomicU8::new(Level::Trace as u8),
            kind: AtomicU8::new(EventKind::Kernel as u8),
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
            published: AtomicU64::new(u64::MAX),
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
    slots: [TraceSlot; CAPACITY],
}

impl<const CAPACITY: usize> TraceRing<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY >= 2);
        Self {
            write_position: AtomicU64::new(0),
            read_position: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            slots: [const { TraceSlot::new() }; CAPACITY],
        }
    }

    pub fn push(&self, event: TraceEvent) {
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
    if u32::from_le_bytes(source[..4].try_into().expect("four bytes")) != JOURNAL_MAGIC {
        return Err(CodecError::InvalidMagic);
    }
    if source[4] != JOURNAL_VERSION {
        return Err(CodecError::InvalidVersion);
    }
    if u64::from_le_bytes(source[120..128].try_into().expect("eight bytes"))
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
        .at(u64::from_le_bytes(
            source[8..16].try_into().expect("eight bytes"),
        ))
        .correlated(CorrelationId::from_raw(u128::from_le_bytes(
            source[16..32].try_into().expect("sixteen bytes"),
        )))
        .on_node(u32::from_le_bytes(
            source[32..36].try_into().expect("four bytes"),
        ));
    for index in 0..field_count {
        let offset = 40 + index * 20;
        let key = u16::from_le_bytes(source[offset..offset + 2].try_into().expect("two bytes"));
        let kind = FieldKind::from_raw(source[offset + 2]).ok_or(CodecError::InvalidField)?;
        let value = u128::from_le_bytes(
            source[offset + 4..offset + 20]
                .try_into()
                .expect("sixteen bytes"),
        );
        event = event.with_field(EventField { key, kind, value })
    }
    Ok(event)
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
