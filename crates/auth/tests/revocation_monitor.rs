use ghostos_auth::{
    RevocationCache, RevocationKey, RevocationMonitor, RevocationMonitorError,
    REVOCATION_CACHE_COUNT,
};

const CACHES: [RevocationCache; REVOCATION_CACHE_COUNT] = RevocationCache::ALL;

#[test]
fn all_cache_layers_report_bounded_propagation_latency() {
    let key = RevocationKey::new(7, 41).unwrap();
    let mut monitor = RevocationMonitor::<8>::new(100);
    let notice = monitor.revoke(key, 2, 1_000).unwrap();

    for (index, cache) in CACHES.into_iter().enumerate() {
        let observation = monitor.observe(cache, notice, 1_010 + index as u64).unwrap();
        assert_eq!(observation.key, key);
        assert_eq!(observation.epoch, 2);
    }

    let report = monitor.report(1_100);
    assert_eq!(report.total_events, 1);
    assert_eq!(report.completed_events, 1);
    assert_eq!(report.pending_events, 0);
    assert_eq!(report.maximum_latency_us, 16);
    assert_eq!(report.late_observations, 0);
}

#[test]
fn refresh_reconnect_and_authorization_fence_stale_entries() {
    let key = RevocationKey::new(7, 41).unwrap();
    let mut monitor = RevocationMonitor::<4>::new(100);
    let notice = monitor.revoke(key, 2, 1_000).unwrap();

    assert_eq!(
        monitor.refresh(RevocationCache::Storage, key, 1, 1_010),
        Err(RevocationMonitorError::StaleEpoch)
    );
    assert_eq!(
        monitor.reconnect(RevocationCache::Client, key, 1, 1_011),
        Err(RevocationMonitorError::StaleEpoch)
    );
    assert_eq!(
        monitor.authorize(RevocationCache::Network, key, 1),
        Err(RevocationMonitorError::StaleEpoch)
    );
    assert_eq!(
        monitor.authorize(RevocationCache::Network, key, 2),
        Err(RevocationMonitorError::CacheNotSynchronized)
    );

    monitor.refresh(RevocationCache::Network, key, notice.epoch, 1_012).unwrap();
    assert_eq!(monitor.authorize(RevocationCache::Network, key, 2), Ok(()));
    assert_eq!(monitor.report(1_020).stale_rejections, 4);
}

#[test]
fn late_observation_is_measured_without_reopening_old_authority() {
    let key = RevocationKey::new(7, 41).unwrap();
    let mut monitor = RevocationMonitor::<2>::new(10);
    let notice = monitor.revoke(key, 2, 100).unwrap();

    let observation = monitor.observe(RevocationCache::Kernel, notice, 125).unwrap();
    assert!(observation.late);
    assert_eq!(monitor.authorize(RevocationCache::Kernel, key, 1), Err(RevocationMonitorError::StaleEpoch));
    assert_eq!(monitor.report(125).maximum_latency_us, 25);
    assert_eq!(monitor.report(125).late_observations, 1);
}
