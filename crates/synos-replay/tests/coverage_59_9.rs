use synos_init::{CrashReason, ProcessId};
use synos_replay::{
    EVENT_BYTES, FlightRecorderRequest, ReplayError, ReplayEvent, ReplayEventKind, ReplayLog,
    ReplayMode, ReplayRing, TimeTravel,
};
use synos_synfs::{RmsMapHandle, SynFs};

#[test]
fn replay_is_deterministic_and_detects_divergent_inputs() {
    let log = ReplayLog::<2>::new();
    assert_eq!(log.record(ReplayEvent::timing(1, 0, 2, 3)), Err(ReplayError::Inactive));
    log.begin_recording();
    assert_eq!(log.record(ReplayEvent::timing(1, 0, 2, 3)), Ok(1));
    assert_eq!(log.record(ReplayEvent::network_interrupt(2, 4, 5, 6)), Ok(2));
    log.begin_replay();
    assert_eq!(log.next(ReplayEventKind::Timing).unwrap().timestamp_us, 1);
    assert_eq!(log.next(ReplayEventKind::CxlMemoryAccess), Err(ReplayError::UnexpectedEvent));
    assert!(matches!(log.next(ReplayEventKind::NetworkInterrupt), Err(ReplayError::NoEvent)));
}

#[test]
fn replay_ring_and_time_travel_are_bounded() {
    let ring = ReplayRing::<2>::new();
    ring.push(ReplayEvent::timing(1, 0, 0, 0));
    ring.push(ReplayEvent::timing(2, 0, 0, 0));
    ring.push(ReplayEvent::timing(3, 0, 0, 0));
    assert_eq!(ring.dropped(), 1);
    let mut events = [ReplayEvent::EMPTY; 2];
    assert_eq!(ring.snapshot_into(&mut events).unwrap(), 2);
    assert_eq!(events[0].timestamp_us, 2);
    assert_eq!(events[1].timestamp_us, 3);

    let mut history = TimeTravel::<u64, 2>::new();
    let first = history.checkpoint(10, 100).unwrap();
    let second = history.checkpoint(20, 200).unwrap();
    assert_eq!(history.checkpoint(30, 300), Err(ReplayError::Capacity));
    assert_eq!(history.nearest_before(19).unwrap().1, 100);
    assert_eq!(history.restore(second.id).unwrap().1, 200);
    history.release(first.id).unwrap();
    assert!(history.restore(first.id).is_err());
}

#[test]
fn crash_flight_recorder_preserves_bounded_evidence() {
    let log = ReplayLog::<2>::new();
    log.begin_recording();
    log.record(ReplayEvent::timing(11, 1, 2, 3)).unwrap();
    let process = ProcessId::new(8).unwrap();
    let mut filesystem = SynFs::<128>::new();
    let receipt = log
        .preserve_on_crash(
            &mut filesystem,
            FlightRecorderRequest {
                process,
                reason: CrashReason::Watchdog,
                timestamp_us: 99,
                directory: "/replay/8",
            },
        )
        .unwrap();
    assert_eq!(receipt.events, 1);
    assert_eq!(receipt.bytes, 40 + EVENT_BYTES as u64);
    let mut metadata = [0; 40];
    filesystem.read("/replay/8/META", &mut metadata).unwrap();
    assert_eq!(&metadata[..8], b"SYNREP01");
    assert_eq!(u16::from_le_bytes(metadata[8..10].try_into().unwrap()), 1);
    let mut event = [0; EVENT_BYTES];
    filesystem.read("/replay/8/EVENT-00000000", &mut event).unwrap();
    assert_eq!(ReplayEvent::decode(&event).unwrap().timestamp_us, 11);
    assert_eq!(RmsMapHandle::from_capability(1), None);
    assert_eq!(log.mode(), ReplayMode::Recording);
}
