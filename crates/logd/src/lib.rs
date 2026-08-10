#![no_std]
#![forbid(unsafe_code)]

use synos_observability::{
    AuditJournal, AuditKey, AuditQuery, EventKind, JOURNAL_RECORD_SIZE, Level,
    SECURITY_AUDIT, SYSTEM_TRACE, TraceEvent, TraceRing, decode_record, encode_record,
    DEFAULT_RECOVERY_AUDIT_CAPACITY,
};
use synos_synfs::{Error as SynFsError, SynFs, SynfsPurged};

pub const SYSTEM_JOURNAL: &str = "SYS$LOG:SYSTEM.JOURNAL";
pub const SECURITY_JOURNAL: &str = "SYS$LOG:SECURITY.AUDIT";
pub const RECOVERY_AUDIT_JOURNAL: &str = "SYS$LOG:SECURITY.RECOVERY";
pub const DEFAULT_JOURNAL_RETENTION: u32 = 1024;
pub const DEFAULT_OPCOM_SUBSCRIBERS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogError {
    Codec,
    Journal,
    Recovery,
    RecoveryUnavailable,
    SubscriberCapacity,
    UnknownSubscriber,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalStream {
    System,
    SecurityAudit,
}

pub trait JournalWriter {
    fn append(&mut self, stream: JournalStream, event: TraceEvent) -> Result<(), LogError>;

    fn rotate(&mut self, work_budget: usize) -> Result<usize, LogError>;
}

/// SynFS journal backend. Every fixed-size record is committed as a new CoW
/// file version. Rotation tombstones old versions in bounded background work.
pub struct SynFsJournal<'a, const BLOCKS: usize> {
    filesystem: &'a mut SynFs<BLOCKS>,
    purger: SynfsPurged,
    recovery: Option<AuditJournal<DEFAULT_RECOVERY_AUDIT_CAPACITY>>,
}

impl<'a, const BLOCKS: usize> SynFsJournal<'a, BLOCKS> {
    pub fn new(filesystem: &'a mut SynFs<BLOCKS>, keep_latest: u32) -> Result<Self, LogError> {
        let mut purger = SynfsPurged::new();
        purger
            .add_rule(SYSTEM_JOURNAL, keep_latest)
            .map_err(|_| LogError::Journal)?;
        purger
            .add_rule(SECURITY_JOURNAL, keep_latest)
            .map_err(|_| LogError::Journal)?;
        purger
            .add_rule(RECOVERY_AUDIT_JOURNAL, keep_latest)
            .map_err(|_| LogError::Journal)?;
        Ok(Self {
            filesystem,
            purger,
            recovery: None,
        })
    }

    /// Create a journal with a bounded in-memory audit copy. The copy is
    /// sealed independently, so it remains exportable if normal log writes
    /// fail or the log daemon stops.
    pub fn new_with_recovery_key(
        filesystem: &'a mut SynFs<BLOCKS>,
        keep_latest: u32,
        key: AuditKey,
    ) -> Result<Self, LogError> {
        let mut journal = Self::new(filesystem, keep_latest)?;
        journal.recovery = Some(
            AuditJournal::new(key).map_err(|_| LogError::Recovery)?,
        );
        Ok(journal)
    }

    pub fn filesystem(&self) -> &SynFs<BLOCKS> {
        self.filesystem
    }

    pub fn recovery_audit(
        &self,
    ) -> Option<&AuditJournal<DEFAULT_RECOVERY_AUDIT_CAPACITY>> {
        self.recovery.as_ref()
    }

    pub fn export_recovery_audit(&self, output: &mut [u8]) -> Result<usize, LogError> {
        self.recovery
            .as_ref()
            .ok_or(LogError::RecoveryUnavailable)?
            .export(output)
            .map_err(|_| LogError::Recovery)
    }

    /// Persist the fallback journal with caller-owned bounded scratch space.
    /// Recovery can load this file without starting the normal log consumer.
    pub fn persist_recovery_audit(&mut self, staging: &mut [u8]) -> Result<(), LogError> {
        let length = self.export_recovery_audit(staging)?;
        self.filesystem
            .write(RECOVERY_AUDIT_JOURNAL, &staging[..length])
            .map_err(|_| LogError::Recovery)?;
        Ok(())
    }

