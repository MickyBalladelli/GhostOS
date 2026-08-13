use synos_observability::{
    EventKind, Level, MetricKind, MetricRegistry, MetricSample, TelemetryDimensions,
    TelemetryExportError, TelemetryExporter, TraceEvent, TraceRing, validate_telemetry_export,
};

#[test]
fn bounded_export_contains_folded_metrics_and_recent_traces() {
    let dimensions = TelemetryDimensions::new(7, 2, 1, 9, 11);
    let mut metrics = MetricRegistry::<1>::new();
    metrics
        .record_bounded(MetricSample::new(1, MetricKind::Counter, 1, 4, dimensions))
        .unwrap();
    metrics
        .record_bounded(MetricSample::new(2, MetricKind::Counter, 2, 6, dimensions))
        .unwrap();

    let traces = TraceRing::<2>::new();
    traces.push(TraceEvent::new(Level::Info, EventKind::Kernel).at(3));
    traces.push(TraceEvent::new(Level::Warn, EventKind::Kernel).at(4));
    traces.push(TraceEvent::new(Level::Error, EventKind::Kernel).at(5));

    let exporter = TelemetryExporter::new(&metrics, &traces);
    let mut output = vec![0; exporter.encoded_len()];
    let length = exporter.export(&mut output).unwrap();

    assert_eq!(validate_telemetry_export(&output[..length]), Ok(length));
    assert_eq!(u32::from_le_bytes(output[12..16].try_into().unwrap()), 2);
    assert_eq!(u32::from_le_bytes(output[16..20].try_into().unwrap()), 2);
    assert_eq!(u32::from_le_bytes(output[44..48].try_into().unwrap()), 1);
    assert_eq!(u64::from_le_bytes(output[28..36].try_into().unwrap()), 1);
}

#[test]
fn bounded_export_rejects_small_buffers_and_tampering() {
    let metrics = MetricRegistry::<1>::new();
    let traces = TraceRing::<2>::new();
    let exporter = TelemetryExporter::new(&metrics, &traces);
    let mut output = vec![0; exporter.encoded_len() - 1];

    assert_eq!(
        exporter.export(&mut output),
        Err(TelemetryExportError::BufferTooSmall { required: exporter.encoded_len() })
    );

    let mut output = vec![0; exporter.encoded_len()];
    let length = exporter.export(&mut output).unwrap();
    output[0] ^= 1;
    assert_eq!(
        validate_telemetry_export(&output[..length]),
        Err(TelemetryExportError::InvalidExport)
    );
}
