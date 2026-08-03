use synos_agent_bridge::{AgentBridge, AgentTaskScope, Error};
use synos_auth::{CapabilityKey, CryptographicCapability, TransportRights};
use synos_fabric::NodeId;
use synos_kernel::Rights;

#[test]
fn task_capabilities_are_attenuated_and_single_use() {
    let issuer = NodeId::LOCAL;
    let subject = NodeId::new(2).unwrap();
    let key = CapabilityKey::new([8; 32]);
    let parent = CryptographicCapability::issue(
        key,
        issuer,
        subject,
        77,
        Rights::READ.union(Rights::WRITE).union(Rights::EXECUTE),
        TransportRights::LAYER2,
        0,
        100,
        1,
        1,
    )
    .unwrap();
    let scope = AgentTaskScope::new(77, Rights::EXECUTE, TransportRights::LAYER2, 30).unwrap();
    let mut bridge = AgentBridge::<2>::new(issuer, key, 9);
    let task = bridge
        .mint_task_capability(&parent, subject, scope, 20, 10)
        .unwrap();
    assert_eq!(bridge.capabilities().active_grants(10), 1);
    bridge
        .capabilities_mut()
        .consume(&task, subject, Rights::EXECUTE, TransportRights::LAYER2, 11)
        .unwrap();
    assert!(matches!(
        bridge
            .capabilities_mut()
            .consume(&task, subject, Rights::EXECUTE, TransportRights::LAYER2, 11),
        Err(Error::AlreadyConsumed)
    ));
}

#[test]
fn task_scope_rejects_larger_rights_and_expired_parent() {
    let issuer = NodeId::LOCAL;
    let subject = NodeId::new(2).unwrap();
    let key = CapabilityKey::new([5; 32]);
    let parent = CryptographicCapability::issue(
        key,
        issuer,
        subject,
        9,
        Rights::READ,
        TransportRights::LAYER2,
        0,
        5,
        1,
        1,
    )
    .unwrap();
    let mut bridge = AgentBridge::<1>::new(issuer, key, 1);
    let write_scope = AgentTaskScope::new(9, Rights::WRITE, TransportRights::LAYER2, 10).unwrap();
    assert!(matches!(
        bridge.mint_task_capability(&parent, subject, write_scope, 5, 1),
        Err(Error::ScopeViolation)
    ));
    let read_scope = AgentTaskScope::new(9, Rights::READ, TransportRights::LAYER2, 10).unwrap();
    assert!(matches!(
        bridge.mint_task_capability(&parent, subject, read_scope, 5, 5),
        Err(Error::Token(_))
    ));
}