    pub fn load_recovery_audit<const CAPACITY: usize>(
        &self,
        staging: &mut [u8],
        key: AuditKey,
    ) -> Result<AuditJournal<CAPACITY>, LogError> {
        let read = self
            .filesystem
            .read(RECOVERY_AUDIT_JOURNAL, staging)
            .map_err(|_| LogError::Recovery)?;
        AuditJournal::recover(&staging[..read.bytes_read], key)
            .map_err(|_| LogError::Recovery)
    }

    pub fn recover_audit<const CAPACITY: usize>(
        input: &[u8],
        key: AuditKey,
    ) -> Result<AuditJournal<CAPACITY>, LogError> {
        AuditJournal::recover(input, key).map_err(|_| LogError::Recovery)
    }

    pub fn analyze_security(
        &self,
        query: AuditQuery,
        mut visitor: impl FnMut(TraceEvent),
    ) -> Result<usize, LogError> {
        let (_, oldest) = self
            .filesystem
            .retained_version_span(SECURITY_JOURNAL)
            .map_err(|_| LogError::Journal)?;
        let Some(oldest) = oldest else { return Ok(0) };
        let latest = self
            .filesystem
            .lookup(SECURITY_JOURNAL)
            .map_err(|_| LogError::Journal)?
            .version;
        let mut matched = 0;
        let mut record = [0; JOURNAL_RECORD_SIZE];
        for version in oldest..=latest {
            match self
                .filesystem
                .read_version(SECURITY_JOURNAL, version, &mut record)
            {
                Ok(_) => {
                    let event = decode_record(&record).map_err(|_| LogError::Codec)?;
                    if query.matches(event) {
                        visitor(event);
                        matched += 1
                    }
                }
                Err(SynFsError::NotFound) => {}
                Err(_) => return Err(LogError::Journal),
            }
        }
        Ok(matched)
    }
}

impl<const BLOCKS: usize> JournalWriter for SynFsJournal<'_, BLOCKS> {
    fn append(&mut self, stream: JournalStream, event: TraceEvent) -> Result<(), LogError> {
        if stream == JournalStream::SecurityAudit {
            if let Some(recovery) = self.recovery.as_mut() {
                recovery.append(event).map_err(|_| LogError::Recovery)?;
            }
        }
        let mut record = [0; JOURNAL_RECORD_SIZE];
        encode_record(event, &mut record).map_err(|_| LogError::Codec)?;
        let path = match stream {
            JournalStream::System => SYSTEM_JOURNAL,
            JournalStream::SecurityAudit => SECURITY_JOURNAL,
        };
        self.filesystem
            .write(path, &record)
            .map_err(|_| LogError::Journal)?;
        Ok(())
    }

    fn rotate(&mut self, work_budget: usize) -> Result<usize, LogError> {
        self.purger
            .poll(self.filesystem, work_budget)
            .map(|report| report.versions_purged)
            .map_err(|_| LogError::Journal)
    }
}

