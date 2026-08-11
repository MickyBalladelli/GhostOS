use synos_observability::{
    AuditQuery, CodecError, CorrelationId, EventField, EventKind, FieldKind, JOURNAL_RECORD_SIZE,
    Alert, AlertLevel, AlertRegistry, AuditJournal, AuditJournalError, AuditKey, Level,
    HealthError, HealthReport, HealthState, HealthTransport, MetricError, MetricKind,
    MetricRegistry, MetricSample, OperationalHealth, QueryError, TelemetryDimensions,
    MetricRecordOutcome, TenantDiagnosticKind, TenantDiagnosticOutcome, TenantDiagnostics,
    TraceEvent, TraceRing, analyze_audit, decode_record, encode_record, field,
    parse_audit_command,
};
use synos_status::Status;

fn audit(timestamp: u64, node: u32, capability: u64, status: Status) -> TraceEvent {
    TraceEvent::new(Level::Info, EventKind::Audit)
        .at(timestamp)
        .on_node(node)
        .correlated(CorrelationId::from_raw(timestamp as u128))
        .with_field(EventField::unsigned(field::CAPABILITY, capability))
        .with_field(EventField::status(status))
        .with_field(EventField::unsigned(field::OPERATION, timestamp))
}

#[test]
fn records_are_bounded_round_trip_and_reject_corruption() {
    let event = TraceEvent::new(Level::Warn, EventKind::Authentication)
        .at(42)
        .on_node(3)
        .correlated(CorrelationId::from_raw(0xfeed))
        .with_field(EventField::unsigned(field::OBJECT, 7))
        .with_field(EventField::signed(field::LENGTH, -4))
        .with_field(EventField::boolean(field::RIGHTS, true))
        .with_field(EventField::identifier(field::CALLER, 0x1234));
    let bounded = event.with_field(EventField::unsigned(field::OWNER, 99));
    assert_eq!(bounded.fields().count(), 4);

    let mut encoded = [0; JOURNAL_RECORD_SIZE];
    assert_eq!(encode_record(bounded, &mut encoded), Ok(JOURNAL_RECORD_SIZE));
    assert_eq!(decode_record(&encoded), Ok(bounded));
    assert_eq!(bounded.field(field::LENGTH).unwrap().kind, FieldKind::Signed);
    assert_eq!(decode_record(&encoded[..JOURNAL_RECORD_SIZE - 1]), Err(CodecError::BufferTooSmall));

    encoded[20] ^= 1;
    assert_eq!(decode_record(&encoded), Err(CodecError::Checksum));
}

#[test]
fn malformed_record_fields_return_errors() {
    let mut encoded = [0; JOURNAL_RECORD_SIZE];
    encode_record(TraceEvent::new(Level::Info, EventKind::Kernel), &mut encoded).unwrap();
    encoded[7] = 5;
    let checksum = encoded[..120].iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ *byte as u64).wrapping_mul(0x100000001b3)
    });
    encoded[120..128].copy_from_slice(&checksum.to_le_bytes());

    assert_eq!(decode_record(&encoded), Err(CodecError::InvalidField));
}

#[test]
fn ring_overflow_reports_drops_and_audit_query_is_stable() {
    let ring = TraceRing::<2>::new();
    ring.push(TraceEvent::new(Level::Info, EventKind::Kernel).at(1));
    ring.push(TraceEvent::new(Level::Info, EventKind::Kernel).at(2));
    ring.push(TraceEvent::new(Level::Info, EventKind::Kernel).at(3));
    assert_eq!(ring.pending(), 2);
    assert_eq!(ring.dropped(), 1);
    assert_eq!(ring.try_pop().unwrap().timestamp, 2);
    assert_eq!(ring.try_pop().unwrap().timestamp, 3);
    assert!(ring.try_pop().is_none());

    let first = audit(10, 2, 8, Status::NORMAL);
    let second = audit(20, 2, 9, Status::BUSY);
    let mut journal = [0; JOURNAL_RECORD_SIZE * 2];
    encode_record(first, &mut journal[..JOURNAL_RECORD_SIZE]).unwrap();
    encode_record(second, &mut journal[JOURNAL_RECORD_SIZE..]).unwrap();
    let query = parse_audit_command(["--since=10", "/before=20", "/node=2", "/capability=8"]).unwrap();
    let mut seen = Vec::new();
    assert_eq!(analyze_audit(&journal, query, |event| seen.push(event.timestamp)), Ok(1));
    assert_eq!(seen, vec![10]);
    assert_eq!(parse_audit_command(["/unknown=1"]), Err(QueryError::InvalidArgument));
    assert_eq!(analyze_audit(&journal[..journal.len() - 1], AuditQuery::default(), |_| {}), Err(QueryError::TrailingBytes));
}

