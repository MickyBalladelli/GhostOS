use ghostos_admission::{AdmissionAction, AdmissionPriority, WorkClass};
use ghostos_fabric::NodeId;
use ghostos_inspect::{
    HealthReport, HealthState, HealthTransport, InspectError, InspectionAuthority,
    InspectionRights, InspectionService, OperationalHealth, PrincipalId, TelemetryStore, View,
};

#[test]
fn remote_diagnostics_report_delay_while_local_diagnostics_stay_available() {
    let authority = InspectionAuthority::new(9, 1).unwrap();
    let capability = authority
        .issue(
            PrincipalId::new(1).unwrap(),
            NodeId::new(1).unwrap(),
            InspectionRights::HEALTH.union(InspectionRights::AUDIT_WORLD),
            1_000,
        )
        .unwrap();
    let mut service = InspectionService::new(authority, TelemetryStore::new());
    let mut report = HealthReport::new();
    report
        .push(OperationalHealth::new(
            100,
            1,
            HealthTransport::Http,
            HealthState::Healthy,
            0,
            1,
            0,
            0,
            false,
        )
        .unwrap())
        .unwrap();
    service.provider_mut().publish_health(report);

    let mut leases = [None; 12];
    for lease in &mut leases {
        let outcome = service
            .admission_mut()
            .admit(WorkClass::RemoteDiagnostics, AdmissionPriority::Normal);
        assert_eq!(outcome.action, AdmissionAction::Admitted);
        *lease = outcome.lease();
    }

    assert!(service.health(capability, View::Local, 200).is_ok());
    assert!(matches!(
        service.health(capability, View::Cluster, 200),
        Err(InspectError::Admission(outcome))
            if outcome.action == AdmissionAction::Delayed
                && outcome.class == WorkClass::RemoteDiagnostics
    ));

    for lease in leases.into_iter().flatten() {
        service.admission_mut().finish(lease).unwrap();
    }
}