impl From<SynFsError> for LogError {
    fn from(_: SynFsError) -> Self {
        Self::Journal
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct TerminalId(u32);

impl TerminalId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Subscription {
    terminal: TerminalId,
    minimum_level: Level,
}

/// OpenVMS-style operator communication registry. Delivery is callback-based
/// so terminals can be local console sessions or remote operator channels.
pub struct Opcom<const SUBSCRIBERS: usize = DEFAULT_OPCOM_SUBSCRIBERS> {
    subscriptions: [Option<Subscription>; SUBSCRIBERS],
}

impl<const SUBSCRIBERS: usize> Opcom<SUBSCRIBERS> {
    pub const fn new() -> Self {
        Self {
            subscriptions: [None; SUBSCRIBERS],
        }
    }

    pub fn subscribe(
        &mut self,
        terminal: TerminalId,
        minimum_level: Level,
    ) -> Result<(), LogError> {
        if let Some(subscription) = self
            .subscriptions
            .iter_mut()
            .flatten()
            .find(|entry| entry.terminal == terminal)
        {
            subscription.minimum_level = minimum_level;
            return Ok(());
        }
        let slot = self
            .subscriptions
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(LogError::SubscriberCapacity)?;
        *slot = Some(Subscription {
            terminal,
            minimum_level,
        });
        Ok(())
    }

    pub fn unsubscribe(&mut self, terminal: TerminalId) -> Result<(), LogError> {
        let slot = self
            .subscriptions
            .iter_mut()
            .find(|entry| entry.is_some_and(|subscription| subscription.terminal == terminal))
            .ok_or(LogError::UnknownSubscriber)?;
        *slot = None;
        Ok(())
    }

    pub fn broadcast(
        &self,
        event: TraceEvent,
        mut deliver: impl FnMut(TerminalId, TraceEvent),
    ) -> usize {
        let mut delivered = 0;
        for subscription in self.subscriptions.iter().flatten() {
            if event.level >= subscription.minimum_level {
                deliver(subscription.terminal, event);
                delivered += 1
            }
        }
        delivered
    }
}

impl<const SUBSCRIBERS: usize> Default for Opcom<SUBSCRIBERS> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct PollReport {
    pub system_records: usize,
    pub audit_records: usize,
    pub operator_deliveries: usize,
    pub versions_rotated: usize,
    pub trace_records_dropped: u64,
    pub audit_records_dropped: u64,
}

/// Ring-3 daemon state machine. The ring references represent read-only pages
/// mapped from the kernel through capability-checked zero-copy IPC.
pub struct LogDaemon {
    last_trace_dropped: u64,
    last_audit_dropped: u64,
}

impl LogDaemon {
    pub const fn new() -> Self {
        Self {
            last_trace_dropped: 0,
            last_audit_dropped: 0,
        }
    }

    pub fn poll<
        const TRACE_CAPACITY: usize,
        const AUDIT_CAPACITY: usize,
        const SUBSCRIBERS: usize,
        Writer: JournalWriter,
    >(
        &mut self,
        trace: &TraceRing<TRACE_CAPACITY>,
        audit: &TraceRing<AUDIT_CAPACITY>,
        writer: &mut Writer,
        opcom: &Opcom<SUBSCRIBERS>,
        record_budget: usize,
        rotation_budget: usize,
        mut deliver: impl FnMut(TerminalId, TraceEvent),
    ) -> Result<PollReport, LogError> {
        let mut report = PollReport::default();
        while report.system_records < record_budget {
            let Some(event) = trace.try_pop() else { break };
            writer.append(JournalStream::System, event)?;
            report.system_records += 1;
            if event.level >= Level::Error || event.kind == EventKind::Operator {
                report.operator_deliveries += opcom.broadcast(event, &mut deliver)
            }
        }
        while report.audit_records < record_budget {
            let Some(event) = audit.try_pop() else { break };
            writer.append(JournalStream::SecurityAudit, event)?;
            report.audit_records += 1
        }
        report.versions_rotated = writer.rotate(rotation_budget)?;
        let trace_dropped = trace.dropped();
        let audit_dropped = audit.dropped();
        report.trace_records_dropped = trace_dropped.saturating_sub(self.last_trace_dropped);
        report.audit_records_dropped = audit_dropped.saturating_sub(self.last_audit_dropped);
        self.last_trace_dropped = trace_dropped;
        self.last_audit_dropped = audit_dropped;
        Ok(report)
    }

    pub fn poll_global<const SUBSCRIBERS: usize, Writer: JournalWriter>(
        &mut self,
        writer: &mut Writer,
        opcom: &Opcom<SUBSCRIBERS>,
        record_budget: usize,
        rotation_budget: usize,
        deliver: impl FnMut(TerminalId, TraceEvent),
    ) -> Result<PollReport, LogError> {
        self.poll(
            &SYSTEM_TRACE,
            &SECURITY_AUDIT,
            writer,
            opcom,
            record_budget,
            rotation_budget,
            deliver,
        )
    }
}

impl Default for LogDaemon {
    fn default() -> Self {
        Self::new()
    }
}
