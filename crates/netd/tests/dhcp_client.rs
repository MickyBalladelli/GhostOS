use synos_netd::{
    dhcp_client_firewall_rules, format_ipv4, install_dhcp_client_rules, CaptureDirection,
    CaptureKind, CapabilityRight, CapturingDhcpTransport, DHCP_CLIENT_PORT, DHCP_SERVER_PORT,
    DhcpClient, DhcpClientState, DhcpError, DhcpLease,
    DhcpLeaseRuntime, DhcpServerFixture, DhcpTransport, Direction, Firewall, FirewallDecision,
    FirewallPolicy, PacketContext, Protocol, MAX_DHCP_PACKET, StaticSnapshot, BACKOFF_MS,
    MAX_DISCOVER_ATTEMPTS,
};

const MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];

struct CaptureTransport {
    packets: [[u8; MAX_DHCP_PACKET]; 8],
    lengths: [usize; 8],
    count: usize,
    interface: [u8; 16],
    interface_len: usize,
}

impl CaptureTransport {
    fn new() -> Self {
        Self {
            packets: [[0; MAX_DHCP_PACKET]; 8],
            lengths: [0; 8],
            count: 0,
            interface: [0; 16],
            interface_len: 0,
        }
    }

    fn last(&self) -> &[u8] {
        let index = self.count - 1;
        &self.packets[index][..self.lengths[index]]
    }
}

impl DhcpTransport for CaptureTransport {
    fn send_udp(
        &mut self,
        interface: &str,
        _src_mac: [u8; 6],
        _src_ip: [u8; 4],
        _dst_ip: [u8; 4],
        _dst_mac: [u8; 6],
        _src_port: u16,
        _dst_port: u16,
        payload: &[u8],
    ) -> Result<(), DhcpError> {
        if self.count == self.packets.len() || payload.len() > MAX_DHCP_PACKET {
            return Err(DhcpError::Capacity);
        }
        self.interface[..interface.len()].copy_from_slice(interface.as_bytes());
        self.interface_len = interface.len();
        self.packets[self.count][..payload.len()].copy_from_slice(payload);
        self.lengths[self.count] = payload.len();
        self.count += 1;
        Ok(())
    }
}

struct RecordingRuntime {
    applied: Option<DhcpLease>,
    restored: Option<StaticSnapshot>,
    events: u8,
}

impl RecordingRuntime {
    fn new() -> Self {
        Self {
            applied: None,
            restored: None,
            events: 0,
        }
    }
}

impl DhcpLeaseRuntime for RecordingRuntime {
    fn apply_lease(&mut self, interface: &str, lease: &DhcpLease) -> Result<(), DhcpError> {
        if interface != "eth0" {
            return Err(DhcpError::InvalidInterface);
        }
        self.applied = Some(*lease);
        self.events = self.events.saturating_add(1);
        Ok(())
    }

    fn restore_static(&mut self, interface: &str, snapshot: &StaticSnapshot) -> Result<(), DhcpError> {
        if interface != "eth0" {
            return Err(DhcpError::InvalidInterface);
        }
        self.restored = Some(*snapshot);
        self.events = self.events.saturating_add(1);
        Ok(())
    }
}

fn fixture() -> DhcpServerFixture {
    DhcpServerFixture {
        server_id: [10, 0, 0, 1],
        offered_ip: [10, 0, 0, 50],
        subnet_mask: [255, 255, 255, 0],
        gateway: [10, 0, 0, 1],
        dns: [10, 0, 0, 53],
        lease_time_secs: 100,
        t1_secs: 50,
        t2_secs: 87,
        mac: MAC,
    }
}

fn authorized_client() -> DhcpClient {
    let mut client = DhcpClient::new("eth0", MAC, 0xA5A5_A5A5).unwrap();
    client
        .authorize(CapabilityRight::Raw as u8 | CapabilityRight::Ingress as u8)
        .unwrap();
    client.preserve_static(StaticSnapshot {
        address: [10, 0, 0, 2],
        gateway: Some([10, 0, 0, 1]),
        subnet_mask: Some([255, 255, 255, 0]),
    });
    client
}

