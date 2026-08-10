//! Two-VM deterministic Ethernet integration coverage.

use synos_vm::{
    DhcpServerConfig, DeterministicVmNetwork, MacAddress, NetworkBackendConfig, Vm, VmConfig,
};

const SERVER_IP: [u8; 4] = [10, 5, 0, 1];
const BROADCAST_IP: [u8; 4] = [255, 255, 255, 255];
const ETHERTYPE_IPV4: u16 = 0x0800;
const IP_PROTOCOL_ICMP: u8 = 1;
const IP_PROTOCOL_UDP: u8 = 17;
const IP_PROTOCOL_TCP: u8 = 6;
const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;
const DHCP_MAGIC_COOKIE: u32 = 0x6382_5363;

fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut index = 0;
    while index + 1 < bytes.len() {
        sum += u16::from_be_bytes([bytes[index], bytes[index + 1]]) as u32;
        index += 2;
    }
    if index < bytes.len() {
        sum += (bytes[index] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !sum as u16
}

fn ethernet(destination: MacAddress, source: MacAddress, ether_type: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(14 + payload.len());
    frame.extend_from_slice(&destination.to_bytes());
    frame.extend_from_slice(&source.to_bytes());
    frame.extend_from_slice(&ether_type.to_be_bytes());
    frame.extend_from_slice(payload);
    frame.resize(frame.len().max(60), 0);
    frame
}

fn ipv4_frame(
    destination: MacAddress,
    source: MacAddress,
    source_ip: [u8; 4],
    destination_ip: [u8; 4],
    protocol: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut packet = vec![0u8; 20 + payload.len()];
    packet[0] = 0x45;
    let packet_len = packet.len() as u16;
    packet[2..4].copy_from_slice(&packet_len.to_be_bytes());
    packet[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    packet[8] = 64;
    packet[9] = protocol;
    packet[12..16].copy_from_slice(&source_ip);
    packet[16..20].copy_from_slice(&destination_ip);
    packet[20..].copy_from_slice(payload);
    let header_checksum = checksum(&packet[..20]);
    packet[10..12].copy_from_slice(&header_checksum.to_be_bytes());
    ethernet(destination, source, ETHERTYPE_IPV4, &packet)
}

fn dhcp_frame(
    mac: MacAddress,
    xid: u32,
    message_type: u8,
    ciaddr: [u8; 4],
    requested_ip: Option<[u8; 4]>,
    server_id: Option<[u8; 4]>,
) -> Vec<u8> {
    let mut dhcp = vec![0u8; 240];
    dhcp[0] = 1;
    dhcp[1] = 1;
    dhcp[2] = 6;
    dhcp[4..8].copy_from_slice(&xid.to_be_bytes());
    dhcp[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    dhcp[12..16].copy_from_slice(&ciaddr);
    dhcp[28..34].copy_from_slice(&mac.to_bytes());
    dhcp[236..240].copy_from_slice(&DHCP_MAGIC_COOKIE.to_be_bytes());

    let mut option = |code: u8, value: &[u8]| {
        dhcp.push(code);
        dhcp.push(value.len() as u8);
        dhcp.extend_from_slice(value);
    };
    option(53, &[message_type]);
    if let Some(address) = requested_ip {
        option(50, &address);
    }
    if let Some(server) = server_id {
        option(54, &server);
    }
    dhcp.push(255);

    let source_ip = if ciaddr == [0; 4] { [0; 4] } else { ciaddr };
    let destination_ip = if ciaddr == [0; 4] {
        BROADCAST_IP
    } else {
        SERVER_IP
    };
    let mut udp = vec![0u8; 8 + dhcp.len()];
    udp[..2].copy_from_slice(&DHCP_CLIENT_PORT.to_be_bytes());
    udp[2..4].copy_from_slice(&DHCP_SERVER_PORT.to_be_bytes());
    let udp_len = udp.len() as u16;
    udp[4..6].copy_from_slice(&udp_len.to_be_bytes());
    udp[8..].copy_from_slice(&dhcp);
    ipv4_frame(
        MacAddress::BROADCAST,
        mac,
        source_ip,
        destination_ip,
        IP_PROTOCOL_UDP,
        &udp,
    )
}

fn dhcp_reply(frame: &[u8], expected_mac: MacAddress, expected_xid: u32) -> Option<(u8, [u8; 4])> {
    if frame.len() < 14 + 20 + 8 + 240
        || frame[12..14] != ETHERTYPE_IPV4.to_be_bytes()
        || frame[14 + 9] != IP_PROTOCOL_UDP
    {
        return None;
    }
    let udp = 14 + 20;
    if u16::from_be_bytes([frame[udp], frame[udp + 1]]) != DHCP_SERVER_PORT
        || u16::from_be_bytes([frame[udp + 2], frame[udp + 3]]) != DHCP_CLIENT_PORT
    {
        return None;
    }
    let payload = &frame[udp + 8..];
    if u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) != expected_xid
        || MacAddress::from_bytes(&payload[28..34])? != expected_mac
    {
        return None;
    }

    let mut message_type = None;
    let mut cursor = 240;
    while cursor < payload.len() {
        let code = payload[cursor];
        cursor += 1;
        if code == 255 {
            break;
        }
        if code == 0 {
            continue;
        }
        let length = *payload.get(cursor)? as usize;
        cursor += 1;
        let value = payload.get(cursor..cursor.checked_add(length)?)?;
        cursor += length;
        if code == 53 && value.len() == 1 {
            message_type = Some(value[0]);
        }
    }
    let address = payload[16..20].try_into().ok()?;
    Some((message_type?, address))
}

fn receive_dhcp(vm: &mut Vm, mac: MacAddress, xid: u32) -> (u8, [u8; 4]) {
    for _ in 0..32 {
        let Some(frame) = vm.receive_network_frame().expect("receive DHCP frame") else {
            continue;
        };
        if let Some(reply) = dhcp_reply(&frame, mac, xid) {
            return reply;
        }
    }
    panic!("DHCP reply not found for {mac} xid {xid}")
}

fn drain_network(vm: &mut Vm) {
    while vm
        .receive_network_frame()
        .expect("drain network frame")
        .is_some()
    {}
}

fn icmp_echo(id: u16, sequence: u16, reply: bool) -> Vec<u8> {
    let mut payload = vec![0u8; 18];
    payload[0] = if reply { 0 } else { 8 };
    payload[4..6].copy_from_slice(&id.to_be_bytes());
    payload[6..8].copy_from_slice(&sequence.to_be_bytes());
    payload[8..].copy_from_slice(b"synos-ping");
    let icmp_checksum = checksum(&payload);
    payload[2..4].copy_from_slice(&icmp_checksum.to_be_bytes());
    payload
}

fn tcp_segment(
    source_ip: [u8; 4],
    destination_ip: [u8; 4],
    source_port: u16,
    destination_port: u16,
    sequence: u32,
    acknowledgment: u32,
    flags: u8,
) -> Vec<u8> {
    let mut segment = vec![0u8; 20];
    segment[..2].copy_from_slice(&source_port.to_be_bytes());
    segment[2..4].copy_from_slice(&destination_port.to_be_bytes());
    segment[4..8].copy_from_slice(&sequence.to_be_bytes());
    segment[8..12].copy_from_slice(&acknowledgment.to_be_bytes());
    segment[12] = 5 << 4;
    segment[13] = flags;
    segment[14..16].copy_from_slice(&64240u16.to_be_bytes());

    let mut pseudo_header = Vec::with_capacity(12 + segment.len());
    pseudo_header.extend_from_slice(&source_ip);
    pseudo_header.extend_from_slice(&destination_ip);
    pseudo_header.extend_from_slice(&[0, IP_PROTOCOL_TCP]);
    pseudo_header.extend_from_slice(&(segment.len() as u16).to_be_bytes());
    pseudo_header.extend_from_slice(&segment);
    let tcp_checksum = checksum(&pseudo_header);
    segment[16..18].copy_from_slice(&tcp_checksum.to_be_bytes());
    segment
}

fn assert_ipv4_frame(
    frame: &[u8],
    source_ip: [u8; 4],
    destination_ip: [u8; 4],
    protocol: u8,
) {
    assert_eq!(&frame[12..14], &ETHERTYPE_IPV4.to_be_bytes());
    assert_eq!(&frame[14 + 12..14 + 16], &source_ip);
    assert_eq!(&frame[14 + 16..14 + 20], &destination_ip);
    assert_eq!(frame[14 + 9], protocol);
}

#[test]
fn two_vms_acquire_distinct_dhcp_leases_ping_connect_and_renew() {
    let network = DeterministicVmNetwork::new(Some(DhcpServerConfig {
        lease_time_secs: 4,
        ..DhcpServerConfig::default()
    }))
    .expect("create shared DHCP network");
    let config = VmConfig {
        memory_size: 4 * 1024 * 1024,
        network: NetworkBackendConfig::DeterministicShared {
            network: network.clone(),
        },
        dhcp_server: None,
        ..VmConfig::default()
    };
    let mut first = Vm::try_with_config(config.clone()).expect("create first VM");
    let mut second = Vm::try_with_config(config).expect("create second VM");
    let first_mac = first.network_mac_addresses()[0];
    let second_mac = second.network_mac_addresses()[0];
    let segment = network.borrow().segment();

    assert!(segment.borrow().link_up());
    assert_ne!(first_mac, second_mac);

    first
        .transmit_network_frame(&dhcp_frame(first_mac, 1, 1, [0; 4], None, None))
        .expect("first DHCP discover");
    second
        .transmit_network_frame(&dhcp_frame(second_mac, 2, 1, [0; 4], None, None))
        .expect("second DHCP discover");
    network.borrow_mut().poll(0).expect("serve DHCP discovers");

    let (first_offer_type, first_ip) = receive_dhcp(&mut first, first_mac, 1);
    let (second_offer_type, second_ip) = receive_dhcp(&mut second, second_mac, 2);
    assert_eq!(first_offer_type, 2);
    assert_eq!(second_offer_type, 2);
    assert_ne!(first_ip, second_ip);

    first
        .transmit_network_frame(&dhcp_frame(
            first_mac,
            1,
            3,
            [0; 4],
            Some(first_ip),
            Some(SERVER_IP),
        ))
        .expect("first DHCP request");
    second
        .transmit_network_frame(&dhcp_frame(
            second_mac,
            2,
            3,
            [0; 4],
            Some(second_ip),
            Some(SERVER_IP),
        ))
        .expect("second DHCP request");
    network.borrow_mut().poll(1).expect("serve DHCP requests");

    let (first_ack_type, first_ack_ip) = receive_dhcp(&mut first, first_mac, 1);
    let (second_ack_type, second_ack_ip) = receive_dhcp(&mut second, second_mac, 2);
    assert_eq!(first_ack_type, 5);
    assert_eq!(second_ack_type, 5);
    assert_eq!(first_ack_ip, first_ip);
    assert_eq!(second_ack_ip, second_ip);
    drain_network(&mut first);
    drain_network(&mut second);

    let first_lease = network
        .borrow()
        .dhcp_server()
        .expect("DHCP server")
        .lease_for(first_mac)
        .expect("first lease");
    let second_lease = network
        .borrow()
        .dhcp_server()
        .expect("DHCP server")
        .lease_for(second_mac)
        .expect("second lease");
    assert_eq!(first_lease.address, first_ip);
    assert_eq!(second_lease.address, second_ip);
    assert_ne!(first_lease.address, second_lease.address);

    let ping_request = icmp_echo(7, 1, false);
    first
        .transmit_network_frame(&ipv4_frame(
            second_mac,
            first_mac,
            first_ip,
            second_ip,
            IP_PROTOCOL_ICMP,
            &ping_request,
        ))
        .expect("peer ping request");
    let received_ping = second
        .receive_network_frame()
        .expect("receive ping request")
        .expect("ping request frame");
    assert_ipv4_frame(&received_ping, first_ip, second_ip, IP_PROTOCOL_ICMP);
    assert_eq!(received_ping[14 + 20], 8);

    let ping_reply = icmp_echo(7, 1, true);
    second
        .transmit_network_frame(&ipv4_frame(
            first_mac,
            second_mac,
            second_ip,
            first_ip,
            IP_PROTOCOL_ICMP,
            &ping_reply,
        ))
        .expect("peer ping reply");
    let received_reply = first
        .receive_network_frame()
        .expect("receive ping reply")
        .expect("ping reply frame");
    assert_ipv4_frame(&received_reply, second_ip, first_ip, IP_PROTOCOL_ICMP);
    assert_eq!(received_reply[14 + 20], 0);

    let syn = tcp_segment(first_ip, second_ip, 40_000, 8080, 100, 0, 0x02);
    first
        .transmit_network_frame(&ipv4_frame(
            second_mac,
            first_mac,
            first_ip,
            second_ip,
            IP_PROTOCOL_TCP,
            &syn,
        ))
        .expect("TCP SYN");
    let received_syn = second
        .receive_network_frame()
        .expect("receive TCP SYN")
        .expect("TCP SYN frame");
    assert_ipv4_frame(&received_syn, first_ip, second_ip, IP_PROTOCOL_TCP);
    assert_eq!(received_syn[14 + 20 + 13], 0x02);

    let syn_ack = tcp_segment(second_ip, first_ip, 8080, 40_000, 200, 101, 0x12);
    second
        .transmit_network_frame(&ipv4_frame(
            first_mac,
            second_mac,
            second_ip,
            first_ip,
            IP_PROTOCOL_TCP,
            &syn_ack,
        ))
        .expect("TCP SYN-ACK");
    let received_syn_ack = first
        .receive_network_frame()
        .expect("receive TCP SYN-ACK")
        .expect("TCP SYN-ACK frame");
    assert_ipv4_frame(&received_syn_ack, second_ip, first_ip, IP_PROTOCOL_TCP);
    assert_eq!(received_syn_ack[14 + 20 + 13], 0x12);

    let ack = tcp_segment(first_ip, second_ip, 40_000, 8080, 101, 201, 0x10);
    first
        .transmit_network_frame(&ipv4_frame(
            second_mac,
            first_mac,
            first_ip,
            second_ip,
            IP_PROTOCOL_TCP,
            &ack,
        ))
        .expect("TCP ACK");
    let received_ack = second
        .receive_network_frame()
        .expect("receive TCP ACK")
        .expect("TCP ACK frame");
    assert_ipv4_frame(&received_ack, first_ip, second_ip, IP_PROTOCOL_TCP);
    assert_eq!(received_ack[14 + 20 + 13], 0x10);

    first
        .transmit_network_frame(&dhcp_frame(
            first_mac,
            3,
            3,
            first_ip,
            None,
            None,
        ))
        .expect("first DHCP renewal");
    second
        .transmit_network_frame(&dhcp_frame(
            second_mac,
            4,
            3,
            second_ip,
            None,
            None,
        ))
        .expect("second DHCP renewal");
    network.borrow_mut().poll(2_000).expect("serve DHCP renewals");

    let (first_renew_type, first_renew_ip) = receive_dhcp(&mut first, first_mac, 3);
    let (second_renew_type, second_renew_ip) = receive_dhcp(&mut second, second_mac, 4);
    assert_eq!(first_renew_type, 5);
    assert_eq!(second_renew_type, 5);
    assert_eq!(first_renew_ip, first_ip);
    assert_eq!(second_renew_ip, second_ip);
    assert!(network
        .borrow()
        .dhcp_server()
        .expect("DHCP server")
        .lease_for(first_mac)
        .expect("renewed first lease")
        .expires_at_ms
        > first_lease.expires_at_ms);
    assert!(network
        .borrow()
        .dhcp_server()
        .expect("DHCP server")
        .lease_for(second_mac)
        .expect("renewed second lease")
        .expires_at_ms
        > second_lease.expires_at_ms);
}
