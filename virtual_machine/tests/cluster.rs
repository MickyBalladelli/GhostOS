//! Deterministic cluster harness coverage.

use synos_fabric::{AddressRange, PAGE_SIZE};
use synos_vm::{
    ClusterFault, ClusterNetwork, ClusterNetworkConfig, ClusterNetworkOutcome, ClusterNodeId,
    ClusterNodeState, ClusterPacket, CxlFabricFixture, VmCluster, VmConfig,
};

fn node(raw: u32) -> ClusterNodeId {
    ClusterNodeId::new(raw).expect("valid test node")
}

fn packet(source: u32, target: u32, payload: &[u8]) -> ClusterPacket {
    ClusterPacket {
        source: node(source),
        target: node(target),
        payload: payload.to_vec(),
    }
}

#[test]
fn cluster_network_is_bounded_and_reproducible() {
    let config = ClusterNetworkConfig {
        latency_ticks: 2,
        duplicate_percent: 100,
        reorder: true,
        ..ClusterNetworkConfig::default()
    };
    let mut network = ClusterNetwork::new(config).expect("valid network config");
    assert_eq!(
        network.send(packet(1, 2, b"first")),
        ClusterNetworkOutcome::Queued {
            copies: 2,
            deliver_at: 2,
        }
    );
    assert_eq!(
        network.send(packet(1, 2, b"second")),
        ClusterNetworkOutcome::Queued {
            copies: 2,
            deliver_at: 2,
        }
    );
    network.advance(1);
    assert!(network.receive(node(2)).is_empty());
    network.advance(1);
    let received = network.receive(node(2));
    assert_eq!(received.len(), 4);
    assert_eq!(received[0].payload, b"second");
    assert_eq!(received[2].payload, b"first");

    network.partition(node(1), node(2));
    assert_eq!(
        network.send(packet(1, 2, b"blocked")),
        ClusterNetworkOutcome::Partitioned
    );
    network.reconnect(node(1), node(2));
    assert!(matches!(
        network.send(packet(1, 2, b"reconnected")),
        ClusterNetworkOutcome::Queued { .. }
    ));
}

#[test]
fn cluster_network_can_drop_every_packet() {
    let mut network = ClusterNetwork::new(ClusterNetworkConfig {
        loss_percent: 100,
        ..ClusterNetworkConfig::default()
    })
    .expect("valid network config");
    assert_eq!(
        network.send(packet(1, 2, b"lost")),
        ClusterNetworkOutcome::Dropped
    );
    network.advance(10);
    assert!(network.receive(node(2)).is_empty());
}

#[test]
fn vm_cluster_orchestrates_nodes_and_faults() {
    let mut cluster = VmCluster::new(ClusterNetworkConfig::default()).expect("cluster");
    let config = VmConfig {
        memory_size: 4 * 1024 * 1024,
        ..VmConfig::default()
    };
    cluster.add_node(node(1), config.clone()).expect("node one");
    cluster.add_node(node(2), config).expect("node two");

    cluster
        .send(packet(1, 2, b"heartbeat"))
        .expect("send heartbeat");
    cluster.advance(1);
    assert_eq!(
        cluster.receive(node(2)).expect("receive heartbeat").len(),
        1
    );

    cluster
        .inject_fault(ClusterFault::IsolateNode(node(2)))
        .expect("isolate node");
    assert_eq!(
        cluster.node(node(2)).expect("node two").state(),
        ClusterNodeState::Isolated
    );
    assert!(cluster.send(packet(2, 1, b"blocked")).is_err());

    cluster
        .inject_fault(ClusterFault::RecoverNode(node(2)))
        .expect("recover node");
    cluster
        .inject_fault(ClusterFault::Partition {
            left: node(1),
            right: node(2),
        })
        .expect("partition");
    assert_eq!(
        cluster.send(packet(1, 2, b"partitioned")).expect("send"),
        ClusterNetworkOutcome::Partitioned
    );
}

#[test]
fn cxl_fixture_maps_migrates_removes_and_fails_over() {
    let mut fabric = CxlFabricFixture::new();
    let first = fabric
        .add_device(
            node(1),
            AddressRange::new(0x1_0000_0000, PAGE_SIZE * 2).expect("first range"),
            0x1000,
            Some(node(2)),
            250,
        )
        .expect("first CXL device");
    let second = fabric
        .add_device(
            node(2),
            AddressRange::new(0x2_0000_0000, PAGE_SIZE * 2).expect("second range"),
            0x2000,
            None,
            100,
        )
        .expect("second CXL device");

    fabric
        .write(0x1_0000_0040, b"cluster-page")
        .expect("write CXL");
    assert_eq!(
        fabric.read(0x1_0000_0040, 12).expect("read CXL"),
        b"cluster-page"
    );
    fabric
        .migrate_page(0x1_0000_0000, second, 0x2000)
        .expect("migrate page");
    assert_eq!(
        fabric
            .resolve(0x1_0000_0040)
            .expect("resolve migrated page")
            .node
            .raw(),
        2
    );
    fabric.hot_remove(first).expect("hot remove source");
    assert_eq!(
        fabric.read(0x1_0000_0040, 12).expect("read after removal"),
        b"cluster-page"
    );

    fabric.fail_node(node(2)).expect("fail mirror node");
    assert!(fabric.resolve(0x1_0000_0040).is_err());
    fabric.restore_node(node(2)).expect("restore mirror node");
    assert_eq!(
        fabric
            .resolve(0x1_0000_0040)
            .expect("restore mapping")
            .node
            .raw(),
        2
    );
}
