// Inventory: coverage_59_9.rs (legacy roadmap section 59).
use ghostos_init::{CrashReason, ProcessId};
use ghostos_replay::{
    EVENT_BYTES, FlightRecorderRequest, ReplayBundle, ReplayBundleEventKind, ReplayError,
    ReplayEvent, ReplayEventKind, ReplayLog, ReplayMode, ReplayRing, ReplaySensitivity,
    TimeTravel, REPLAY_BUNDLE_EVENT_BYTES, REPLAY_BUNDLE_HEADER_BYTES,
};
use ghostos_ghostfs::{RmsMapHandle, SynFs};

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
    let mut filesystem = SynFs::<64>::new();
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

#[test]
fn replay_bundle_round_trips_all_deterministic_inputs_without_secrets() {
    let mut bundle = ReplayBundle::new([7; 32], 0x1234, 900);
    bundle.record_input(1, 10, b"public input", ReplaySensitivity::Public).unwrap();
    bundle.record_clock(2, 901).unwrap();
    bundle.record_random_seed(3, 11, 0x55).unwrap();
    bundle
        .record_device_completion(4, 12, 0x99, b"completion", ReplaySensitivity::Public)
        .unwrap();
    bundle.record_scheduler_decision(5, 0, 0b101, 42).unwrap();
    assert_eq!(
        bundle.record_input(6, 10, b"secret", ReplaySensitivity::Secret),
        Err(ReplayError::SecretExcluded)
    );

    let mut encoded = [0; REPLAY_BUNDLE_HEADER_BYTES + 5 * REPLAY_BUNDLE_EVENT_BYTES];
    let encoded_len = bundle.encode(&mut encoded).unwrap();
    let decoded = ReplayBundle::decode(&encoded[..encoded_len]).unwrap();
    assert_eq!(decoded.config_digest(), &[7; 32]);
    assert_eq!(decoded.random_seed(), 0x1234);
    assert_eq!(decoded.initial_clock_ns(), 900);

    let mut replay = decoded.replay();
    assert_eq!(replay.next_input(10).unwrap().payload(), b"public input");
    assert_eq!(replay.clock().unwrap(), 901);
    assert_eq!(replay.random_seed(11).unwrap(), 0x55);
    assert_eq!(
        replay.device_completion(12).unwrap().payload(),
        b"completion"
    );
    assert_eq!(replay.scheduler_decision(0).unwrap(), (0b101, 42));
    assert_eq!(replay.pending(), 0);
    assert_eq!(decoded.events()[0].kind, ReplayBundleEventKind::Input);
}

#[test]
fn replay_bundle_rejects_divergent_public_input_and_corrupt_padding() {
    let mut bundle = ReplayBundle::new([0; 32], 1, 2);
    bundle
        .record_input(1, 1, b"expected", ReplaySensitivity::Public)
        .unwrap();
    let mut encoded = [0; REPLAY_BUNDLE_HEADER_BYTES + REPLAY_BUNDLE_EVENT_BYTES];
    bundle.encode(&mut encoded).unwrap();
    let decoded = ReplayBundle::decode(&encoded).unwrap();
    let mut replay = decoded.replay();
    assert_eq!(replay.input(1, b"different"), Err(ReplayError::InputMismatch));

    encoded[REPLAY_BUNDLE_HEADER_BYTES + REPLAY_BUNDLE_EVENT_BYTES - 1] = 1;
    assert_eq!(ReplayBundle::decode(&encoded), Err(ReplayError::Corrupt));
}
