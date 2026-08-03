use synos_agentd::{
    AgentError, ContextBus, CpuEmbedding, IngestOptions, SourceKind, SourceRef,
};
use synos_auth::{CapabilityKey, TransportRights};
use synos_fabric::NodeId;
use synos_kernel::Rights;

#[test]
fn semantic_bus_embeds_queries_resolves_zero_copy_and_expires() {
    let owner = NodeId::new(2).unwrap();
    let mut bus = ContextBus::<4, 4, 4, 128>::new(
        NodeId::LOCAL,
        CapabilityKey::new([3; 32]),
        synos_agentd::CONTEXT_RESOURCE,
    )
    .unwrap();
    let token = bus
        .issue_capability(
            owner,
            Rights::READ.union(Rights::MAP),
            TransportRights::LAYER2,
            0,
            100,
        )
        .unwrap();
    let source = SourceRef::new(SourceKind::KvState, 7).unwrap();
    let options = IngestOptions::new(owner, 0, 100).unwrap().with_ttl(50).with_decay(20);
    bus.submit(source, b"/context/a", b"distributed memory", options)
        .unwrap();
    let report = bus.poll(&mut CpuEmbedding, 2, 1).unwrap();
    assert_eq!(report.processed, 1);

    let mut hits = [None; 2];
    let count = bus
        .query(&token, owner, TransportRights::LAYER2, &[1.0; 4], 2, 2, &mut hits)
        .unwrap();
    assert_eq!(count, 1);
    let handle = hits[0].unwrap().handle;
    let view = bus
        .resolve(&token, owner, TransportRights::LAYER2, handle, 2)
        .unwrap();
    assert_eq!(view.bytes, b"distributed memory");
    assert_eq!(view.source, source);
    assert!(matches!(bus.resolve(&token, owner, TransportRights::LAYER2, handle, 51), Err(AgentError::StaleHandle)));
    assert_eq!(bus.collect_expired(51, 4), 1);
}

#[test]
fn semantic_bus_rejects_bad_inputs_and_revoked_capabilities() {
    let owner = NodeId::new(2).unwrap();
    assert!(matches!(SourceRef::new(SourceKind::KvState, 0), Err(AgentError::InvalidInput)));
    assert!(IngestOptions::new(owner, 10, 10).is_err());
    let mut bus = ContextBus::<2, 2, 2, 8>::new(NodeId::LOCAL, CapabilityKey::new([4; 32]), 9).unwrap();
    let token = bus
        .issue_capability(owner, Rights::READ, TransportRights::LAYER2, 0, 100)
        .unwrap();
    bus.revoke_all();
    let mut output = [None; 1];
    assert!(matches!(bus.query(&token, owner, TransportRights::LAYER2, &[1.0; 2], 1, 1, &mut output), Err(AgentError::AccessDenied)));
}
