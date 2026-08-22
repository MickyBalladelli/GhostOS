use ghostos_ghostfs::{CapacityResource, SynFs};

#[test]
fn capacity_forecast_reports_threshold_and_fragmentation() {
    let mut filesystem = SynFs::<32>::new();
    filesystem
        .create_directory("/system", true)
        .expect("directory");
    filesystem
        .write("/system/state", &[1, 2, 3, 4])
        .expect("state");
    let checkpoint = filesystem.create_checkpoint().expect("checkpoint");
    filesystem
        .write("/system/state", &[5, 6, 7, 8])
        .expect("new state");

    let fragmentation = filesystem
        .fragmentation_report()
        .expect("fragmentation report");
    assert!(fragmentation.capacity_blocks >= fragmentation.allocated_blocks);
    assert!(fragmentation.largest_free_run <= fragmentation.free_blocks);

    let observation = filesystem
        .capacity_observation(CapacityResource::Snapshot, 10, 4096)
        .expect("capacity observation");
    let forecast = observation.forecast(3_600_000_000);
    assert_eq!(forecast.resource, CapacityResource::Snapshot);
    assert!(forecast.warning_free_bytes >= forecast.critical_free_bytes);
    assert!(forecast.gc_bounded);

    filesystem
        .release_checkpoint(checkpoint.id)
        .expect("release checkpoint");
    assert!(filesystem
        .path_diagnostics("/system/state")
        .expect("path diagnostics")
        .retained_versions
        >= 1);
}
