use synos_netd::{
    CaptureDirection, CaptureKind, CapabilityKey, CapabilityRight, Direction, Firewall,
    FirewallDecision, FirewallError,
    FirewallPolicy, FirewallRule, Ipv4Cidr, NetworkCapability, PacketContext, PacketError,
    PacketCapture, PacketQueue, PacketView, PortRange, Protocol, RateLimit, RuleAction, SocketOperation,
    SocketRequest, SocketRights,
};

fn udp_frame(payload: &[u8]) -> [u8; 64] {
    let total = 14 + 20 + 8 + payload.len();
    let mut frame = [0; 64];
    frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    frame[14] = 0x45;
    frame[16..18].copy_from_slice(&(total as u16 - 14).to_be_bytes());
    frame[23] = 17;
    frame[26..30].copy_from_slice(&[10, 0, 0, 1]);
    frame[30..34].copy_from_slice(&[20, 0, 0, 1]);
    frame[34..36].copy_from_slice(&0x1234u16.to_be_bytes());
    frame[36..38].copy_from_slice(&8080u16.to_be_bytes());
    frame[38..40].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    frame[42..42 + payload.len()].copy_from_slice(payload);
    frame
}

#[test]
fn packet_queue_is_bounded_and_zero_copy() {
    let mut queue = PacketQueue::<1, 8>::new();
    let mut writer = queue.reserve().expect("one slot");
    writer.buffer()[..3].copy_from_slice(b"abc");
    writer.commit(3).expect("frame fits");
    assert_eq!(queue.pending(), 1);
    assert_eq!(queue.reserve().err(), Some(PacketError::Full));
    let reader = queue.dequeue().expect("ready frame");
    assert_eq!(reader.frame(), b"abc");
    drop(reader);
    assert_eq!(queue.available(), 1);

    let writer = queue.reserve().expect("slot returned");
    assert_eq!(writer.capacity(), 8);
    assert_eq!(writer.commit(9), Err(PacketError::FrameTooLarge));
    assert_eq!(queue.pending(), 0);
}

#[test]
fn firewall_parses_udp_and_rejects_bad_frames() {
    let frame = udp_frame(b"hello");
    let packet = PacketView::parse(&frame).expect("valid IPv4 UDP");
    assert_eq!(packet.source, [10, 0, 0, 1]);
    assert_eq!(packet.destination, [20, 0, 0, 1]);
    assert_eq!(packet.destination_port, 8080);
    assert_eq!(packet.payload, b"hello");
    assert_eq!(PacketView::parse(&frame[..20]), Err(FirewallError::InvalidFrame));

    let mut policy = FirewallPolicy::<4>::new();
    policy.default_action = RuleAction::Drop;
    policy
        .add_rule(FirewallRule {
            direction: Some(Direction::Ingress),
            protocol: Some(Protocol::Udp),
            source: None,
            destination: Some(Ipv4Cidr::new([20, 0, 0, 0], 24).unwrap()),
            source_ports: None,
            destination_ports: Some(PortRange::new(8080, 8080).unwrap()),
            action: RuleAction::Allow,
            stateful: false,
            capability: None,
            rate_limit: Some(RateLimit::new(1, 1_000).unwrap()),
        })
        .unwrap();
    let mut firewall = Firewall::<4, 4, 4>::new(policy);
    let context = PacketContext {
        principal: 7,
        direction: Direction::Ingress,
        now_ms: 10,
        leased_workload: false,
        capability: None,
        signature: None,
        signer: None,
    };
    assert_eq!(firewall.inspect(&frame, context), FirewallDecision::Allow);
    assert_eq!(firewall.inspect(&frame, PacketContext { now_ms: 11, ..context }), FirewallDecision::RateLimited);
    assert_eq!(firewall.inspect(&frame[..30], context), FirewallDecision::Invalid);
}

#[test]
fn packet_capture_preserves_frame_decision_order_and_bounds() {
    let frame = udp_frame(b"evidence");
    let mut policy = FirewallPolicy::<2>::new();
    policy.default_action = RuleAction::Allow;
    let mut firewall = Firewall::<2, 2, 2>::new(policy);
    let context = PacketContext {
        principal: 7,
        direction: Direction::Ingress,
        now_ms: 42,
        leased_workload: false,
        capability: None,
        signature: None,
        signer: None,
    };
    let mut capture = PacketCapture::<4>::new();

    assert_eq!(
        firewall.inspect_with_capture(&frame, context, &mut capture),
        FirewallDecision::Allow
    );
    assert_eq!(capture.len(), 2);
    assert_eq!(capture.records()[0].kind, CaptureKind::Packet);
    assert_eq!(capture.records()[0].direction, Some(CaptureDirection::Ingress));
    assert_eq!(capture.records()[0].bytes(), &frame);
    assert_eq!(capture.records()[1].kind, CaptureKind::FirewallDecision);
    assert_eq!(capture.records()[1].code, 0);

    capture.record_packet(
        43,
        CaptureDirection::Egress,
        [0; 6],
        [0; 6],
        [0; 4],
        [0; 4],
        0,
        0,
        &[1; synos_netd::MAX_CAPTURE_BYTES + 1],
    );
    assert!(capture.records()[2].truncated);
    assert_eq!(capture.records()[2].original_length, synos_netd::MAX_CAPTURE_BYTES + 1);
    capture.record_event(44, CaptureKind::Link, 0);
    capture.record_event(45, CaptureKind::Rollback, 0);
    assert_eq!(capture.len(), 4);
    capture.record_event(46, CaptureKind::NetworkCommand, 64);
    assert_eq!(capture.dropped(), 1);
}

#[test]
fn firewall_policy_and_capability_expiry_are_wire_safe() {
    let key = CapabilityKey::new([9; 32]);
    let destination = Ipv4Cidr::new([192, 168, 1, 0], 24).unwrap();
    let capability = NetworkCapability::issue(
        &key,
        42,
        CapabilityRight::Connect as u8,
        PortRange::new(443, 443).unwrap(),
        destination,
        100,
        1,
    )
    .unwrap();
    assert!(capability
        .verify(&key, 42, CapabilityRight::Connect, [192, 168, 1, 8], 443, 99)
        .is_ok());
    assert_eq!(
        capability.verify(&key, 42, CapabilityRight::Connect, [192, 168, 1, 8], 443, 100),
        Err(FirewallError::ExpiredCapability)
    );

    let mut policy = FirewallPolicy::<2>::new();
    policy.add_rule(FirewallRule::allow()).unwrap();
    let mut image = [0; 104];
    let length = policy.encode(&mut image).unwrap();
    let decoded = FirewallPolicy::<2>::decode(&image[..length]).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded.default_action, RuleAction::Drop);
}

#[test]
fn socket_request_rejects_bad_schema_and_accepts_open() {
    let envelope = SocketRequest::open(99, SocketRights::CONNECT);
    let request = SocketRequest::decode(envelope).unwrap();
    assert_eq!(request.correlation, 99);
    assert_eq!(request.operation, SocketOperation::OpenTcp);
    assert_eq!(request.argument0, SocketRights::CONNECT.bits() as u64);

    let mut bad = envelope;
    bad.label = 0;
    assert!(SocketRequest::decode(bad).is_err());
}
