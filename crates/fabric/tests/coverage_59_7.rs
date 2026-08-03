use synos_fabric::{
    Access, AddressRange, NodeId, PAGE_SIZE,
    cluster::{Heartbeat, HeartbeatMonitor, NodeState},
    cxl::{CxlBandwidthDecision, CxlBandwidthPolicy, CxlBandwidthQos, CxlChannel, CxlVersion, DeviceType, Endpoint, Registry},
    dsm::{CoherenceAction, CoherenceDirectory, DlmLeaseMode, DsmHeader, DsmPacket, MessageKind, PageAssembler, SoftwareDlmLease, SYNOS_DSM_ETHERTYPE},
    PageFault,
};

fn node(raw: u32) -> NodeId {
    NodeId::new(raw).unwrap()
}

#[test]
fn cxl_discovery_qos_and_hot_remove_are_bounded() {
    let endpoint = Endpoint {
        node: node(2),
        serial: 7,
        version: CxlVersion::V3_1,
        device_type: DeviceType::Type3,
        component_register_base: 1,
        component_register_bytes: 0x100,
        hdm_decoder_offset: 0,
        volatile_capacity: 256 * 1024 * 1024,
        persistent_capacity: 0,
    };
    let mut registry = Registry::<1>::new();
    assert_eq!(registry.discover(endpoint, 2), Ok(0));
    assert_eq!(registry.discover(endpoint, 0), Err(synos_fabric::Error::InvalidDevice));
    assert_eq!(registry.begin_remove(7).unwrap().state, synos_fabric::cxl::DeviceState::Draining);
    assert_eq!(registry.begin_remove(7), Err(synos_fabric::Error::Busy));
    registry.cancel_remove(7).unwrap();
    registry.begin_remove(7).unwrap();
    assert_eq!(registry.complete_remove(7).unwrap().endpoint.serial, 7);

    let channel = CxlChannel::new(node(2), 0);
    let mut qos = CxlBandwidthQos::<1>::new();
    qos.configure(channel, CxlBandwidthPolicy::new(100, 100).unwrap()).unwrap();
    assert_eq!(qos.admit(channel, 0, 100).unwrap(), CxlBandwidthDecision::Allowed);
    assert!(matches!(qos.admit(channel, 0, 1), Ok(CxlBandwidthDecision::Throttled { .. })));
    assert_eq!(qos.admit(channel, 1_000_000, 1).unwrap(), CxlBandwidthDecision::Allowed);
}

#[test]
fn dsm_packets_reassemble_and_reject_corruption() {
    let header = DsmHeader {
        kind: MessageKind::PageRequest,
        source: node(1),
        destination: node(2),
        sequence: 9,
        page_address: PAGE_SIZE,
        lease_epoch: 3,
        fragment: 0,
        fragment_count: 1,
    };
    let packet = DsmPacket::new(header, b"fetch").unwrap();
    let mut wire = [0; 64];
    let length = packet.encode(&mut wire).unwrap();
    assert_eq!(u16::from_be_bytes([wire[0], wire[1]]), SYNOS_DSM_ETHERTYPE);
    assert_eq!(DsmPacket::decode(&wire[..length]).unwrap().payload(), b"fetch");
    assert!(matches!(
        DsmPacket::new(DsmHeader { page_address: 1, ..header }, b"x"),
        Err(synos_fabric::Error::CorruptPacket)
    ));

    let mut page = [0; PAGE_SIZE as usize];
    page[0] = 1;
    page[PAGE_SIZE as usize - 1] = 2;
    let mut assembler = PageAssembler::new(PAGE_SIZE, 10);
    for fragment in [2, 0, 1] {
        let packet = DsmPacket::page_fragment(node(2), node(1), 10, PAGE_SIZE, 4, fragment, &page).unwrap();
        assert_eq!(assembler.push(&packet).unwrap(), fragment == 1);
    }
    assert_eq!(assembler.page().unwrap()[0], 1);
    assert_eq!(assembler.page().unwrap()[PAGE_SIZE as usize - 1], 2);
}

#[test]
fn dsm_coherence_fences_stale_owners() {
    let page = 2 * PAGE_SIZE;
    let owner = node(1);
    let reader = node(2);
    let lease = SoftwareDlmLease {
        owner,
        mode: DlmLeaseMode::Exclusive,
        epoch: 8,
        expires_at_us: 100,
    };
    let mut directory = CoherenceDirectory::<2>::new();
    directory.grant_lease(page, lease).unwrap();
    directory.page_arrived(page, reader, false).unwrap();
    let fault = PageFault {
        virtual_address: page + 17,
        access: Access::Write,
        user: true,
        present: false,
        reserved_bit: false,
        instruction_fetch: false,
    };
    assert!(matches!(directory.begin_fault(owner, fault, 10), Ok(CoherenceAction::Invalidate { nodes, .. }) if nodes != 0));
    directory.invalidation_complete(page, owner).unwrap();
    assert_eq!(directory.begin_fault(owner, fault, 10), Ok(CoherenceAction::MapLocal { writable: true }));
    assert_eq!(directory.begin_fault(owner, PageFault { reserved_bit: true, ..fault }, 10), Err(synos_fabric::Error::InvalidAddress));
    assert_eq!(directory.fail_node(owner).unwrap(), 1);
    assert_eq!(directory.pages().next().unwrap().lease, None);
}

#[test]
fn heartbeat_wire_ordering_and_failure_detection() {
    let heartbeat = Heartbeat { node: node(2), sequence: 4, sent_at_us: 77 };
    assert_eq!(Heartbeat::decode(heartbeat.encode()).unwrap(), heartbeat);
    let mut monitor = HeartbeatMonitor::<1>::new(node(1), 100, 2, 0).unwrap();
    monitor.add_node(node(2), 0).unwrap();
    assert_eq!(monitor.due(99), None);
    assert_eq!(monitor.due(100).unwrap().sequence, 1);
    monitor.observe(heartbeat, 100).unwrap();
    monitor.observe(Heartbeat { sequence: 3, ..heartbeat }, 101).unwrap();
    assert_eq!(monitor.state(node(2)), Some(NodeState::Alive));
    let failure = monitor.detect(301).unwrap();
    assert_eq!(failure.node, node(2));
    assert_eq!(monitor.state(node(2)), Some(NodeState::Failed));
}

#[test]
fn invalid_ranges_are_not_admitted() {
    assert_eq!(AddressRange::new(10, 0), Err(synos_fabric::Error::InvalidRange));
    assert_eq!(NodeId::new(0), None);
}
