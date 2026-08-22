use ghostos_observability::{SloKind, SloObservation, SloReport, SLO_COUNT};

fn complete_report(bad_events: u64) -> SloReport {
    let mut report = SloReport::new(100);
    for kind in SloKind::ALL {
        report
            .record(SloObservation::new(kind, 1, 100, 1_000, bad_events).unwrap())
            .unwrap();
    }
    report
}

#[test]
fn all_nine_slos_report_consumption_and_release_readiness() {
    let report = complete_report(0);
    let measurement = report.measurement_at(SloKind::Boot, 101).unwrap();
    assert_eq!(measurement.allowed_bad_events, 1);
    assert_eq!(measurement.consumed_per_million, 0);
    assert!(report.status(101).release_ready());
    assert_eq!(report.observations().count(), SLO_COUNT);
}

#[test]
fn stale_evidence_and_exhausted_budget_are_not_release_ready() {
    let report = complete_report(2);
    let measurement = report.measurement_at(SloKind::Rpc, 101).unwrap();
    assert!(measurement.budget_exhausted());
    assert_eq!(measurement.consumed_per_million, 1_000_000);
    assert_eq!(report.status(86_400_000_101).stale, SLO_COUNT);
    assert!(!report.status(101).release_ready());
}