#[test]
fn dhcp_requires_capability_and_binds_interface() {
    let mut client = DhcpClient::new("eth0", MAC, 1).unwrap();
    assert_eq!(client.start(0), Err(DhcpError::AccessDenied));
    assert_eq!(
        client.authorize(CapabilityRight::Connect as u8),
        Err(DhcpError::AccessDenied)
    );
    client
        .authorize(CapabilityRight::Raw as u8 | CapabilityRight::Ingress as u8)
        .unwrap();
    assert_eq!(client.interface_name(), "eth0");
    client.start(0).unwrap();
    assert_eq!(client.state(), DhcpClientState::Init);
}

#[test]
fn dhcp_admin_and_carrier_gates_stop_and_restart_transmission() {
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();

    client.start(0).unwrap();
    client.poll(0, &mut transport, &mut runtime).unwrap();
    assert_eq!(transport.count, 1);

    client.set_enabled(false, 1);
    assert!(!client.view().enabled);
    client.poll(1_000, &mut transport, &mut runtime).unwrap();
    assert_eq!(transport.count, 1);

    client.set_enabled(true, 2);
    client.set_link(false, 3);
    client.poll(2_000, &mut transport, &mut runtime).unwrap();
    assert_eq!(transport.count, 1);

    client.set_link(true, 4);
    client.poll(4, &mut transport, &mut runtime).unwrap();
    assert_eq!(transport.count, 2);
    assert!(client.view().enabled);
    assert!(client.view().link_up);
}

#[test]
fn dora_assigns_lease_atomically_and_exposes_state() {
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    let server = fixture();

    client.start(0).unwrap();
    client.poll(0, &mut transport, &mut runtime).unwrap();
    assert_eq!(client.state(), DhcpClientState::Selecting);
    assert_eq!(
        core::str::from_utf8(&transport.interface[..transport.interface_len]).unwrap(),
        "eth0"
    );

    let offer = server.respond(transport.last()).unwrap();
    client
        .handle_packet(&offer, 10, &mut runtime)
        .unwrap();
    assert_eq!(client.state(), DhcpClientState::Requesting);

    client.poll(10, &mut transport, &mut runtime).unwrap();
    let ack = server.respond(transport.last()).unwrap();
    client.handle_packet(&ack, 20, &mut runtime).unwrap();

    assert_eq!(client.state(), DhcpClientState::Bound);
    let lease = client.lease().unwrap();
    assert_eq!(lease.address, [10, 0, 0, 50]);
    assert_eq!(lease.server_id, [10, 0, 0, 1]);
    assert_eq!(lease.subnet_mask, [255, 255, 255, 0]);
    assert_eq!(lease.gateway, Some([10, 0, 0, 1]));
    assert_eq!(lease.dns_count, 1);
    assert_eq!(runtime.applied, Some(lease));
    assert_eq!(client.view().state, DhcpClientState::Bound);
}

#[test]
fn dhcp_capture_records_dora_and_link_recovery_evidence() {
    let mut client = authorized_client();
    let mut transport: CapturingDhcpTransport<CaptureTransport, 16> =
        CapturingDhcpTransport::new(CaptureTransport::new());
    let mut runtime = RecordingRuntime::new();
    let server = fixture();

    client.start(0).unwrap();
    transport.set_timestamp(0);
    client.poll(0, &mut transport, &mut runtime).unwrap();
    transport.capture.record_event(0, CaptureKind::DhcpLifecycle, 1);
    let offer = server.respond(transport.transport.last()).unwrap();
    transport.capture.record_packet(
        1,
        CaptureDirection::Ingress,
        [0; 6],
        MAC,
        server.server_id,
        [255, 255, 255, 255],
        DHCP_SERVER_PORT,
        DHCP_CLIENT_PORT,
        &offer,
    );
    transport.capture.record_event(1, CaptureKind::DhcpLifecycle, 2);
    client.handle_packet(&offer, 1, &mut runtime).unwrap();

    transport.set_timestamp(1);
    client.poll(1, &mut transport, &mut runtime).unwrap();
    transport.capture.record_event(1, CaptureKind::DhcpLifecycle, 3);
    let ack = server.respond(transport.transport.last()).unwrap();
    transport.capture.record_packet(
        2,
        CaptureDirection::Ingress,
        [0; 6],
        MAC,
        server.server_id,
        [255, 255, 255, 255],
        DHCP_SERVER_PORT,
        DHCP_CLIENT_PORT,
        &ack,
    );
    transport.capture.record_event(2, CaptureKind::DhcpLifecycle, 5);
    client.handle_packet(&ack, 2, &mut runtime).unwrap();

    client.set_link(false, 3);
    transport.capture.record_event(3, CaptureKind::Link, 0);
    assert_eq!(transport.capture.records().iter().filter(|record| {
        record.kind == CaptureKind::Packet && record.direction == Some(CaptureDirection::Egress)
    }).count(), 2);
    assert_eq!(transport.capture.records().iter().filter(|record| {
        record.kind == CaptureKind::Packet && record.direction == Some(CaptureDirection::Ingress)
    }).count(), 2);
    assert_eq!(transport.capture.records().last().unwrap().kind, CaptureKind::Link);
    assert!(transport.capture.records().windows(2).all(|records| {
        records[0].sequence < records[1].sequence
    }));

    client.set_link(true, 4);
    transport.capture.record_event(4, CaptureKind::Link, 1);
    transport.set_timestamp(4);
    client.poll(4, &mut transport, &mut runtime).unwrap();
    let reboot_ack = server.respond(transport.transport.last()).unwrap();
    transport.capture.record_packet(
        5,
        CaptureDirection::Ingress,
        [0; 6],
        MAC,
        server.server_id,
        [255, 255, 255, 255],
        DHCP_SERVER_PORT,
        DHCP_CLIENT_PORT,
        &reboot_ack,
    );
    client.handle_packet(&reboot_ack, 5, &mut runtime).unwrap();
    assert_eq!(client.state(), DhcpClientState::Bound);
    assert_eq!(transport.capture.records().last().unwrap().kind, CaptureKind::Packet);
}

