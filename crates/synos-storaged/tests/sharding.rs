use synos_fabric::NodeId;
use synos_storaged::{
    ClusterShardCoordinator, ConsistencyContract, ShardError, ShardNamespace, ShardState,
};

fn node(raw: u32) -> NodeId {
    NodeId::new(raw).unwrap()
}

fn exercise() -> synos_storaged::RecoveryEvidence {
    let mut cluster = ClusterShardCoordinator::<16, 8, 32>::new(3).unwrap();
    for raw in 1..=5 {
        cluster.add_node(node(raw), 100).unwrap();
    }
    cluster.elect_leader(node(1), 1).unwrap();
    let metadata = cluster.create_shard(ShardNamespace::ClusterMetadata).unwrap();
    let capabilities = cluster.create_shard(ShardNamespace::CapabilityIndex).unwrap();
    let packages = cluster.create_shard(ShardNamespace::PackageCatalog).unwrap();
    let audit = cluster.create_shard(ShardNamespace::AuditStream).unwrap();
    let placement = cluster.create_shard(ShardNamespace::PlacementDecision).unwrap();

    assert_eq!(ShardNamespace::ClusterMetadata.consistency(), ConsistencyContract::Linearizable);
    assert_eq!(ShardNamespace::PackageCatalog.consistency(), ConsistencyContract::Snapshot);
    assert_eq!(ShardNamespace::AuditStream.consistency(), ConsistencyContract::AppendOnly);
    assert_eq!(cluster.route(ShardNamespace::AuditStream, 0).unwrap().contract, ConsistencyContract::AppendOnly);

    for shard in [metadata, capabilities, packages, audit, placement] {
        let primary = cluster.shard(shard).unwrap().primary;
        cluster.commit_write(shard, primary, 1, 1, 0).unwrap();
    }

    assert_eq!(cluster.elect_leader(node(2), 1), Err(ShardError::SplitBrain));
    let generation = cluster.shard(metadata).unwrap().generation;
    cluster.begin_move(metadata, node(4), generation, node(1), 1).unwrap();
    assert_eq!(cluster.shard(metadata).unwrap().state, ShardState::Moving);
    cluster.acknowledge_move(metadata, node(4), 1).unwrap();
    cluster.commit_move(metadata, node(1), 1).unwrap();
    cluster.rebalance(node(1), 1).unwrap();

    cluster.fail_node(node(2)).unwrap();
    assert!(cluster.recovery_evidence().degraded_shards > 0);
    let recovered_epoch = cluster.recover_node(node(2)).unwrap();
    assert_eq!(recovered_epoch, 2);
    cluster.reconcile_node(node(2), node(1), 1).unwrap();
    let evidence = cluster.recovery_evidence();
    assert!(evidence.is_stable());
    assert_eq!(evidence.shard_moves, 2);
    assert_eq!(evidence.rebalances, 1);
    assert_eq!(evidence.split_brain_rejections, 1);
    assert_eq!(evidence.node_losses, 1);
    assert_eq!(evidence.recovered_nodes, 1);
    evidence
}

#[test]
fn shard_recovery_evidence_is_stable_across_replay() {
    assert_eq!(exercise(), exercise());
}

#[test]
fn stale_writer_cannot_cross_node_loss_fence() {
    let mut cluster = ClusterShardCoordinator::<4, 4, 16>::new(3).unwrap();
    for raw in 1..=3 {
        cluster.add_node(node(raw), 100).unwrap();
    }
    cluster.elect_leader(node(1), 1).unwrap();
    let shard = cluster.create_shard(ShardNamespace::CapabilityIndex).unwrap();
    cluster.fail_node(node(1)).unwrap();
    cluster.elect_leader(node(2), 2).unwrap();
    assert_eq!(
        cluster.commit_write(shard, node(1), 1, 1, 0),
        Err(ShardError::StaleTerm)
    );
    assert_eq!(
        cluster.commit_write(shard, node(2), 1, 2, 0).unwrap().index,
        1
    );
}
