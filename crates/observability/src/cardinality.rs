//! Fixed-capacity cardinality guards for telemetry dimensions and labels.
//!
//! Every table keeps exact counts for the first bounded set of keys and folds
//! later keys into an overflow counter. This makes hostile high-cardinality
//! input visible without allocating or making ingestion fail.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use crate::{EventKind, TraceEvent};

pub const MAX_METRIC_CARDINALITY: usize = 64;
pub const MAX_TRACE_LABEL_CARDINALITY: usize = 128;
pub const MAX_AUDIT_LABEL_CARDINALITY: usize = 64;
pub const MAX_METRIC_AGGREGATES: usize = 32;
pub const MAX_TENANT_DIAGNOSTICS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TenantDiagnosticKind {
    Metric = 1,
    Trace = 2,
    Audit = 3,
    Capture = 4,
    CaptureBytes = 5,
    Dropped = 6,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct TenantDiagnostic {
    /// Zero is the bounded aggregate bucket for tenants beyond the limit.
    pub tenant: u64,
    pub metric_samples: u64,
    pub trace_events: u64,
    pub audit_events: u64,
    pub capture_packets: u64,
    pub capture_bytes: u64,
    pub dropped: u64,
    pub aggregated: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TenantDiagnosticError {
    InvalidTenant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TenantDiagnosticOutcome {
    Exact,
    Aggregated,
}

/// Bounded per-tenant observability accounting. Unknown tenants are folded
/// into tenant zero instead of allocating another diagnostic record.
#[derive(Clone, Copy)]
pub struct TenantDiagnostics<const CAPACITY: usize = MAX_TENANT_DIAGNOSTICS> {
    entries: [Option<TenantDiagnostic>; CAPACITY],
    other: TenantDiagnostic,
}

impl<const CAPACITY: usize> TenantDiagnostics<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY >= 1);
        Self {
            entries: [None; CAPACITY],
            other: TenantDiagnostic {
                tenant: 0,
                metric_samples: 0,
                trace_events: 0,
                audit_events: 0,
                capture_packets: 0,
                capture_bytes: 0,
                dropped: 0,
                aggregated: 0,
            },
        }
    }

    pub fn record(
        &mut self,
        tenant: u64,
        kind: TenantDiagnosticKind,
        amount: u64,
    ) -> Result<TenantDiagnosticOutcome, TenantDiagnosticError> {
        if tenant == 0 {
            return Err(TenantDiagnosticError::InvalidTenant)
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.tenant == tenant)
        {
            apply_diagnostic(entry, kind, amount);
            return Ok(TenantDiagnosticOutcome::Exact)
        }
        if let Some(slot) = self.entries.iter_mut().find(|entry| entry.is_none()) {
            let mut entry = TenantDiagnostic {
                tenant,
                ..TenantDiagnostic::default()
            };
            apply_diagnostic(&mut entry, kind, amount);
            *slot = Some(entry);
            return Ok(TenantDiagnosticOutcome::Exact)
        }
        apply_diagnostic(&mut self.other, kind, amount);
        self.other.aggregated = self.other.aggregated.saturating_add(1);
        Ok(TenantDiagnosticOutcome::Aggregated)
    }

    pub fn diagnostics(&self) -> impl Iterator<Item = TenantDiagnostic> + '_ {
        self.entries
            .iter()
            .flatten()
            .copied()
            .chain((self.other.aggregated != 0).then_some(self.other))
    }

    pub const fn other(&self) -> TenantDiagnostic {
        self.other
    }
}

impl<const CAPACITY: usize> Default for TenantDiagnostics<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn apply_diagnostic(entry: &mut TenantDiagnostic, kind: TenantDiagnosticKind, amount: u64) {
    match kind {
        TenantDiagnosticKind::Metric => {
            entry.metric_samples = entry.metric_samples.saturating_add(amount)
        }
        TenantDiagnosticKind::Trace => {
            entry.trace_events = entry.trace_events.saturating_add(amount)
        }
        TenantDiagnosticKind::Audit => {
            entry.audit_events = entry.audit_events.saturating_add(amount)
        }
        TenantDiagnosticKind::Capture => {
            entry.capture_packets = entry.capture_packets.saturating_add(amount)
        }
        TenantDiagnosticKind::CaptureBytes => {
            entry.capture_bytes = entry.capture_bytes.saturating_add(amount)
        }
        TenantDiagnosticKind::Dropped => {
            entry.dropped = entry.dropped.saturating_add(amount)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CardinalityEntry {
    pub scope: u64,
    pub value: u128,
    pub observations: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct CardinalitySnapshot {
    pub unique: usize,
    pub observations: u64,
    pub overflow: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CardinalityDecision {
    Admitted,
    Aggregated,
}

/// Single-writer cardinality table for metric and audit registries.
#[derive(Clone, Copy)]
pub struct CardinalityTable<const CAPACITY: usize> {
    entries: [Option<CardinalityEntry>; CAPACITY],
    observations: u64,
    overflow: u64,
}

impl<const CAPACITY: usize> CardinalityTable<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY >= 1);
        Self {
            entries: [None; CAPACITY],
            observations: 0,
            overflow: 0,
        }
    }

    pub fn observe(&mut self, scope: u64, value: u128) -> CardinalityDecision {
        self.observations = self.observations.saturating_add(1);
        if let Some(entry) = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.scope == scope && entry.value == value)
        {
            entry.observations = entry.observations.saturating_add(1);
            return CardinalityDecision::Admitted
        }
        if let Some(slot) = self.entries.iter_mut().find(|entry| entry.is_none()) {
            *slot = Some(CardinalityEntry {
                scope,
                value,
                observations: 1,
            });
            CardinalityDecision::Admitted
        } else {
            self.overflow = self.overflow.saturating_add(1);
            CardinalityDecision::Aggregated
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = CardinalityEntry> + '_ {
        self.entries.iter().flatten().copied()
    }

    pub const fn snapshot(&self) -> CardinalitySnapshot {
        let mut unique = 0;
        let mut index = 0;
        while index < CAPACITY {
            if self.entries[index].is_some() {
                unique += 1;
            }
            index += 1;
        }
        CardinalitySnapshot {
            unique,
            observations: self.observations,
            overflow: self.overflow,
        }
    }

    pub const fn overflow(&self) -> u64 {
        self.overflow
    }
}

impl<const CAPACITY: usize> Default for CardinalityTable<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

struct AtomicCardinalitySlot {
    state: AtomicU8,
    scope: AtomicU64,
    value_low: AtomicU64,
    value_high: AtomicU64,
    observations: AtomicU64,
}

impl AtomicCardinalitySlot {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(0),
            scope: AtomicU64::new(0),
            value_low: AtomicU64::new(0),
            value_high: AtomicU64::new(0),
            observations: AtomicU64::new(0),
        }
    }
}