#[test]
fn malformed_packets_and_conflicting_offers_are_rejected() {
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    let server = fixture();
    client.start(0).unwrap();
    client.poll(0, &mut transport, &mut runtime).unwrap();

    assert_eq!(
        client.handle_packet(&[0; 16], 1, &mut runtime),
        Err(DhcpError::InvalidPacket)
    );

    let mut bad_xid = server.respond(transport.last()).unwrap();
    bad_xid[4] ^= 0xff;
    assert_eq!(
        client.handle_packet(&bad_xid, 1, &mut runtime),
        Err(DhcpError::InvalidPacket)
    );

    let offer = server.respond(transport.last()).unwrap();
    client.handle_packet(&offer, 1, &mut runtime).unwrap();

    let mut conflict = fixture();
    conflict.server_id = [10, 0, 0, 9];
    conflict.offered_ip = [10, 0, 0, 99];
    // Rebuild a discover-shaped request for the conflicting server using the captured packet.
    let conflicting = conflict.respond(transport.packets[0][..transport.lengths[0]].as_ref()).unwrap();
    assert_eq!(
        client.handle_packet(&conflicting, 2, &mut runtime),
        Err(DhcpError::ConflictingOffer)
    );
}

fn bind_lease(
    client: &mut DhcpClient,
    transport: &mut CaptureTransport,
    runtime: &mut RecordingRuntime,
    server: &DhcpServerFixture,
    now_ms: u64,
) {
    client.start(now_ms).unwrap();
    client.poll(now_ms, transport, runtime).unwrap();
    let offer = server.respond(transport.last()).unwrap();
    client.handle_packet(&offer, now_ms, runtime).unwrap();
    client.poll(now_ms, transport, runtime).unwrap();
    let ack = server.respond(transport.last()).unwrap();
    client.handle_packet(&ack, now_ms, runtime).unwrap();
    assert_eq!(client.state(), DhcpClientState::Bound);
}

