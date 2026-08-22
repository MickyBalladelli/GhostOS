use ghostos_fabric::NodeId;
use ghostos_observability::{EventField, Level, audit_event, field};
use ghostos_status::IntoStatus;

use crate::Error;

pub const DEFAULT_PROBE_CAPACITY: usize = 256;
pub const MAX_MEMORY_RULES: usize = 16;
pub const MAX_CAPABILITY_RULES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Operation {
    IpcSend = 1,
    IpcReceive = 2,
    CapabilityUse = 3,
    MemoryRead = 4,
    MemoryWrite = 5,
    MemoryExecute = 6,
}

impl Operation {
    pub const fn mask(self) -> u8 {
        1 << (self as u8 - 1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryRule {
    pub start: u64,
    pub length: u64,
    pub writable: bool,
    pub executable: bool,
}

impl MemoryRule {
    pub const fn new(
        start: u64,
        length: u64,
        writable: bool,
        executable: bool,
    ) -> Option<Self> {
        if length == 0 || start.checked_add(length).is_none() {
            None
        } else {
            Some(Self {
                start,
                length,
                writable,
                executable,
            })
        }
    }

    const fn contains(self, address: u64, length: u64) -> bool {
        if address < self.start {
            return false
        }
        let Some(end) = address.checked_add(length) else {
            return false
        };
        let Some(rule_end) = self.start.checked_add(self.length) else {
            return false
        };
        end <= rule_end
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityRule {
    pub subject: u64,
    pub capability: u64,
    pub operations: u8,
}

impl CapabilityRule {
    pub const fn allows(self, subject: u64, capability: u64, operation: Operation) -> bool {
        self.subject == subject
            && self.capability == capability
            && self.operations & operation.mask() != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeEvent {
    pub timestamp_us: u64,
    pub node: NodeId,
    pub subject: u64,
    pub capability: u64,
    pub operation: Operation,
    pub address: u64,
    pub length: u64,
}

impl ProbeEvent {
    pub const fn capability_use(
        timestamp_us: u64,
        node: NodeId,
        subject: u64,
        capability: u64,
        operation: Operation,
    ) -> Self {
        Self {
            timestamp_us,
            node,
            subject,
            capability,
            operation,
            address: 0,
            length: 0,
        }
    }

    pub const fn memory_access(
        timestamp_us: u64,
        node: NodeId,
        subject: u64,
        capability: u64,
        operation: Operation,
        address: u64,
        length: u64,
    ) -> Self {
        Self {
            timestamp_us,
            node,
            subject,
            capability,
            operation,
            address,
            length,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeOutcome {
    Allowed,
    Denied,
    Throttled { retry_after_us: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbePolicy {
    pub window_us: u64,
    pub max_events: u32,
    pub memory_rules: [Option<MemoryRule>; MAX_MEMORY_RULES],
    pub capability_rules: [Option<CapabilityRule>; MAX_CAPABILITY_RULES],
}

impl ProbePolicy {
    pub const fn new(window_us: u64, max_events: u32) -> Self {
        Self {
            window_us,
            max_events,
            memory_rules: [None; MAX_MEMORY_RULES],
            capability_rules: [None; MAX_CAPABILITY_RULES],
        }
    }

    pub fn add_memory_rule(&mut self, rule: MemoryRule) -> Result<(), Error> {
        let slot = self
            .memory_rules
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(rule);
        Ok(())
    }

    pub fn add_capability_rule(&mut self, rule: CapabilityRule) -> Result<(), Error> {
        if rule.subject == 0 || rule.capability == 0 || rule.operations == 0 {
            return Err(Error::InvalidConfiguration)
        }
        if self.capability_rules.iter().flatten().any(|entry| *entry == rule) {
            return Err(Error::Duplicate)
        }
        let slot = self
            .capability_rules
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(rule);
        Ok(())
    }

    fn allows(&self, event: ProbeEvent) -> bool {
        let capability_allowed = self
            .capability_rules
            .iter()
            .flatten()
            .any(|rule| rule.allows(event.subject, event.capability, event.operation));
        if !capability_allowed {
            return false
        }

        if event.length == 0 {
            return true
        }

        self.memory_rules.iter().flatten().any(|rule| {
            rule.contains(event.address, event.length)
                && match event.operation {
                    Operation::MemoryWrite => rule.writable,
                    Operation::MemoryExecute => rule.executable,
                    _ => true,
                }
        })
    }
}

#[derive(Clone, Copy)]
struct SubjectWindow {
    subject: u64,
    window_start_us: u64,
    events: u32,
}

impl SubjectWindow {
    const EMPTY: Self = Self {
        subject: 0,
        window_start_us: 0,
        events: 0,
    };
}

pub trait ProbeSink {
    fn record(&mut self, event: ProbeEvent) -> Result<(), Error>;
}

pub struct NoopProbe;

impl ProbeSink for NoopProbe {
    #[inline(always)]
    fn record(&mut self, _event: ProbeEvent) -> Result<(), Error> {
        Ok(())
    }
}

pub struct ProbeRecorder<const CAPACITY: usize = DEFAULT_PROBE_CAPACITY> {
    events: [Option<ProbeEvent>; CAPACITY],
    next: usize,
    len: usize,
    dropped: u64,
}

impl<const CAPACITY: usize> ProbeRecorder<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            events: [None; CAPACITY],
            next: 0,
            len: 0,
            dropped: 0,
        }
    }

    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn events(&self) -> impl Iterator<Item = ProbeEvent> + '_ {
        self.events.iter().flatten().copied()
    }
}

impl<const CAPACITY: usize> Default for ProbeRecorder<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAPACITY: usize> ProbeSink for ProbeRecorder<CAPACITY> {
    fn record(&mut self, event: ProbeEvent) -> Result<(), Error> {
        if CAPACITY == 0 {
            return Err(Error::Capacity)
        }
        if self.events[self.next].is_some() {
            self.dropped = self.dropped.saturating_add(1)
        } else {
            self.len += 1
        }
        self.events[self.next] = Some(event);
        self.next = (self.next + 1) % CAPACITY;
        Ok(())
    }
}

pub struct BehaviorMonitor<
    S,
    const SUBJECTS: usize = DEFAULT_PROBE_CAPACITY,
> {
    sink: S,
    policy: ProbePolicy,
    windows: [SubjectWindow; SUBJECTS],
}

impl<S, const SUBJECTS: usize> BehaviorMonitor<S, SUBJECTS>
where
    S: ProbeSink,
{
    pub fn new(sink: S, policy: ProbePolicy) -> Result<Self, Error> {
        if policy.window_us == 0 || policy.max_events == 0 || SUBJECTS == 0 {
            return Err(Error::InvalidConfiguration)
        }
        Ok(Self {
            sink,
            policy,
            windows: [SubjectWindow::EMPTY; SUBJECTS],
        })
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    pub fn observe(&mut self, event: ProbeEvent) -> Result<ProbeOutcome, Error> {
        if event.subject == 0 || event.capability == 0 {
            return Err(Error::InvalidInput)
        }
        if !self.policy.allows(event) {
            audit_event!(
                Level::Warn,
                EventField::unsigned(field::CALLER, event.subject),
                EventField::unsigned(field::CAPABILITY, event.capability),
                EventField::unsigned(field::ADDRESS, event.address),
                EventField::status(Error::Unauthorized.status()),
            );
            return Ok(ProbeOutcome::Denied)
        }

        let window = self
            .windows
            .iter_mut()
            .find(|entry| entry.subject == event.subject || entry.subject == 0)
            .ok_or(Error::Capacity)?;
        if window.subject == 0 || event.timestamp_us.saturating_sub(window.window_start_us)
            >= self.policy.window_us
        {
            *window = SubjectWindow {
                subject: event.subject,
                window_start_us: event.timestamp_us,
                events: 0,
            }
        }
        if window.events >= self.policy.max_events {
            let retry_after_us = self.policy.window_us.saturating_sub(
                event.timestamp_us.saturating_sub(window.window_start_us),
            );
            audit_event!(
                Level::Warn,
                EventField::unsigned(field::CALLER, event.subject),
                EventField::unsigned(field::OPERATION, event.operation as u64),
                EventField::unsigned(field::LENGTH, event.length),
                EventField::status(Error::FaultRateExceeded.status()),
            );
            return Ok(ProbeOutcome::Throttled { retry_after_us })
        }
        window.events += 1;
        self.sink.record(event)?;
        Ok(ProbeOutcome::Allowed)
    }
}

#[inline(always)]
pub fn trace<S: ProbeSink>(sink: &mut S, event: ProbeEvent) -> Result<(), Error> {
    sink.record(event)
}
