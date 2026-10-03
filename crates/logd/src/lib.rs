#![no_std]
#![deny(unsafe_code)]

#[allow(unsafe_code)]
mod native;

use ghostos_observability::{
    AuditJournal, AuditKey, AuditQuery, EventKind, JOURNAL_RECORD_SIZE, Level,
    SECURITY_AUDIT, SYSTEM_TRACE, TraceEvent, TraceRing, decode_record, encode_record,
    DEFAULT_RECOVERY_AUDIT_CAPACITY, MAX_TELEMETRY_BATCH, ScalePath, ScalePolicy,
};
use ghostos_observability::{BatchController, ProducerPolicy};
use ghostos_ghostfs::{
    BlockStore, CapacityObservation, CapacityResource, Error as SynFsError, SynFs, SynfsPurged,
};

pub const SYSTEM_JOURNAL: &str = "SYS$LOG:SYSTEM.JOURNAL";
pub const SECURITY_JOURNAL: &str = "SYS$LOG:SECURITY.AUDIT";
pub const SERVICE_JOURNAL: &str = "SYS$LOG:SERVICE.JOURNAL";
pub const RECOVERY_AUDIT_JOURNAL: &str = "SYS$LOG:SECURITY.RECOVERY";
pub const DEFAULT_JOURNAL_RETENTION: u32 = 1024;
pub const DEFAULT_OPCOM_SUBSCRIBERS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogError {
    Codec,
    Journal,
    Recovery,
    RecoveryUnavailable,
    CoreIsolated,
    InvalidService,
    SubscriberCapacity,
    UnknownSubscriber,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalStream {
    System,
    SecurityAudit,
    Service(u32),
}

pub trait JournalWriter {
    fn append(&mut self, stream: JournalStream, event: TraceEvent) -> Result<(), LogError>;

    fn append_service(&mut self, service: u32, event: TraceEvent) -> Result<(), LogError> {
        if service == 0 {
            return Err(LogError::InvalidService)
        }
        self.append(JournalStream::Service(service), event.for_service(service))
    }

    fn append_batch(&mut self, stream: JournalStream, events: &[TraceEvent]) -> Result<(), LogError> {
        for event in events {
            self.append(stream, *event)?
        }
        Ok(())
    }

    fn rotate(&mut self, work_budget: usize) -> Result<usize, LogError>;
}

/// GhostFS journal backend. Every fixed-size record is committed as a new CoW
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
            .add_rule(SERVICE_JOURNAL, keep_latest)
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

    pub fn capacity_observation(
        &self,
        resource: CapacityResource,
        sampled_at_us: u64,
        growth_bytes_per_hour: u64,
    ) -> Result<CapacityObservation, LogError> {
        let mut allocated_bytes = 0u64;
        for path in [
            SYSTEM_JOURNAL,
            SECURITY_JOURNAL,
            SERVICE_JOURNAL,
            RECOVERY_AUDIT_JOURNAL,
        ] {
            if let Ok(diagnostics) = self.filesystem.path_diagnostics(path) {
                allocated_bytes = allocated_bytes.saturating_add(diagnostics.retained_bytes)
            }
        }
        let fragmentation = self
            .filesystem
            .fragmentation_report()
            .map_err(|_| LogError::Journal)?;
        let block_bytes = ghostos_ghostfs::BLOCK_SIZE as u64;
        let fragmented_bytes = (fragmentation.fragmented_blocks as u64).saturating_mul(block_bytes);
        Ok(CapacityObservation {
            resource,
            sampled_at_us,
            capacity_bytes: (self.filesystem.capacity() as u64).saturating_mul(block_bytes),
            allocated_bytes: allocated_bytes.max(fragmented_bytes),
            reclaimable_bytes: fragmented_bytes,
            fragmented_bytes,
            largest_free_extent_bytes: (fragmentation.largest_free_run as u64).saturating_mul(block_bytes),
            allocation_unit_bytes: block_bytes,
            gc_pending_bytes: (fragmentation.reclaimable_blocks as u64).saturating_mul(block_bytes),
            gc_work_limit_bytes: (self.filesystem.capacity() as u64).saturating_mul(block_bytes),
            growth_bytes_per_hour,
        })
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

    /// Make all log records visible to the persistent volume's recovery
    /// generation. A successful return means the records survive reboot.
    pub fn sync<D: BlockStore>(&mut self, device: &mut D) -> Result<(), LogError> {
        self.filesystem
            .fsync(device)
            .map(|_| ())
            .map_err(|_| LogError::Journal)
    }

    pub fn analyze_system(
        &self,
        query: AuditQuery,
        visitor: impl FnMut(TraceEvent),
    ) -> Result<usize, LogError> {
        self.analyze_stream(SYSTEM_JOURNAL, query, visitor)
    }

    pub fn analyze_security(
        &self,
        query: AuditQuery,
        visitor: impl FnMut(TraceEvent),
    ) -> Result<usize, LogError> {
        self.analyze_stream(SECURITY_JOURNAL, query, visitor)
    }

    pub fn analyze_service(
        &self,
        service: u32,
        query: AuditQuery,
        mut visitor: impl FnMut(TraceEvent),
    ) -> Result<usize, LogError> {
        if service == 0 {
            return Err(LogError::InvalidService)
        }
        let mut matched = 0;
        self.analyze_stream(SERVICE_JOURNAL, query, |event| {
            if event
                .field(ghostos_observability::field::SERVICE)
                .is_some_and(|field| field.as_u64() == u64::from(service))
            {
                matched += 1;
                visitor(event)
            }
        })?;
        Ok(matched)
    }

    fn analyze_stream(
        &self,
        path: &str,
        query: AuditQuery,
        mut visitor: impl FnMut(TraceEvent),
    ) -> Result<usize, LogError> {
        let (_, oldest) = match self.filesystem.retained_version_span(path) {
            Ok(span) => span,
            Err(SynFsError::NotFound) => return Ok(0),
            Err(_) => return Err(LogError::Journal),
        };
        let Some(oldest) = oldest else { return Ok(0) };
        let latest = self
            .filesystem
            .lookup(path)
            .map_err(|_| LogError::Journal)?
            .version;
        let mut matched = 0;
        let mut record = [0; JOURNAL_RECORD_SIZE];
        for version in oldest..=latest {
            match self.filesystem.read_version(path, version, &mut record) {
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
        let event = match stream {
            JournalStream::Service(service) if service != 0 => event.for_service(service),
            JournalStream::Service(_) => return Err(LogError::InvalidService),
            JournalStream::System | JournalStream::SecurityAudit => event,
        };
        let mut record = [0; JOURNAL_RECORD_SIZE];
        encode_record(event, &mut record).map_err(|_| LogError::Codec)?;
        let path = match stream {
            JournalStream::System => SYSTEM_JOURNAL,
            JournalStream::SecurityAudit => SECURITY_JOURNAL,
            JournalStream::Service(service) => {
                if service == 0 {
                    return Err(LogError::InvalidService)
                }
                self.filesystem
                    .write(SERVICE_JOURNAL, &record)
                    .map_err(|_| LogError::Journal)?;
                return Ok(())
            }
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

/// A GhostFS journal connected to the volume's block device. The plain
/// [`SynFsJournal`] is useful for volatile callers; this adapter makes every
/// log batch and retention change power-loss durable before it returns.
pub struct PersistentSynFsJournal<'fs, 'device, const BLOCKS: usize, D: BlockStore> {
    journal: SynFsJournal<'fs, BLOCKS>,
    device: &'device mut D,
}

impl<'fs, 'device, const BLOCKS: usize, D: BlockStore>
    PersistentSynFsJournal<'fs, 'device, BLOCKS, D>
{
    pub fn new(
        filesystem: &'fs mut SynFs<BLOCKS>,
        device: &'device mut D,
        keep_latest: u32,
    ) -> Result<Self, LogError> {
        Ok(Self {
            journal: SynFsJournal::new(filesystem, keep_latest)?,
            device,
        })
    }

    pub fn new_with_recovery_key(
        filesystem: &'fs mut SynFs<BLOCKS>,
        device: &'device mut D,
        keep_latest: u32,
        key: AuditKey,
    ) -> Result<Self, LogError> {
        Ok(Self {
            journal: SynFsJournal::new_with_recovery_key(filesystem, keep_latest, key)?,
            device,
        })
    }

    pub fn filesystem(&self) -> &SynFs<BLOCKS> {
        self.journal.filesystem()
    }

    pub fn sync(&mut self) -> Result<(), LogError> {
        self.journal.sync(self.device)
    }

    pub fn persist_recovery_audit(&mut self, staging: &mut [u8]) -> Result<(), LogError> {
        self.journal.persist_recovery_audit(staging)?;
        self.sync()
    }

    pub fn journal(&self) -> &SynFsJournal<'fs, BLOCKS> {
        &self.journal
    }
}

impl<const BLOCKS: usize, D: BlockStore> JournalWriter
    for PersistentSynFsJournal<'_, '_, BLOCKS, D>
{
    fn append(&mut self, stream: JournalStream, event: TraceEvent) -> Result<(), LogError> {
        self.journal.append(stream, event)?;
        self.sync()
    }

    fn append_batch(&mut self, stream: JournalStream, events: &[TraceEvent]) -> Result<(), LogError> {
        self.journal.append_batch(stream, events)?;
        self.sync()
    }

    fn rotate(&mut self, work_budget: usize) -> Result<usize, LogError> {
        let purged = self.journal.rotate(work_budget)?;
        if purged != 0 {
            self.sync()?
        }
        Ok(purged)
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
        let views = self.subscription_views();
        match crate::native::subscribe(&views, terminal.raw(), minimum_level as u8) {
            Ok((true, index)) => {
                if let Some(subscription) = self.subscriptions[index].as_mut() {
                    subscription.minimum_level = minimum_level;
                }
            }
            Ok((false, index)) => {
                self.subscriptions[index] = Some(Subscription { terminal, minimum_level });
            }
            Err(()) => return Err(LogError::SubscriberCapacity),
        }
        Ok(())
    }

    pub fn unsubscribe(&mut self, terminal: TerminalId) -> Result<(), LogError> {
        let index = crate::native::unsubscribe(&self.subscription_views(), terminal.raw())
            .map_err(|_| LogError::UnknownSubscriber)?;
        self.subscriptions[index] = None;
        Ok(())
    }

    pub fn broadcast(
        &self,
        event: TraceEvent,
        mut deliver: impl FnMut(TerminalId, TraceEvent),
    ) -> usize {
        let mut delivered = 0;
        for subscription in self.subscriptions.iter().flatten() {
            if crate::native::deliver(event.level as u8, subscription.minimum_level as u8) {
                deliver(subscription.terminal, event);
                delivered += 1
            }
        }
        delivered
    }

    fn subscription_views(&self) -> [crate::native::Subscription; SUBSCRIBERS] {
        self.subscriptions.map(|subscription| match subscription {
            Some(subscription) => crate::native::Subscription {
                terminal: subscription.terminal.raw(),
                minimum_level: subscription.minimum_level as u8,
                occupied: true,
            },
            None => crate::native::Subscription { terminal: 0, minimum_level: 0, occupied: false },
        })
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
    pub batches: usize,
    pub interrupts_moderated: usize,
}

/// Ring-3 daemon state machine. The ring references represent read-only pages
/// mapped from the kernel through capability-checked zero-copy IPC.
pub struct LogDaemon {
    last_trace_dropped: u64,
    last_audit_dropped: u64,
    controller: BatchController,
    scale_policy: ScalePolicy,
}

impl LogDaemon {
    pub const fn new() -> Self {
        Self {
            last_trace_dropped: 0,
            last_audit_dropped: 0,
            controller: BatchController::new(ProducerPolicy::LOGGING),
            scale_policy: ScalePolicy::for_cpu_count(1).expect("one CPU scale tier"),
        }
    }

    pub const fn scale_policy(&self) -> ScalePolicy {
        self.scale_policy
    }

    pub fn set_scale_policy(&mut self, policy: ScalePolicy) {
        self.scale_policy = policy
    }

    pub const fn accepts_on(&self, cpu: usize) -> bool {
        self.scale_policy.accepts(ScalePath::Logging, cpu)
    }

    pub fn poll_on_cpu<
        const TRACE_CAPACITY: usize,
        const AUDIT_CAPACITY: usize,
        const SUBSCRIBERS: usize,
        Writer: JournalWriter,
    >(
        &mut self,
        cpu: usize,
        trace: &TraceRing<TRACE_CAPACITY>,
        audit: &TraceRing<AUDIT_CAPACITY>,
        writer: &mut Writer,
        opcom: &Opcom<SUBSCRIBERS>,
        record_budget: usize,
        rotation_budget: usize,
        now_us: u64,
        interactive_pending: bool,
        deliver: impl FnMut(TerminalId, TraceEvent),
    ) -> Result<PollReport, LogError> {
        if !self.accepts_on(cpu) {
            return Err(LogError::CoreIsolated)
        }
        self.poll_with_policy(
            trace,
            audit,
            writer,
            opcom,
            record_budget,
            rotation_budget,
            now_us,
            interactive_pending,
            deliver,
        )
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
        deliver: impl FnMut(TerminalId, TraceEvent),
    ) -> Result<PollReport, LogError> {
        self.poll_with_policy(
            trace,
            audit,
            writer,
            opcom,
            record_budget,
            rotation_budget,
            0,
            false,
            deliver,
        )
    }

    pub fn poll_with_policy<
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
        now_us: u64,
        interactive_pending: bool,
        mut deliver: impl FnMut(TerminalId, TraceEvent),
    ) -> Result<PollReport, LogError> {
        let mut report = PollReport::default();
        let trace_decision = self.controller.plan(
            trace.pending().min(record_budget),
            JOURNAL_RECORD_SIZE,
            now_us,
            interactive_pending,
        );
        let mut trace_batch = [TraceEvent::new(Level::Info, EventKind::Kernel); MAX_TELEMETRY_BATCH];
        let mut trace_count = 0;
        while trace_count < trace_decision.count {
            let Some(event) = trace.try_pop() else { break };
            trace_batch[trace_count] = event;
            trace_count += 1;
        }
        if trace_count != 0 {
            writer.append_batch(JournalStream::System, &trace_batch[..trace_count])?;
            report.batches += 1;
            report.system_records = trace_count;
            for event in &trace_batch[..trace_count] {
                if crate::native::operator(event.level as u8, event.kind as u8) {
                    report.operator_deliveries += opcom.broadcast(*event, &mut deliver)
                }
            }
        }
        if trace_decision.interrupt {
            report.interrupts_moderated += 1
        }
        let audit_decision = self.controller.plan(
            audit.pending().min(crate::native::remaining(record_budget, report.system_records)),
            JOURNAL_RECORD_SIZE,
            now_us,
            interactive_pending,
        );
        let mut audit_batch = [TraceEvent::new(Level::Info, EventKind::Audit); MAX_TELEMETRY_BATCH];
        let mut audit_count = 0;
        while audit_count < audit_decision.count {
            let Some(event) = audit.try_pop() else { break };
            audit_batch[audit_count] = event;
            audit_count += 1;
        }
        if audit_count != 0 {
            writer.append_batch(JournalStream::SecurityAudit, &audit_batch[..audit_count])?;
            report.batches += 1;
            report.audit_records = audit_count
        }
        if audit_decision.interrupt {
            report.interrupts_moderated += 1
        }
        report.versions_rotated = writer.rotate(rotation_budget)?;
        let trace_dropped = trace.dropped();
        let audit_dropped = audit.dropped();
        report.trace_records_dropped = crate::native::dropped_delta(trace_dropped, self.last_trace_dropped);
        report.audit_records_dropped = crate::native::dropped_delta(audit_dropped, self.last_audit_dropped);
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
