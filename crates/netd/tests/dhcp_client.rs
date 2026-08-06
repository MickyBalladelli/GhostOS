use synos_netd::{
    dhcp_client_firewall_rules, install_dhcp_client_rules, CapabilityRight, DhcpClient,
    DhcpClientState, DhcpError, DhcpLease, DhcpLeaseRuntime, DhcpServerFixture, DhcpTransport,
    FirewallPolicy, MAX_DHCP_PACKET, StaticSnapshot, BACKOFF_MS, MAX_DISCOVER_ATTEMPTS,
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