#[test]
fn renew_rebind_expiry_and_release_restore_static() {
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    let server = fixture();
    bind_lease(&mut client, &mut transport, &mut runtime, &server, 0);

    // Successful renew at T1.
    client.poll(50_000, &mut transport, &mut runtime).unwrap();
    assert_eq!(client.state(), DhcpClientState::Renewing);
    let renew_ack = server.respond(transport.last()).unwrap();
    client
        .handle_packet(&renew_ack, 50_000, &mut runtime)
        .unwrap();
    assert_eq!(client.state(), DhcpClientState::Bound);

    // Unanswered renew progresses to rebind, then expiry restores static.
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    bind_lease(&mut client, &mut transport, &mut runtime, &server, 0);
    client.poll(50_000, &mut transport, &mut runtime).unwrap();
    assert_eq!(client.state(), DhcpClientState::Renewing);
    client.poll(87_000, &mut transport, &mut runtime).unwrap();
    assert_eq!(client.state(), DhcpClientState::Rebinding);
    client.poll(100_000, &mut transport, &mut runtime).unwrap();
    assert_eq!(client.state(), DhcpClientState::Init);
    assert!(client.lease().is_none());
    assert_eq!(
        runtime.restored,
        Some(StaticSnapshot {
            address: [10, 0, 0, 2],
            gateway: Some([10, 0, 0, 1]),
            subnet_mask: Some([255, 255, 255, 0]),
        })
    );

    // Explicit release restores static.
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    bind_lease(&mut client, &mut transport, &mut runtime, &server, 200_000);
    client
        .release(201_000, &mut transport, &mut runtime)
        .unwrap();
    assert_eq!(client.state(), DhcpClientState::Init);
    assert!(runtime.restored.is_some());
}

#[test]
fn renewal_race_rejects_stale_ack_and_changed_server_identity() {
    let server = fixture();
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    bind_lease(&mut client, &mut transport, &mut runtime, &server, 0);

    client.poll(50_000, &mut transport, &mut runtime).unwrap();
    let stale_ack = server.respond(transport.last()).unwrap();
    client.poll(54_000, &mut transport, &mut runtime).unwrap();
    assert_eq!(
        client.handle_packet(&stale_ack, 54_001, &mut runtime),
        Err(DhcpError::InvalidPacket)
    );
    let fresh_ack = server.respond(transport.last()).unwrap();
    client.handle_packet(&fresh_ack, 54_002, &mut runtime).unwrap();
    assert_eq!(client.state(), DhcpClientState::Bound);

    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    bind_lease(&mut client, &mut transport, &mut runtime, &server, 0);
    client.poll(50_000, &mut transport, &mut runtime).unwrap();
    let mut changed_server = server.respond(transport.last()).unwrap();
    set_option_u32(&mut changed_server, 54, u32::from_be_bytes([10, 0, 0, 9]));
    assert_eq!(
        client.handle_packet(&changed_server, 50_001, &mut runtime),
        Err(DhcpError::ConflictingOffer)
    );
    assert_eq!(client.state(), DhcpClientState::Renewing);
    assert_eq!(runtime.applied.unwrap().server_id, server.server_id);
}

#[test]
fn link_down_and_restart_recover_through_init_reboot() {
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    let server = fixture();
    client.start(0).unwrap();
    client.poll(0, &mut transport, &mut runtime).unwrap();
    let offer = server.respond(transport.last()).unwrap();
    client.handle_packet(&offer, 0, &mut runtime).unwrap();
    client.poll(0, &mut transport, &mut runtime).unwrap();
    let ack = server.respond(transport.last()).unwrap();
    client.handle_packet(&ack, 0, &mut runtime).unwrap();

    client.set_link(false, 1_000);
    assert!(client.view().next_action_ms.is_none());
    client.set_link(true, 2_000);
    assert_eq!(client.state(), DhcpClientState::InitReboot);
    client.poll(2_000, &mut transport, &mut runtime).unwrap();
    let ack = server.respond(transport.last()).unwrap();
    client
        .handle_packet(&ack, 2_100, &mut runtime)
        .unwrap();
    assert_eq!(client.state(), DhcpClientState::Bound);
}

#[test]
fn unavailable_server_uses_bounded_backoff_and_preserves_static() {
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    client.start(0).unwrap();

    let mut now = 0u64;
    for _ in 0..MAX_DISCOVER_ATTEMPTS {
        let result = client.poll(now, &mut transport, &mut runtime);
        assert!(result.is_ok() || result == Err(DhcpError::ServerUnavailable));
        now = now.saturating_add(BACKOFF_MS[BACKOFF_MS.len() - 1]);
    }
    assert_eq!(
        client.poll(now, &mut transport, &mut runtime),
        Err(DhcpError::ServerUnavailable)
    );
    assert_eq!(
        runtime.restored,
        Some(StaticSnapshot {
            address: [10, 0, 0, 2],
            gateway: Some([10, 0, 0, 1]),
            subnet_mask: Some([255, 255, 255, 0]),
        })
    );
    assert!(runtime.applied.is_none());
}

