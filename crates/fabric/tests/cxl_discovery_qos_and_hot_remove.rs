// Inventory: coverage_59_7.rs (legacy roadmap section 59).
use ghostos_fabric::{
    Access, AddressRange, NodeId, PAGE_SIZE,
    cluster::{Heartbeat, HeartbeatMonitor, NodeState},
    cxl::{CxlBandwidthDecision, CxlBandwidthPolicy, CxlBandwidthQos, CxlChannel, CxlVersion, DeviceType, Endpoint, Registry},
    dsm::{CoherenceAction, CoherenceDirectory, DlmLeaseMode, DsmHeader, DsmPacket, MessageKind, PageAssembler, SoftwareDlmLease, GHOSTOS_DSM_ETHERTYPE},
    PageFault,
};
use ghostos_fabric::cluster::{NodeFailure, NodeIsolation, recover_failed_node};
use ghostos_fabric::memory::{
    GlobalAddressSpace, LeaseOwner, LeaseRights, LeaseTable, MemoryKind, MemoryPool, PoolId,
    Transport,
};
use ghostos_time_sync::ManualClock;

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
    assert_eq!(registry.discover(endpoint, 0), Err(ghostos_fabric::Error::InvalidDevice));
    assert_eq!(registry.begin_remove(7).unwrap().state, ghostos_fabric::cxl::DeviceState::Draining);
    assert_eq!(registry.begin_remove(7), Err(ghostos_fabric::Error::Busy));
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
    assert_eq!(u16::from_be_bytes([wire[0], wire[1]]), GHOSTOS_DSM_ETHERTYPE);
    assert_eq!(DsmPacket::decode(&wire[..length]).unwrap().payload(), b"fetch");
    assert!(matches!(
        DsmPacket::new(DsmHeader { page_address: 1, ..header }, b"x"),
        Err(ghostos_fabric::Error::CorruptPacket)
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
    assert_eq!(directory.begin_fault(owner, PageFault { reserved_bit: true, ..fault }, 10), Err(ghostos_fabric::Error::InvalidAddress));
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

struct Isolation {
    isolated: bool,
}

impl NodeIsolation for Isolation {
    fn is_node_isolated(&self, _node: NodeId) -> bool {
        self.isolated
    }
}

fn recovered_memory_fixture() -> (
    GlobalAddressSpace<2, 2>,
    LeaseTable<2>,
    CoherenceDirectory<2>,
    PoolId,
    NodeId,
) {
    let owner = node(2);
    let mirror = node(1);
    let pool = PoolId::new(1).unwrap();
    let mut space = GlobalAddressSpace::new();
    space
        .add_pool(MemoryPool {
            id: pool,
            node: owner,
            mirror: Some(mirror),
            kind: MemoryKind::Ram,
            transport: Transport::Layer2,
            global: AddressRange::new(PAGE_SIZE, PAGE_SIZE).unwrap(),
            backing_start: PAGE_SIZE,
            latency_ns: 1,
        })
        .unwrap();
    let mut leases = LeaseTable::new();
    let _ = leases
        .allocate(
            &space,
            LeaseOwner::Node(owner),
            pool,
            PAGE_SIZE,
            PAGE_SIZE,
            LeaseRights::READ_WRITE,
            0,
            1_000,
        )
        .unwrap();
    let mut coherence = CoherenceDirectory::new();
    coherence
        .grant_lease(
            PAGE_SIZE,
            SoftwareDlmLease {
                owner,
                mode: DlmLeaseMode::Exclusive,
                epoch: 1,
                expires_at_us: 1_000,
            },
        )
        .unwrap();
    coherence.page_arrived(PAGE_SIZE, owner, true).unwrap();
    (space, leases, coherence, pool, owner)
}

#[test]
fn partition_recovery_does_not_release_memory_before_fencing() {
    let (mut space, mut leases, mut coherence, _pool, owner) = recovered_memory_fixture();
    let failure = NodeFailure {
        node: owner,
        detected_at_us: 100,
        silence_us: 100,
    };
    assert_eq!(
        recover_failed_node(
            failure,
            &Isolation { isolated: false },
            &mut space,
            &mut leases,
            &mut coherence,
        ),
        Err(ghostos_fabric::Error::NodeNotFenced)
    );
    assert!(!space.is_node_failed(owner));
    assert_eq!(leases.active_for_pool(PoolId::new(1).unwrap()), 1);
    assert_eq!(coherence.pages().next().unwrap().owner, Some(owner));

    let summary = recover_failed_node(
        failure,
        &Isolation { isolated: true },
        &mut space,
        &mut leases,
        &mut coherence,
    )
    .unwrap();
    assert_eq!(summary.leases_released, 1);
    assert_eq!(summary.coherence_pages_recovered, 1);
    assert!(space.is_node_failed(owner));
    assert_eq!(space.resolve(PAGE_SIZE).unwrap().node, node(1));
    assert!(space.resolve(PAGE_SIZE).unwrap().failed_over);
    assert_eq!(leases.active_for_pool(PoolId::new(1).unwrap()), 0);
    assert_eq!(coherence.pages().next().unwrap().owner, None);
    assert_eq!(coherence.pages().next().unwrap().lease, None);
}

#[test]
fn future_peer_timestamp_cannot_extend_partition_recovery_deadline() {
    let clock = ManualClock::new(1_000);
    let mut monitor = HeartbeatMonitor::<1>::new_with_clock(node(1), 100, 2, &clock).unwrap();
    monitor.add_node_with_clock(node(2), &clock).unwrap();
    monitor
        .observe(
            Heartbeat {
                node: node(2),
                sequence: 1,
                sent_at_us: u64::MAX,
            },
            clock.now_us(),
        )
        .unwrap();
    clock.advance_us(199);
    assert_eq!(monitor.detect_with_clock(&clock), None);
    clock.advance_us(1);
    let failure = monitor.detect_with_clock(&clock).unwrap();
    assert_eq!(failure.detected_at_us, 1_200);
    assert_eq!(failure.silence_us, 200);

    let (mut space, mut leases, mut coherence, _pool, owner) = recovered_memory_fixture();
    assert_eq!(
        recover_failed_node(
            failure,
            &Isolation { isolated: false },
            &mut space,
            &mut leases,
            &mut coherence,
        ),
        Err(ghostos_fabric::Error::NodeNotFenced)
    );
    assert!(!space.is_node_failed(owner));
    assert_eq!(leases.active_for_pool(PoolId::new(1).unwrap()), 1);
}

#[test]
fn invalid_ranges_are_not_admitted() {
    assert_eq!(AddressRange::new(10, 0), Err(ghostos_fabric::Error::InvalidRange));
    assert_eq!(NodeId::new(0), None);
}
