//! Fixed-size export for metrics and tracing data.
//!
//! The exporter contains only numeric metric IDs, bounded dimensions, and the
//! existing fixed-size trace record. It never serializes names, paths,
//! payloads, or other free-form data. A caller owns the output buffer, so an
//! unavailable sink cannot make the observability path allocate or block.

use crate::{
    CodecError, MetricAggregate, MetricKind, MetricRegistry, MetricSample, TraceRing,
    MAX_METRIC_AGGREGATES, MAX_METRIC_SAMPLES, JOURNAL_RECORD_SIZE,
};

pub const TELEMETRY_EXPORT_MAGIC: &[u8; 8] = b"SNTELEM1";
pub const TELEMETRY_EXPORT_VERSION: u16 = 1;
pub const TELEMETRY_EXPORT_HEADER_BYTES: usize = 64;
pub const METRIC_EXPORT_RECORD_BYTES: usize = 64;
pub const TRACE_EXPORT_RECORD_BYTES: usize = JOURNAL_RECORD_SIZE;
pub const MAX_TELEMETRY_EXPORT_METRICS: usize = MAX_METRIC_SAMPLES + MAX_METRIC_AGGREGATES;
pub const MAX_TELEMETRY_EXPORT_TRACES: usize = crate::GLOBAL_TRACE_CAPACITY;

const METRIC_SAMPLE_RECORD: u8 = 1;
const METRIC_AGGREGATE_RECORD: u8 = 2;
const TELEMETRY_CHECKSUM_OFFSET: usize = 48;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetryExportError {
    BufferTooSmall { required: usize },
    InvalidExport,
    Codec(CodecError),
}

/// A zero-allocation exporter for one metric registry and one trace ring.
///
/// The output is bounded by [`Self::max_encoded_len`]. `export` returns the
/// used prefix; the remaining bytes in a maximum-sized buffer are zeroed.
pub struct TelemetryExporter<
    'a,
    const METRIC_CAPACITY: usize = MAX_METRIC_SAMPLES,
    const TRACE_CAPACITY: usize = { crate::GLOBAL_TRACE_CAPACITY },
> {
    metrics: &'a MetricRegistry<METRIC_CAPACITY>,
    traces: &'a TraceRing<TRACE_CAPACITY>,
}

