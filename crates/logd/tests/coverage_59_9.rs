use ghostos_logd::{
    JournalStream, JournalWriter, LogDaemon, LogError, Opcom, PollReport, SynFsJournal,
    TerminalId, SECURITY_JOURNAL,
};
use ghostos_observability::{EventKind, Level, TraceEvent, TraceRing};
use ghostos_observability::AuditQuery;
use ghostos_ghostfs::SynFs;

#[derive(Default)]
struct Writer {
    records: Vec<(JournalStream, TraceEvent)>,
    rotations: usize,
    fail: bool,
}

impl JournalWriter for Writer {
    fn append(&mut self, stream: JournalStream, event: TraceEvent) -> Result<(), LogError> {
        if self.fail {
            return Err(LogError::Journal)
        }
        self.records.push((stream, event));
        Ok(())
    }

    fn rotate(&mut self, work_budget: usize) -> Result<usize, LogError> {
        self.rotations += work_budget;
        Ok(work_budget)
    }
}

#[test]
fn daemon_filters_operator_delivery_and_reports_rotation() {
    let trace = TraceRing::<8>::new();
    let audit = TraceRing::<8>::new();
    trace.push(TraceEvent::new(Level::Info, EventKind::Kernel).at(1));
    trace.push(TraceEvent::new(Level::Error, EventKind::Kernel).at(2));
    audit.push(TraceEvent::new(Level::Info, EventKind::Audit).at(3));

    let terminal = TerminalId::new(7).unwrap();
    let mut opcom = Opcom::<2>::new();
    opcom.subscribe(terminal, Level::Error).unwrap();
    let mut writer = Writer::default();
    let mut daemon = LogDaemon::new();
    let mut delivered = Vec::new();
    let report = daemon
        .poll(&trace, &audit, &mut writer, &opcom, 8, 3, |terminal, event| {
            delivered.push((terminal.raw(), event.timestamp))
        })
        .unwrap();

    assert_eq!(report, PollReport {
        system_records: 2,
        audit_records: 1,
        operator_deliveries: 1,
        versions_rotated: 3,
        trace_records_dropped: 0,
        audit_records_dropped: 0,
        batches: 2,
        interrupts_moderated: 0,
    });
    assert_eq!(delivered, vec![(7, 2)]);
    assert_eq!(writer.records.len(), 3);
    assert_eq!(writer.rotations, 3);
}

#[test]
fn daemon_preserves_bounded_sink_failures_and_subscriber_limits() {
    let trace = TraceRing::<2>::new();
    let audit = TraceRing::<2>::new();
    trace.push(TraceEvent::new(Level::Info, EventKind::Kernel));
    let opcom = Opcom::<1>::new();
    let mut writer = Writer { fail: true, ..Writer::default() };
    assert_eq!(LogDaemon::new().poll(&trace, &audit, &mut writer, &opcom, 1, 0, |_, _| {}), Err(LogError::Journal));

    let mut opcom = Opcom::<1>::new();
    opcom.subscribe(TerminalId::new(1).unwrap(), Level::Info).unwrap();
    assert_eq!(opcom.subscribe(TerminalId::new(2).unwrap(), Level::Info), Err(LogError::SubscriberCapacity));
    assert_eq!(opcom.unsubscribe(TerminalId::new(2).unwrap()), Err(LogError::UnknownSubscriber));
}

#[test]
fn journal_records_survive_daemon_restart_and_bounded_rotation() {
    let mut filesystem = SynFs::<64>::new();
    {
        let mut journal = SynFsJournal::new(&mut filesystem, 2).unwrap();
        for timestamp in 1..=3 {
            journal
                .append(
                    JournalStream::SecurityAudit,
                    TraceEvent::new(Level::Info, EventKind::Audit).at(timestamp),
                )
                .unwrap();
        }
        assert!(journal.rotate(8).unwrap() <= 3);
    }

    let journal = SynFsJournal::new(&mut filesystem, 2).unwrap();
    let mut timestamps = Vec::new();
    let count = journal
        .analyze_security(AuditQuery::default(), |event| timestamps.push(event.timestamp))
        .unwrap();
    assert_eq!(count, timestamps.len());
    assert!(!timestamps.is_empty());
    assert!(timestamps.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(filesystem.lookup(SECURITY_JOURNAL).unwrap().version, 3);
}
