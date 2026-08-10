use synos_fabric::NodeId;
use synos_inspect::{
    HealthReport, HealthState, HealthTransport, InspectError, InspectionAuthority,
    InspectionRights, InspectionService, OperationalHealth, PrincipalId, TelemetryStore, View,
};

fn sample(node: u32, transport: HealthTransport, state: HealthState) -> OperationalHealth {
    OperationalHealth::new(100, node, transport, state, 2, 8, 3, 4, state != HealthState::Healthy)
        .unwrap()
}

#[test]
fn health_inspection_filters_local_nodes_and_requires_cluster_rights() {
    let authority = InspectionAuthority::new(9, 1).unwrap();
    let local_capability = authority
        .issue(
            PrincipalId::new(1).unwrap(),
            NodeId::new(1).unwrap(),
            InspectionRights::HEALTH,
            1_000,
        )
        .unwrap();
    let cluster_capability = authority
        .issue(
            PrincipalId::new(1).unwrap(),
            NodeId::new(1).unwrap(),
            InspectionRights::HEALTH.union(InspectionRights::AUDIT_WORLD),
            1_000,
        )
        .unwrap();

    let mut report = HealthReport::new();
    report.push(sample(1, HealthTransport::Http, HealthState::Healthy)).unwrap();
    report.push(sample(2, HealthTransport::Mesh, HealthState::Degraded)).unwrap();
    let mut service = InspectionService::new(authority, TelemetryStore::new());
    service.provider_mut().publish_health(report);

    let local = service.health(local_capability, View::Local, 200).unwrap();
    assert_eq!(local.samples().count(), 1);
    assert_eq!(local.healthy_count(), 1);
    assert_eq!(local.degraded_count(), 0);

    assert!(matches!(
        service.health(local_capability, View::Cluster, 200),
        Err(InspectError::AccessDenied)
    ));
    let cluster = service.health(cluster_capability, View::Cluster, 200).unwrap();
    assert_eq!(cluster.samples().count(), 2);
    assert_eq!(cluster.degraded_count(), 1);
    assert!(cluster.degraded_mode());
}