#[test]
fn nak_rolls_back_to_static_and_firewall_rules_are_capability_gated() {
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    let server = fixture();
    client.start(0).unwrap();
    client.poll(0, &mut transport, &mut runtime).unwrap();
    let offer = server.respond(transport.last()).unwrap();
    client.handle_packet(&offer, 0, &mut runtime).unwrap();
    client.poll(0, &mut transport, &mut runtime).unwrap();
    let nak = server.nak(transport.last()).unwrap();
    client.handle_packet(&nak, 1, &mut runtime).unwrap();
    assert_eq!(client.state(), DhcpClientState::Init);
    assert!(runtime.restored.is_some());

    let mut policy = FirewallPolicy::<8>::new();
    install_dhcp_client_rules(&mut policy).unwrap();
    let rules = dhcp_client_firewall_rules();
    assert_eq!(rules[0].capability, Some(CapabilityRight::Raw));
    assert_eq!(rules[1].capability, Some(CapabilityRight::Ingress));
    assert_eq!(policy.rules().iter().flatten().count(), 2);
}

fn set_option_u32(packet: &mut [u8], code: u8, value: u32) {
    let mut index = 240;
    while index + 2 <= packet.len() {
        let option = packet[index];
        if option == 255 {
            break;
        }
        if option == 0 {
            index += 1;
            continue;
        }
        let length = packet[index + 1] as usize;
        if option == code && length == 4 && index + 2 + length <= packet.len() {
            packet[index + 2..index + 6].copy_from_slice(&value.to_be_bytes());
            return;
        }
        index += 2 + length;
    }
    panic!("option {code} not found");
}

fn remove_option(packet: &mut [u8], code: u8) {
    let mut index = 240;
    while index + 2 <= packet.len() {
        let option = packet[index];
        if option == 255 {
            break;
        }
        if option == 0 {
            index += 1;
            continue;
        }
        let length = packet[index + 1] as usize;
        let end = index + 2 + length;
        if option == code && end <= packet.len() {
            packet.copy_within(end.., index);
            return;
        }
        index = end;
    }
}

fn dhcp_udp_frame(payload: &[u8], source_port: u16, destination_port: u16) -> [u8; 640] {
    let total = 14 + 20 + 8 + payload.len();
    let mut frame = [0; 640];
    frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    frame[14] = 0x45;
    frame[16..18].copy_from_slice(&(total as u16 - 14).to_be_bytes());
    frame[23] = 17;
    frame[26..30].copy_from_slice(&[10, 0, 0, 1]);
    frame[30..34].copy_from_slice(&[255, 255, 255, 255]);
    frame[34..36].copy_from_slice(&source_port.to_be_bytes());
    frame[36..38].copy_from_slice(&destination_port.to_be_bytes());
    frame[38..40].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    frame[42..42 + payload.len()].copy_from_slice(payload);
    frame
}

#[test]
fn rejects_wrong_mac_invalid_lease_options_and_bad_magic() {
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    let server = fixture();
    client.start(0).unwrap();
    client.poll(0, &mut transport, &mut runtime).unwrap();

    let mut wrong_mac = server.respond(transport.last()).unwrap();
    wrong_mac[28] ^= 0xff;
    assert_eq!(
        client.handle_packet(&wrong_mac, 1, &mut runtime),
        Err(DhcpError::InvalidPacket)
    );

    let mut zero_lease = server.respond(transport.last()).unwrap();
    set_option_u32(&mut zero_lease, 51, 0);
    assert_eq!(
        client.handle_packet(&zero_lease, 1, &mut runtime),
        Err(DhcpError::InvalidLease)
    );

    let mut no_server = server.respond(transport.last()).unwrap();
    remove_option(&mut no_server, 54);
    assert_eq!(
        client.handle_packet(&no_server, 1, &mut runtime),
        Err(DhcpError::InvalidLease)
    );

    let mut bad_timers = server.respond(transport.last()).unwrap();
    set_option_u32(&mut bad_timers, 51, 100);
    set_option_u32(&mut bad_timers, 58, 100);
    set_option_u32(&mut bad_timers, 59, 100);
    assert_eq!(
        client.handle_packet(&bad_timers, 1, &mut runtime),
        Err(DhcpError::InvalidLease)
    );

    let mut bad_magic = server.respond(transport.last()).unwrap();
    bad_magic[236] = 0;
    assert_eq!(
        client.handle_packet(&bad_magic, 1, &mut runtime),
        Err(DhcpError::InvalidPacket)
    );
}

