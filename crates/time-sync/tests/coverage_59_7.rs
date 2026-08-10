use synos_time_sync::{
    ClusterClock, EpochStamp, ManualClock, MonotonicClock, MonotonicEpochCounter, PtpDaemon,
    PtpMessage, PtpRole,
    PtpTimestamp, SyncError, SyncMeasurement,
};

#[test]
fn manual_clock_is_injectable_and_never_moves_backwards() {
    let clock = ManualClock::new(10);
    assert_eq!(clock.now_us(), 10);
    clock.advance_us(5);
    assert_eq!(clock.now_us(), 15);
    clock.set_us(3);
    assert_eq!(clock.now_us(), 15);
    assert_eq!(MonotonicClock::now_us(&clock), 15);
}

#[test]
fn ptp_wire_round_trip_and_timestamp_bounds() {
    let timestamp = PtpTimestamp::new(3, 400).unwrap();
    let message = PtpMessage::Sync {
        sequence_id: 7,
        source: 1,
        target: 2,
        origin_timestamp: timestamp,
        correction_ns: -4,
    };
    let mut wire = [0; synos_time_sync::PTP_PACKET_BYTES];
    assert_eq!(message.encode(&mut wire).unwrap(), synos_time_sync::PTP_PACKET_BYTES);
    assert_eq!(PtpMessage::decode(&wire).unwrap(), message);
    wire[0] = b'X';
    assert_eq!(PtpMessage::decode(&wire), Err(SyncError::InvalidPacket));
    assert_eq!(PtpTimestamp::new(0, 1_000_000_000), Err(SyncError::InvalidTimestamp));
    assert_eq!(timestamp.to_nanos().unwrap(), 3_000_000_400);
}

#[test]
fn clock_never_moves_global_time_backwards() {
    let mut clock = ClusterClock::new();
    let adjustment = clock.discipline(
        SyncMeasurement {
            sequence_id: 1,
            master: 1,
            offset_ns: 10_000,
            path_delay_ns: 20,
        },
        100,
    );
    assert!(adjustment.stepped);
    let first = clock.corrected_time(100);
    let second = clock.corrected_time(90);
    assert!(second > first);
    assert!(clock.is_synchronized());
}

#[test]
fn daemon_rejects_wrong_role_and_handles_two_step_exchange() {
    let mut master = PtpDaemon::new(1, PtpRole::Master).unwrap();
    let mut slave = PtpDaemon::new(2, PtpRole::Slave).unwrap();
    let sync = master.sync_two_step(2).unwrap();
    let follow_up = master.follow_up(sync, 1_000).unwrap();
    slave.receive_sync(sync, 1_100).unwrap();
    slave.receive_follow_up(follow_up).unwrap();
    let request = slave.delay_request(1_200).unwrap();
    let response = master.receive_delay_request(request, 1_300).unwrap();
    let measurement = slave.receive_delay_response(response, 1_400).unwrap();
    assert_eq!(measurement.sequence_id, sync.sequence_id());
    assert!(slave.clock().synchronized);
    assert_eq!(master.sync(1, 0), Err(SyncError::InvalidNode));
    assert!(matches!(PtpDaemon::new(0, PtpRole::Master), Err(SyncError::InvalidNode)));
}

#[test]
fn monotonic_epoch_counter_fences_invalid_stamps() {
    let mut counter = MonotonicEpochCounter::new(2, 10).unwrap();
    let first = counter.issue(10).unwrap();
    let second = counter.issue(1).unwrap();
    assert!(second.counter > first.counter);
    counter.observe(EpochStamp { epoch: 4, counter: 99, node: 1 }).unwrap();
    assert_eq!(counter.current().epoch, 4);
    assert_eq!(counter.observe(EpochStamp { epoch: 5, counter: 0, node: 0 }), Err(SyncError::InvalidNode));
}
