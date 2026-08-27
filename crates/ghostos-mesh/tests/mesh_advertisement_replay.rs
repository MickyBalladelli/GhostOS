// Inventory: coverage_59_7.rs (legacy roadmap section 59).
use ghostos_fabric::NodeId;
use ghostos_auth::CapabilityKey;
use ghostos_mesh::{
    ComputeTarget, CowDelta, DeltaError, DeltaMode, GossipDiscovery, InterfaceSet, MeshInterface,
    NodeAdvertisement, NodeRole, OffloadPlanner, OffloadSession, WorkloadClass, WorkloadSpec,
};
use ghostos_mesh::{
    ClusterAdvertisement, ClusterCapabilities, ClusterId, ConnectivityManager, Endpoint,
    Reachability, RouteKind, TopologyGraph, TopologyLink, Transport, Zone,
};
use ghostos_ghostfs::{RmsMapHandle, SynFs};

fn advertisement(raw: u32, generation: u64) -> NodeAdvertisement {
    NodeAdvertisement::new(
        NodeId::new(raw).unwrap(),
        generation,
        NodeRole::Cluster,
        InterfaceSet::ALL,
        4_000,
        8 * 1024 * 1024,
        1_000,
        20,
        generation,
    )
    .unwrap()
}

#[test]
fn mesh_advertisements_round_trip_and_replay_is_rejected() {
    let local = advertisement(1, 1);
    let remote = advertisement(2, 4);
    assert_eq!(NodeAdvertisement::decode(local.encode()).unwrap(), local);
    let mut discovery = GossipDiscovery::<2>::new(local);
    let announcement = GossipDiscovery::<2>::new(remote).due(MeshInterface::LocalNetwork);
    assert!(discovery.observe(announcement, 100).unwrap());
    assert_eq!(discovery.peer(NodeId::new(2).unwrap()).unwrap().last_sequence, 1);
    assert_eq!(discovery.observe(announcement, 101), Err(ghostos_mesh::DiscoveryError::StaleAnnouncement));
    assert_eq!(discovery.expire(200, 100).unwrap(), Some(NodeId::new(2).unwrap()));
    assert_eq!(discovery.peer(NodeId::new(2).unwrap()).unwrap().status, ghostos_mesh::PeerStatus::Expired);
}

#[test]
fn cow_delta_preserves_generation_and_rejects_conflicts() {
    let mut base_fs = SynFs::<16>::new();
    base_fs.write("/NOTE.TXT", b"old").unwrap();
    let base = base_fs.mapped_snapshot(RmsMapHandle::from_capability(1 << 32).unwrap());
    let mut target_fs = SynFs::<16>::new();
    target_fs.write("/NOTE.TXT", b"new").unwrap();
    let target = target_fs.mapped_snapshot(RmsMapHandle::from_capability(1 << 32).unwrap());
    let mut delta = CowDelta::<4, 16>::new(0, 0);
    ghostos_mesh::build_delta(&base, &target, &mut delta).unwrap();
    assert_eq!(delta.len(), 1);
    assert!(delta.target_generation() >= delta.base_generation());

    let mut conflicting = SynFs::<16>::new();
    conflicting.write("/NOTE.TXT", b"local").unwrap();
    assert!(matches!(delta.apply(&mut conflicting, DeltaMode::RejectConflicts), Err(DeltaError::Conflict { .. })));
}

#[test]
fn asymmetric_offload_skips_failed_nodes_and_closes_sessions() {
    let remote = advertisement(2, 1);
    let mut planner = OffloadPlanner::<2>::new(NodeId::new(1).unwrap());
    planner
        .update(ComputeTarget::from_advertisement(remote, 3_000, 4 * 1024 * 1024))
        .unwrap();
    let workload = WorkloadSpec {
        id: 9,
        class: WorkloadClass::Heavy,
        cpu_millis: 500,
        memory_bytes: 1024,
        input_bytes: 100,
        deadline_us: 1_000,
    };
    let plan = planner.plan(workload).unwrap();
    assert_eq!(plan.target, NodeId::new(2).unwrap());
    let mut session = OffloadSession::<8>::open(plan);
    assert_eq!(session.chunk(b"abc", false).unwrap().sequence, 0);
    assert_eq!(session.chunk(b"done", true).unwrap().final_chunk, true);
    assert!(session.is_closed());
    assert!(matches!(session.chunk(b"x", false), Err(ghostos_mesh::OffloadError::SessionClosed)));
    planner.mark_failed(NodeId::new(2).unwrap(), true).unwrap();
    assert!(matches!(planner.plan(workload), Err(ghostos_mesh::OffloadError::NoEligibleTarget)));
}

#[test]
fn signed_topology_rotates_endpoints_and_negotiates_mtu() {
    let key = CapabilityKey::new([7; 32]);
    let cluster = ClusterId::new([3; 16]).unwrap();
    let primary = Endpoint::new(Transport::Ethernet, RouteKind::Direct, "10.0.0.2", 7_001, 1, 1_500).unwrap();
    let relay = Endpoint::new(Transport::Tunnel, RouteKind::Relay, "relay.example", 7_002, 2, 1_200).unwrap();
    let advertisement = ClusterAdvertisement::issue(
        key,
        cluster,
        NodeId::new(2).unwrap(),
        1,
        10,
        1_000,
        (1, 1, 1),
        ClusterCapabilities::CONTROL_PLANE.with(ClusterCapabilities::DATA_PLANE),
        &[primary, relay],
    )
    .unwrap();
    advertisement.verify(key, 100).unwrap();
    assert_eq!(ClusterAdvertisement::decode(advertisement.encode()).unwrap(), advertisement);

    let mut connectivity = ConnectivityManager::<2>::new(10, 100, 3).unwrap();
    connectivity.observe(advertisement, key, 100).unwrap();
    assert_eq!(connectivity.connect(NodeId::new(2).unwrap(), 100).unwrap().endpoint, primary);
    connectivity.record_result(NodeId::new(2).unwrap(), false, 100, 0).unwrap();
    assert!(connectivity.connect(NodeId::new(2).unwrap(), 100).is_err());
    assert_eq!(connectivity.connect(NodeId::new(2).unwrap(), 110).unwrap().endpoint, relay);
    connectivity.record_result(NodeId::new(2).unwrap(), true, 110, 900).unwrap();
    assert_eq!(connectivity.negotiated_mtu(NodeId::new(2).unwrap()), Some(900));

    let zone = Zone::new("rack-a").unwrap();
    let local = ClusterAdvertisement::issue(
        key,
        cluster,
        NodeId::new(1).unwrap(),
        1,
        10,
        1_000,
        (1, 1, 1),
        ClusterCapabilities::CONTROL_PLANE,
        &[Endpoint::new(Transport::Loopback, RouteKind::Direct, "local", 0, 1, 65_535).unwrap()],
    )
    .unwrap();
    let mut graph = TopologyGraph::<2, 2>::new();
    graph.observe(local, key, zone, 100).unwrap();
    graph.observe(advertisement, key, zone, 100).unwrap();
    graph
        .update_link(TopologyLink {
            from: NodeId::new(1).unwrap(),
            to: NodeId::new(2).unwrap(),
            transport: Transport::Ethernet,
            route: RouteKind::Direct,
            reachability: Reachability::Reachable,
            latency_us: 20,
            bandwidth_mbps: 1_000,
            mtu: 1_500,
            observed_at_us: 100,
        })
        .unwrap();
    assert_eq!(graph.link_count(), 1);
}
