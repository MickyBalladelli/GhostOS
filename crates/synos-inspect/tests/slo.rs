use synos_fabric::NodeId;
use synos_inspect::{
    InspectionAuthority, InspectionRights, InspectionService, PrincipalId, SloKind,
    SloObservation, SloReport, TelemetryStore, View,
};

fn report() -> SloReport {
    let mut report = SloReport::new(100);
    for kind in SloKind::ALL {
        report
            .record(SloObservation::new(kind, 1, 100, 1_000, 0).unwrap())
            .unwrap();
    }
    report
}

#[test]
fn slo_inspection_requires_slo_rights_and_returns_budget_report() {
    let authority = InspectionAuthority::new(9, 1).unwrap();
    let capability = authority
        .issue(
            PrincipalId::new(1).unwrap(),
            NodeId::new(1).unwrap(),
            InspectionRights::SLO,
            1_000,
        )
        .unwrap();
    let mut service = InspectionService::new(authority, TelemetryStore::new());
    service.provider_mut().publish_slo(report());

    let report = service.slo(capability, View::Local, 101).unwrap();
    assert!(report.status(101).release_ready());
    assert_eq!(report.observations().count(), 9);
}
