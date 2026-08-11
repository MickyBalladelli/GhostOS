use synos_auth::{CapabilityKey, CapabilityLease, LeaseContext};
use synos_fabric::NodeId;
use synos_kernel::Rights;

fn main() {
    let key = CapabilityKey::new([7; 32]);
    let node = NodeId::new(2).expect("nonzero node id");
    let lease = CapabilityLease::issue(
        key,
        NodeId::LOCAL,
        node,
        node,
        10,
        20,
        1,
        30,
        Rights::READ,
        100,
        200,
        1,
        99,
    )
    .expect("valid lease");
    lease
        .authorize(
            key,
            LeaseContext {
                subject: node,
                audience: node,
                object: 10,
                tenant: 20,
                generation: 1,
                purpose: 30,
                required: Rights::READ,
                now_us: 150,
            },
            1,
        )
        .expect("lease is authorized");
    println!("capability lease authorized for node {}", node.raw());
}
