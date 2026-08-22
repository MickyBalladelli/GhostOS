use ghostos_test_support::crash::{CrashBoundary, CrashDomain, CrashHarness, CrashPoint};

#[test]
fn crash_matrix_is_stable_and_each_target_fires_once() {
    let points: Vec<_> = CrashPoint::matrix().collect();
    assert_eq!(points.len(), 36);

    for point in points {
        let mut harness = CrashHarness::new(Some(point));
        assert_eq!(
            harness.checkpoint(point.domain, point.boundary),
            Err(ghostos_test_support::crash::CrashInjected {
                point,
                sequence: 1,
            })
        );
        assert!(harness.injected());
        assert_eq!(harness.checkpoint(point.domain, point.boundary), Ok(()));
        assert_eq!(harness.events().len(), 2);
    }
}

#[test]
fn seeded_crash_selection_is_replayable_and_logs_shared_boundaries() {
    let first = CrashHarness::from_seed(17);
    let second = CrashHarness::from_seed(17);
    assert_eq!(first.target(), second.target());

    let mut harness = CrashHarness::without_crash();
    for domain in CrashDomain::ALL {
        for boundary in CrashBoundary::ALL {
            harness.checkpoint(domain, boundary).unwrap();
        }
    }
    assert_eq!(harness.events().len(), 36);
    assert_eq!(harness.events()[35].sequence, 36);
}