impl<'a, const METRIC_CAPACITY: usize, const TRACE_CAPACITY: usize>
    TelemetryExporter<'a, METRIC_CAPACITY, TRACE_CAPACITY>
{
    pub const fn new(
        metrics: &'a MetricRegistry<METRIC_CAPACITY>,
        traces: &'a TraceRing<TRACE_CAPACITY>,
    ) -> Self {
        Self { metrics, traces }
    }

    pub const fn max_encoded_len() -> usize {
        TELEMETRY_EXPORT_HEADER_BYTES
            + MAX_TELEMETRY_EXPORT_METRICS * METRIC_EXPORT_RECORD_BYTES
            + MAX_TELEMETRY_EXPORT_TRACES * TRACE_EXPORT_RECORD_BYTES
    }

    /// Returns the maximum size, not a race-prone live size.
    pub const fn encoded_len(&self) -> usize {
        Self::max_encoded_len()
    }

    pub fn export(&self, destination: &mut [u8]) -> Result<usize, TelemetryExportError> {
        let required = Self::max_encoded_len();
        if destination.len() < required {
            return Err(TelemetryExportError::BufferTooSmall { required })
        }
        destination[..required].fill(0);

        let mut metric_offset = TELEMETRY_EXPORT_HEADER_BYTES;
        let mut sample_count = 0usize;
        for sample in self.metrics.samples() {
            if sample_count + self.metrics.aggregates().count() >= MAX_TELEMETRY_EXPORT_METRICS {
                break
            }
            encode_metric_sample(
                sample,
                &mut destination[metric_offset..metric_offset + METRIC_EXPORT_RECORD_BYTES],
            );
            metric_offset += METRIC_EXPORT_RECORD_BYTES;
            sample_count += 1;
        }

        let mut aggregate_count = 0usize;
        for aggregate in self.metrics.aggregates() {
            if sample_count + aggregate_count >= MAX_TELEMETRY_EXPORT_METRICS {
                break
            }
            encode_metric_aggregate(
                aggregate,
                &mut destination[metric_offset..metric_offset + METRIC_EXPORT_RECORD_BYTES],
            );
            metric_offset += METRIC_EXPORT_RECORD_BYTES;
            aggregate_count += 1;
        }

        let trace_limit = MAX_TELEMETRY_EXPORT_TRACES.min(TRACE_CAPACITY);
        let trace_bytes = trace_limit * TRACE_EXPORT_RECORD_BYTES;
        let trace_end = metric_offset + trace_bytes;
        let trace_count = self
            .traces
            .encode_recent(&mut destination[metric_offset..trace_end])
            .map_err(TelemetryExportError::Codec)?
            / TRACE_EXPORT_RECORD_BYTES;

        let metric_count = sample_count + aggregate_count;
        destination[..8].copy_from_slice(TELEMETRY_EXPORT_MAGIC);
        destination[8..10].copy_from_slice(&TELEMETRY_EXPORT_VERSION.to_le_bytes());
        destination[10..12]
            .copy_from_slice(&(TELEMETRY_EXPORT_HEADER_BYTES as u16).to_le_bytes());
        destination[12..16].copy_from_slice(&(metric_count as u32).to_le_bytes());
        destination[16..20].copy_from_slice(&(trace_count as u32).to_le_bytes());
        destination[20..28].copy_from_slice(&self.metrics.dropped().to_le_bytes());
        destination[28..36].copy_from_slice(&self.traces.dropped().to_le_bytes());
        destination[36..38].copy_from_slice(&(METRIC_EXPORT_RECORD_BYTES as u16).to_le_bytes());
        destination[38..40].copy_from_slice(&(TRACE_EXPORT_RECORD_BYTES as u16).to_le_bytes());
        destination[40..44].copy_from_slice(&(sample_count as u32).to_le_bytes());
        destination[44..48].copy_from_slice(&(aggregate_count as u32).to_le_bytes());

        let used = metric_offset + trace_count * TRACE_EXPORT_RECORD_BYTES;
        let checksum = telemetry_checksum(&destination[..used]);
        destination[TELEMETRY_CHECKSUM_OFFSET..TELEMETRY_CHECKSUM_OFFSET + 8]
            .copy_from_slice(&checksum.to_le_bytes());
        Ok(used)
    }
}

fn encode_metric_sample(sample: MetricSample, destination: &mut [u8]) {
    encode_metric_header(sample.name, sample.kind, METRIC_SAMPLE_RECORD, destination);
    destination[4..12].copy_from_slice(&sample.timestamp.to_le_bytes());
    encode_metric_value(sample.value, sample.dimensions, destination);
}

fn encode_metric_aggregate(aggregate: MetricAggregate, destination: &mut [u8]) {
    encode_metric_header(
        aggregate.name,
        aggregate.kind,
        METRIC_AGGREGATE_RECORD,
        destination,
    );
    destination[4..12].copy_from_slice(&aggregate.observations.to_le_bytes());
    encode_metric_value(aggregate.value, aggregate.dimensions, destination);
}

fn encode_metric_header(
    name: u16,
    kind: MetricKind,
    record_kind: u8,
    destination: &mut [u8],
) {
    destination[..2].copy_from_slice(&name.to_le_bytes());
    destination[2] = kind as u8;
    destination[3] = record_kind;
}

fn encode_metric_value(
    value: u64,
    dimensions: crate::TelemetryDimensions,
    destination: &mut [u8],
) {
    destination[12..20].copy_from_slice(&value.to_le_bytes());
    destination[20..36].copy_from_slice(&dimensions.cluster.to_le_bytes());
    destination[36..40].copy_from_slice(&dimensions.node.to_le_bytes());
    destination[40] = dimensions.transport;
    destination[41..49].copy_from_slice(&dimensions.workload.to_le_bytes());
    destination[49..51].copy_from_slice(&dimensions.operation.to_le_bytes());
}