#[test]
fn validates_gateway_dns_and_formats_ipv4() {
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    let mut server = fixture();
    server.gateway = [10, 0, 0, 254];
    server.dns = [1, 1, 1, 1];
    bind_lease(&mut client, &mut transport, &mut runtime, &server, 0);
    let lease = client.lease().unwrap();
    assert_eq!(lease.gateway, Some([10, 0, 0, 254]));
    assert_eq!(lease.dns[0], [1, 1, 1, 1]);
    assert_eq!(lease.dns_count, 1);

    let mut buffer = [0u8; 16];
    let length = format_ipv4([10, 0, 0, 50], &mut buffer).unwrap();
    assert_eq!(&buffer[..length], b"10.0.0.50");
}

#[test]
fn client_rejects_invalid_interface_and_zero_mac() {
    assert_eq!(
        DhcpClient::new("", MAC, 1).err(),
        Some(DhcpError::InvalidInterface)
    );
    assert_eq!(
        DhcpClient::new("eth0", [0; 6], 1).err(),
        Some(DhcpError::InvalidInterface)
    );
    let long = "a".repeat(64);
    assert_eq!(
        DhcpClient::new(&long, MAC, 1).err(),
        Some(DhcpError::InvalidInterface)
    );
}

#[test]
fn release_without_lease_and_poll_without_link_are_safe() {
    let mut client = authorized_client();
    let mut transport = CaptureTransport::new();
    let mut runtime = RecordingRuntime::new();
    assert_eq!(
        client.release(0, &mut transport, &mut runtime),
        Err(DhcpError::InvalidState)
    );
    client.set_link(false, 0);
    assert_eq!(client.start(0), Err(DhcpError::InvalidState));
    assert_eq!(client.poll(0, &mut transport, &mut runtime), Ok(()));
}

#[test]
fn firewall_allows_capability_gated_dhcp_client_ports() {
    let mut policy = FirewallPolicy::<8>::new();
    policy.default_action = synos_netd::RuleAction::Drop;
    install_dhcp_client_rules(&mut policy).unwrap();
    let mut firewall = Firewall::<8, 4, 4>::new(policy);
    let key = synos_netd::CapabilityKey::new([3; 32]);
    firewall.set_capability_key(key);

    let discover = [0u8; 8];
    let egress = dhcp_udp_frame(&discover, DHCP_CLIENT_PORT, DHCP_SERVER_PORT);
    let ingress = dhcp_udp_frame(&discover, DHCP_SERVER_PORT, DHCP_CLIENT_PORT);

    let denied = PacketContext {
        principal: 1,
        direction: Direction::Egress,
        now_ms: 1,
        leased_workload: false,
        capability: None,
        signature: None,
        signer: None,
    };
    assert_eq!(
        firewall.inspect(&egress, denied),
        FirewallDecision::AccessDenied
    );

    // inspect() validates capability ports against the packet destination port.
    let raw = synos_netd::NetworkCapability::issue(
        &key,
        1,
        CapabilityRight::Raw as u8,
        synos_netd::PortRange::new(DHCP_SERVER_PORT, DHCP_SERVER_PORT).unwrap(),
        synos_netd::Ipv4Cidr::ANY,
        1_000,
        1,
    )
    .unwrap();
    let allowed_egress = PacketContext {
        capability: Some(raw),
        ..denied
    };
    assert_eq!(
        firewall.inspect(&egress, allowed_egress),
        FirewallDecision::Allow
    );

    let ingress_cap = synos_netd::NetworkCapability::issue(
        &key,
        1,
        CapabilityRight::Ingress as u8,
        synos_netd::PortRange::new(DHCP_CLIENT_PORT, DHCP_CLIENT_PORT).unwrap(),
        synos_netd::Ipv4Cidr::ANY,
        1_000,
        2,
    )
    .unwrap();
    let allowed_ingress = PacketContext {
        principal: 1,
        direction: Direction::Ingress,
        now_ms: 1,
        leased_workload: false,
        capability: Some(ingress_cap),
        signature: None,
        signer: None,
    };
    assert_eq!(
        firewall.inspect(&ingress, allowed_ingress),
        FirewallDecision::Allow
    );
    assert_eq!(
        synos_netd::PacketView::parse(&egress).unwrap().protocol,
        Protocol::Udp
    );
}
