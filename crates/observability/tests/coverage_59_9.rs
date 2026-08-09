use synos_observability::{
    AuditQuery, CodecError, CorrelationId, EventField, EventKind, FieldKind, JOURNAL_RECORD_SIZE,
    Level, QueryError, TraceEvent, TraceRing, analyze_audit, decode_record, encode_record, field,
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
