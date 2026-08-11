use synos_fabric::cluster::{Heartbeat, HeartbeatMonitor, NodeState};
use synos_fabric::NodeId;

fn main() {
    let peer = NodeId::new(2).expect("nonzero node id");
    let mut monitor = HeartbeatMonitor::<4>::new(NodeId::LOCAL, 100, 3, 0)
        .expect("bounded heartbeat policy");
    monitor.add_node(peer, 0).expect("cluster has capacity");
    monitor
        .observe(
            Heartbeat {
                node: peer,
                sequence: 1,
                sent_at_us: 100,
            },
            100,
        )
        .expect("heartbeat is fresh");
    assert_eq!(monitor.state(peer), Some(NodeState::Alive));
    println!("cluster peer {} is alive", peer.raw());
}
