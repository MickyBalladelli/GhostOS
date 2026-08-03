use synos_fabric::NodeId;
use synos_mesh::{
    ComputeTarget, CowDelta, DeltaError, DeltaMode, GossipDiscovery, InterfaceSet, MeshInterface,
    NodeAdvertisement, NodeRole, OffloadPlanner, OffloadSession, WorkloadClass, WorkloadSpec,
};
use synos_synfs::{RmsMapHandle, SynFs};

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
    assert_eq!(discovery.observe(announcement, 101), Err(synos_mesh::DiscoveryError::StaleAnnouncement));
    assert_eq!(discovery.expire(200, 100).unwrap(), Some(NodeId::new(2).unwrap()));
    assert_eq!(discovery.peer(NodeId::new(2).unwrap()).unwrap().status, synos_mesh::PeerStatus::Expired);
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
    synos_mesh::build_delta(&base, &target, &mut delta).unwrap();
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
    assert!(matches!(session.chunk(b"x", false), Err(synos_mesh::OffloadError::SessionClosed)));
    planner.mark_failed(NodeId::new(2).unwrap(), true).unwrap();
    assert!(matches!(planner.plan(workload), Err(synos_mesh::OffloadError::NoEligibleTarget)));
}
