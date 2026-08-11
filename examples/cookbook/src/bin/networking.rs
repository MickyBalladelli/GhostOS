use synos_netd::{
    install_core_network_rules, FirewallPolicy, Ipv4Cidr, PacketQueue, PortRange,
};

fn main() {
    let network = Ipv4Cidr::new([192, 168, 1, 0], 24).expect("valid CIDR");
    assert!(network.contains([192, 168, 1, 10]));
    assert!(PortRange::new(8000, 8080).expect("valid port range").contains(8080));

    let mut policy = FirewallPolicy::<32>::new();
    install_core_network_rules(&mut policy).expect("baseline rules fit");

    let mut queue = PacketQueue::<4, 1500>::new();
    let mut packet = queue.reserve().expect("packet slot available");
    packet.buffer()[..4].copy_from_slice(b"ping");
    packet.commit(4).expect("frame fits MTU");
    let received = queue.dequeue().expect("frame is ready");
    assert_eq!(received.frame(), b"ping");
    println!("network policy: {} rules; one packet moved", policy.len());
}