/// Lock-free cardinality guard for multi-producer trace rings.
pub struct AtomicCardinality<const CAPACITY: usize> {
    entries: [AtomicCardinalitySlot; CAPACITY],
    observations: AtomicU64,
    overflow: AtomicU64,
}

impl<const CAPACITY: usize> AtomicCardinality<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY >= 1);
        Self {
            entries: [const { AtomicCardinalitySlot::new() }; CAPACITY],
            observations: AtomicU64::new(0),
            overflow: AtomicU64::new(0),
        }
    }

    pub fn observe(&self, scope: u64, value: u128) -> CardinalityDecision {
        self.observations.fetch_add(1, Ordering::Relaxed);
        for entry in &self.entries {
            if entry.state.load(Ordering::Acquire) == 1
                && entry.scope.load(Ordering::Relaxed) == scope
                && entry.value_low.load(Ordering::Relaxed) == value as u64
                && entry.value_high.load(Ordering::Relaxed) == (value >> 64) as u64
            {
                entry.observations.fetch_add(1, Ordering::Relaxed);
                return CardinalityDecision::Admitted
            }
        }
        for entry in &self.entries {
            if entry
                .state
                .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                entry.scope.store(scope, Ordering::Relaxed);
                entry.value_low.store(value as u64, Ordering::Relaxed);
                entry.value_high.store((value >> 64) as u64, Ordering::Relaxed);
                entry.observations.store(1, Ordering::Relaxed);
                entry.state.store(1, Ordering::Release);
                return CardinalityDecision::Admitted
            }
        }
        self.overflow.fetch_add(1, Ordering::Relaxed);
        CardinalityDecision::Aggregated
    }

    pub fn observe_event(&self, event: TraceEvent) {
        let mut fields = 0;
        for field in event.fields() {
            let scope = ((event.kind as u64) << 16) | field.key as u64;
            self.observe(scope, field.as_u128());
            fields += 1;
        }
        if fields == 0 {
            self.observe(event.kind as u64, 0);
        }
    }

    pub fn snapshot(&self) -> CardinalitySnapshot {
        let unique = self
            .entries
            .iter()
            .filter(|entry| entry.state.load(Ordering::Acquire) == 1)
            .count();
        CardinalitySnapshot {
            unique,
            observations: self.observations.load(Ordering::Acquire),
            overflow: self.overflow.load(Ordering::Acquire),
        }
    }

    pub fn export(&self, destination: &mut [Option<CardinalityEntry>]) -> usize {
        let mut count = 0;
        for entry in &self.entries {
            if count == destination.len() {
                break
            }
            if entry.state.load(Ordering::Acquire) != 1 {
                continue
            }
            destination[count] = Some(CardinalityEntry {
                scope: entry.scope.load(Ordering::Relaxed),
                value: entry.value_low.load(Ordering::Relaxed) as u128
                    | ((entry.value_high.load(Ordering::Relaxed) as u128) << 64),
                observations: entry.observations.load(Ordering::Relaxed),
            });
            count += 1;
        }
        count
    }
}

impl<const CAPACITY: usize> Default for AtomicCardinality<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuditLabelAggregate {
    pub field: u16,
    pub value: u128,
    pub observations: u64,
}

/// Tracks audit label values separately from the durable audit records.
/// Security evidence remains exact while label explosions are folded.
#[derive(Clone, Copy)]
pub struct AuditLabelAggregator<const CAPACITY: usize = MAX_AUDIT_LABEL_CARDINALITY> {
    table: CardinalityTable<CAPACITY>,
}

impl<const CAPACITY: usize> AuditLabelAggregator<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            table: CardinalityTable::new(),
        }
    }

    pub fn observe(&mut self, event: TraceEvent) {
        if event.kind != EventKind::Audit {
            return
        }
        for field in event.fields() {
            self.table.observe(field.key as u64, field.as_u128());
        }
    }

    pub fn aggregates(&self) -> impl Iterator<Item = AuditLabelAggregate> + '_ {
        self.table.entries().map(|entry| AuditLabelAggregate {
            field: entry.scope as u16,
            value: entry.value,
            observations: entry.observations,
        })
    }

    pub const fn snapshot(&self) -> CardinalitySnapshot {
        self.table.snapshot()
    }
}

impl<const CAPACITY: usize> Default for AuditLabelAggregator<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
