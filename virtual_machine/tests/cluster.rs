//! Deterministic cluster harness coverage.

use std::fs::{self, File};
use std::io::Write;

use synos_fabric::dsm::{CoherenceAction, CoherenceDirectory, DlmLeaseMode, SoftwareDlmLease};
use synos_fabric::{Access, AddressRange, NodeId, PageFault, PAGE_SIZE};
use synos_vm::{
    ClusterFault, ClusterNetwork, ClusterNetworkConfig, ClusterNetworkOutcome, ClusterNodeId,
    ClusterNodeState, ClusterPacket, ClusterWorkload, CxlFabricFixture, VmCluster, VmConfig,
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

fn serial_kernel() -> Vec<u8> {
    let mut code = Vec::new();
    for byte in b"SynOS kernel bootstrap\nsynos> " {
        code.extend_from_slice(&[0xBA, 0xF8, 0x03, 0x00, 0x00, 0xB0, *byte, 0xEE]);
    }
    code.push(0xF4);
    code
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
    assert_eq!(fabric.discover().len(), 2);
    assert!(fabric.access(node(1), 0x1_0000_0040, false).is_ok());

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

#[test]
fn coherence_fences_stale_owner_and_recovers_a_fetched_page() {
    let mut directory = CoherenceDirectory::<8>::new();
    let page = 0x4000;
    let read_fault = PageFault {
        virtual_address: page + 7,
        access: Access::Read,
        user: false,
        present: false,
        reserved_bit: false,
        instruction_fetch: false,
    };
    assert!(matches!(
        directory.begin_fault(NodeId::new(1).expect("node one"), read_fault, 10),
        Ok(CoherenceAction::AcquireDlmLease {
            mode: DlmLeaseMode::ProtectedRead,
            ..
        })
    ));
    directory
        .grant_lease(
            page,
            SoftwareDlmLease {
                owner: NodeId::new(1).expect("node one"),
                mode: DlmLeaseMode::ProtectedRead,
                epoch: 4,
                expires_at_us: 100,
            },
        )
        .expect("grant read lease");
    directory
        .page_arrived(page, NodeId::new(1).expect("node one"), false)
        .expect("page fetch");

    let write_fault = PageFault {
        access: Access::Write,
        ..read_fault
    };
    assert!(matches!(
        directory.begin_fault(NodeId::new(2).expect("node two"), write_fault, 20),
        Ok(CoherenceAction::AcquireDlmLease {
            mode: DlmLeaseMode::Exclusive,
            ..
        })
    ));
    directory
        .grant_lease(
            page,
            SoftwareDlmLease {
                owner: NodeId::new(2).expect("node two"),
                mode: DlmLeaseMode::Exclusive,
                epoch: 5,
                expires_at_us: 100,
            },
        )
        .expect("grant write lease");
    directory
        .invalidation_complete(page, NodeId::new(2).expect("node two"))
        .expect("fence old owner");
    assert_eq!(
        directory.pages().next().expect("coherence page").state,
        synos_fabric::dsm::DsmPageState::Exclusive
    );
}

#[test]
fn cluster_membership_heartbeats_epochs_quorum_and_rejoin_are_deterministic() {
    let mut cluster = VmCluster::new(ClusterNetworkConfig::default()).expect("cluster");
    let config = VmConfig {
        memory_size: 4 * 1024 * 1024,
        ..VmConfig::default()
    };
    for raw in 1..=3 {
        cluster
            .add_node(node(raw), config.clone())
            .expect("add cluster node");
    }
    assert!(cluster.add_node(node(1), config.clone()).is_err());

    assert_eq!(cluster.discover_nodes(), vec![node(1), node(2), node(3)]);
    assert!(cluster.status().quorum);
    let heartbeat = cluster.heartbeat(node(1)).expect("heartbeat");
    cluster
        .observe_heartbeat(node(2), heartbeat)
        .expect("observe heartbeat");

    cluster
        .inject_fault(ClusterFault::Partition {
            left: node(1),
            right: node(2),
        })
        .expect("partition nodes");
    assert_eq!(
        cluster
            .send(packet(1, 2, b"split-brain"))
            .expect("partition send"),
        ClusterNetworkOutcome::Partitioned
    );
    assert!(cluster.status().quorum);

    let old_epoch = cluster.epoch();
    cluster
        .inject_fault(ClusterFault::FailNode(node(3)))
        .expect("fail node");
    cluster
        .inject_fault(ClusterFault::FailNode(node(2)))
        .expect("lose quorum");
    assert!(!cluster.status().quorum);
    cluster
        .inject_fault(ClusterFault::RecoverNode(node(3)))
        .expect("rejoin node");
    cluster
        .inject_fault(ClusterFault::RecoverNode(node(2)))
        .expect("rejoin second node");
    assert!(cluster.status().quorum);
    assert!(cluster.epoch() > old_epoch);
    assert!(cluster.observe_heartbeat(node(2), heartbeat).is_err());

    cluster
        .inject_fault(ClusterFault::Reconnect {
            left: node(1),
            right: node(2),
        })
        .expect("reconnect nodes");
    assert!(cluster
        .send(packet(1, 2, b"rejoined"))
        .expect("reconnected send")
        .ne(&ClusterNetworkOutcome::Partitioned));
}

#[test]
fn two_and_three_node_synos_boot_has_serial_evidence() {
    let kernel_path = std::env::temp_dir().join(format!(
        "synos-vm-cluster-kernel-{}-{}.bin",
        std::process::id(),
        10_6
    ));
    File::create(&kernel_path)
        .expect("create cluster kernel")
        .write_all(&serial_kernel())
        .expect("write cluster kernel");

    let mut cluster = VmCluster::new(ClusterNetworkConfig::default()).expect("cluster");
    for raw in 1..=3 {
        cluster
            .add_node(
                node(raw),
                VmConfig {
                    memory_size: 8 * 1024 * 1024,
                    kernel_path: Some(kernel_path.clone()),
                    boot_args: "console=serial0".to_string(),
                    ..VmConfig::default()
                },
            )
            .expect("add bootable node");
    }

    for raw in 1..=3 {
        let steps = cluster.run_node(node(raw), 256).expect("bounded node boot");
        assert!(steps > 0);
        assert!(String::from_utf8_lossy(
            &cluster.node(node(raw)).expect("booted node").serial_output()
        )
        .contains("SynOS kernel bootstrap"));
    }
    assert!(cluster.nodes().all(|node| node.executed_steps() > 0));
    assert_eq!(cluster.evidence().serial_output.len(), 3);
    let _ = fs::remove_file(kernel_path);
}

#[test]
fn shared_memory_fixture_covers_mapping_corruption_and_hot_removal() {
    let mut cluster = VmCluster::new(ClusterNetworkConfig::default()).expect("cluster");
    let config = VmConfig {
        memory_size: 4 * 1024 * 1024,
        ..VmConfig::default()
    };
    cluster
        .add_node(node(1), config.clone())
        .expect("node one");
    cluster.add_node(node(2), config).expect("node two");

    let mapping = cluster
        .shared_memory_mut()
        .map(node(1), 0, 64)
        .expect("map shared memory");
    assert_eq!(mapping.epoch, 1);
    cluster
        .shared_memory_mut()
        .write(node(1), 0, b"shared-page")
        .expect("write shared memory");
    assert_eq!(
        cluster
            .shared_memory()
            .read(node(2), 0, 11)
            .expect("read shared memory"),
        b"shared-page"
    );

    cluster.shared_memory_mut().corrupt();
    assert!(cluster.shared_memory().read(node(2), 0, 1).is_err());
    cluster.shared_memory_mut().repair();
    cluster.shared_memory_mut().hot_remove();
    assert!(cluster.shared_memory().read(node(1), 0, 1).is_err());
    cluster.shared_memory_mut().restore();
    assert_eq!(
        cluster
            .shared_memory()
            .read(node(2), 0, 11)
            .expect("read restored memory"),
        b"shared-page"
    );
}

#[test]
fn cluster_workload_faults_recover_and_emit_bounded_evidence() {
    let mut cluster = VmCluster::new(ClusterNetworkConfig::default()).expect("cluster");
    let config = VmConfig {
        memory_size: 4 * 1024 * 1024,
        ..VmConfig::default()
    };
    cluster.add_node(node(1), config).expect("node one");
    cluster
        .add_node(
            node(2),
            VmConfig {
                memory_size: 4 * 1024 * 1024,
                ..VmConfig::default()
            },
        )
        .expect("node two");

    for workload in [
        ClusterWorkload::Ipc,
        ClusterWorkload::FilesystemCommit,
        ClusterWorkload::MemoryFetch,
        ClusterWorkload::Inference,
        ClusterWorkload::MembershipChange,
    ] {
        cluster
            .inject_fault(ClusterFault::KillNodeDuring {
                node: node(2),
                workload,
            })
            .expect("kill node during workload");
        assert!(cluster.send(packet(1, 2, b"in-flight")).is_err());
        cluster.recover_last_fault().expect("recover workload");
        assert_eq!(
            cluster.node(node(2)).expect("node two").state(),
            ClusterNodeState::Running
        );
        assert!(cluster.status().quorum);
    }

    assert!(cluster.fault_records().iter().all(|record| record.recovered));
    let evidence = cluster.evidence();
    assert!(evidence.to_text().contains("network_events="));
    assert!(evidence.pending_packets <= 5);
}