/// Validate an export without allocating or trusting record counts.
pub fn validate_telemetry_export(source: &[u8]) -> Result<usize, TelemetryExportError> {
    if source.len() < TELEMETRY_EXPORT_HEADER_BYTES
        || &source[..8] != TELEMETRY_EXPORT_MAGIC
        || u16::from_le_bytes([source[8], source[9]]) != TELEMETRY_EXPORT_VERSION
        || u16::from_le_bytes([source[10], source[11]]) as usize != TELEMETRY_EXPORT_HEADER_BYTES
        || u16::from_le_bytes([source[36], source[37]]) as usize != METRIC_EXPORT_RECORD_BYTES
        || u16::from_le_bytes([source[38], source[39]]) as usize != TRACE_EXPORT_RECORD_BYTES
    {
        return Err(TelemetryExportError::InvalidExport)
    }

    let metric_count = u32::from_le_bytes(read_four(source, 12)?) as usize;
    let trace_count = u32::from_le_bytes(read_four(source, 16)?) as usize;
    let sample_count = u32::from_le_bytes(read_four(source, 40)?) as usize;
    let aggregate_count = u32::from_le_bytes(read_four(source, 44)?) as usize;
    if metric_count > MAX_TELEMETRY_EXPORT_METRICS
        || trace_count > MAX_TELEMETRY_EXPORT_TRACES
        || sample_count > MAX_METRIC_SAMPLES
        || aggregate_count > MAX_METRIC_AGGREGATES
        || metric_count != sample_count.saturating_add(aggregate_count)
    {
        return Err(TelemetryExportError::InvalidExport)
    }

    let metric_bytes = metric_count
        .checked_mul(METRIC_EXPORT_RECORD_BYTES)
        .ok_or(TelemetryExportError::InvalidExport)?;
    let trace_bytes = trace_count
        .checked_mul(TRACE_EXPORT_RECORD_BYTES)
        .ok_or(TelemetryExportError::InvalidExport)?;
    let required = TELEMETRY_EXPORT_HEADER_BYTES
        .checked_add(metric_bytes)
        .and_then(|length| length.checked_add(trace_bytes))
        .ok_or(TelemetryExportError::InvalidExport)?;
    if source.len() < required {
        return Err(TelemetryExportError::InvalidExport)
    }
    if u64::from_le_bytes(
        source[TELEMETRY_CHECKSUM_OFFSET..TELEMETRY_CHECKSUM_OFFSET + 8]
            .try_into()
            .unwrap(),
    ) != telemetry_checksum(&source[..required])
        || source[56..64].iter().any(|byte| *byte != 0)
    {
        return Err(TelemetryExportError::InvalidExport)
    }

    for index in 0..metric_count {
        let offset = TELEMETRY_EXPORT_HEADER_BYTES + index * METRIC_EXPORT_RECORD_BYTES;
        validate_metric_record(&source[offset..offset + METRIC_EXPORT_RECORD_BYTES])?;
    }
    let trace_offset = TELEMETRY_EXPORT_HEADER_BYTES + metric_bytes;
    for index in 0..trace_count {
        let offset = trace_offset + index * TRACE_EXPORT_RECORD_BYTES;
        crate::decode_record(&source[offset..offset + TRACE_EXPORT_RECORD_BYTES])
            .map_err(TelemetryExportError::Codec)?;
    }
    if source[required..].iter().any(|byte| *byte != 0) {
        return Err(TelemetryExportError::InvalidExport)
    }
    Ok(required)
}

fn validate_metric_record(source: &[u8]) -> Result<(), TelemetryExportError> {
    let name = u16::from_le_bytes([source[0], source[1]]);
    MetricKind::from_raw(source[2]).ok_or(TelemetryExportError::InvalidExport)?;
    let record_kind = source[3];
    let count_or_timestamp = u64::from_le_bytes(source[4..12].try_into().unwrap());
    let node = u32::from_le_bytes(source[36..40].try_into().unwrap());
    if name == 0 || node == 0 || count_or_timestamp == 0 {
        return Err(TelemetryExportError::InvalidExport)
    }
    if !matches!(record_kind, METRIC_SAMPLE_RECORD | METRIC_AGGREGATE_RECORD) {
        return Err(TelemetryExportError::InvalidExport)
    }
    if source[51..].iter().any(|byte| *byte != 0) {
        return Err(TelemetryExportError::InvalidExport)
    }
    Ok(())
}

fn read_four(source: &[u8], offset: usize) -> Result<[u8; 4], TelemetryExportError> {
    source
        .get(offset..offset + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(TelemetryExportError::InvalidExport)
}

fn telemetry_checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for (index, byte) in bytes.iter().enumerate() {
        if (TELEMETRY_CHECKSUM_OFFSET..TELEMETRY_CHECKSUM_OFFSET + 8).contains(&index) {
            continue
        }
        hash = (hash ^ *byte as u64).wrapping_mul(0x100000001b3);
    }
    hash
}