#[test]
fn durable_audit_and_observability_buffers_have_explicit_full_behavior() {
    let mut journal = AuditJournal::<1>::new(AuditKey::new([7; 32]).unwrap()).unwrap();
    journal.append(audit(1, 1, 1, Status::NORMAL)).unwrap();
    assert_eq!(
        journal.append(audit(2, 1, 2, Status::BUSY)),
        Err(AuditJournalError::Capacity)
    );
    assert_eq!(journal.dropped(), 1);

    let dimensions = TelemetryDimensions::new(1, 1, 0, 0, 0);
    let mut metrics = MetricRegistry::<1>::new();
    metrics
        .record(MetricSample::new(1, MetricKind::Gauge, 1, 10, dimensions))
        .unwrap();
    assert_eq!(
        metrics.record(MetricSample::new(2, MetricKind::Gauge, 2, 20, dimensions)),
        Err(MetricError::Capacity)
    );
    assert_eq!(metrics.dropped(), 1);

    let mut alerts = AlertRegistry::<1>::new();
    let alert = Alert {
        sequence: 0,
        timestamp: 1,
        level: AlertLevel::Warning,
        code: 1,
        dimensions,
        correlation: CorrelationId::from_raw(1),
    };
    alerts.push(alert).unwrap();
    assert_eq!(alerts.push(alert), Err(MetricError::Capacity));
    assert_eq!(alerts.dropped(), 1);
}

#[test]
fn logs_and_audit_exports_do_not_carry_free_form_payloads_or_keys() {
    let secret = b"operator-secret-log-payload";
    let event = TraceEvent::new(Level::Info, EventKind::Audit)
        .at(7)
        .with_field(EventField::unsigned(field::OBJECT, 0x4455));
    let mut record = [0; JOURNAL_RECORD_SIZE];
    encode_record(event, &mut record).unwrap();
    assert!(!record.windows(secret.len()).any(|window| window == secret));
    assert!(event.fields().all(|field| {
        matches!(
            field.kind,
            FieldKind::Unsigned
                | FieldKind::Signed
                | FieldKind::Boolean
                | FieldKind::Identifier
                | FieldKind::Status
        )
    }));

    let key_bytes = [0x5a; 32];
    let key = AuditKey::from_bytes(key_bytes);
    let mut journal = AuditJournal::<1>::new(key).unwrap();
    journal.append(event).unwrap();
    let mut export = vec![0; AuditJournal::<1>::encoded_len()];
    journal.export(&mut export).unwrap();
    assert_eq!(&export[32..64], &key.id());
    assert!(!export.windows(key_bytes.len()).any(|window| window == key_bytes));
}

#[test]
fn operational_health_is_bounded_and_aggregates_transport_failures() {
    let healthy = OperationalHealth::new(
        10,
        1,
        HealthTransport::Http,
        HealthState::Healthy,
        2,
        8,
        3,
        4,
        false,
    )
    .unwrap();
    let degraded = OperationalHealth::new(
        11,
        1,
        HealthTransport::Grpc,
        HealthState::Degraded,
        5,
        8,
        7,
        9,
        true,
    )
    .unwrap();
    let failed = OperationalHealth::new(
        12,
        2,
        HealthTransport::Mesh,
        HealthState::Failed,
        1,
        4,
        11,
        13,
        false,
    )
    .unwrap();

    let mut report = HealthReport::<2>::new();
    report.push(healthy).unwrap();
    report.push(degraded).unwrap();
    assert_eq!(report.push(failed), Err(HealthError::Capacity));
    assert_eq!(report.push(degraded), Err(HealthError::Duplicate));
    assert_eq!(report.sampled_at_us(), 11);
    assert_eq!(report.healthy_count(), 1);
    assert_eq!(report.degraded_count(), 1);
    assert_eq!(report.failed_count(), 0);
    assert_eq!(report.queue_depth(), 7);
    assert_eq!(report.queue_capacity(), 16);
    assert_eq!(report.dropped_packets(), 10);
    assert_eq!(report.retries(), 13);
    assert!(report.degraded_mode());
    assert_eq!(report.status(), Status::CLUSTER_DEGRADED);
    assert_eq!(
        OperationalHealth::new(0, 1, HealthTransport::Packet, HealthState::Healthy, 0, 1, 0, 0, false),
        None
    );
}

#[test]
fn cardinality_overflow_is_folded_and_visible() {
    let dimensions = TelemetryDimensions::new(1, 1, 0, 1, 1);
    let mut metrics = MetricRegistry::<1>::new();
    assert_eq!(
        metrics.record_bounded(MetricSample::new(1, MetricKind::Counter, 1, 2, dimensions)),
        Ok(MetricRecordOutcome::NewSeries)
    );
    assert_eq!(
        metrics.record_bounded(MetricSample::new(2, MetricKind::Counter, 2, 3, dimensions)),
        Ok(MetricRecordOutcome::Aggregated)
    );
    assert_eq!(metrics.aggregates().next().unwrap().value, 3);

    let mut labels = synos_observability::AuditLabelAggregator::<1>::new();
    labels.observe(audit(1, 1, 1, Status::NORMAL));
    labels.observe(audit(2, 1, 2, Status::NORMAL));
    assert_eq!(labels.snapshot().overflow, 5);

    let mut tenants = TenantDiagnostics::<1>::new();
    assert_eq!(
        tenants.record(1, TenantDiagnosticKind::Trace, 2),
        Ok(TenantDiagnosticOutcome::Exact)
    );
    assert_eq!(
        tenants.record(2, TenantDiagnosticKind::Trace, 4),
        Ok(TenantDiagnosticOutcome::Aggregated)
    );
    assert_eq!(tenants.other().trace_events, 4);
    assert_eq!(tenants.other().aggregated, 1);
}
