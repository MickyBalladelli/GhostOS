# Operational health evidence

GhostOS health samples use the bounded `ghostos-observability::HealthReport`.
Each sample names a node and transport, then records:

- health state and degraded mode;
- queue depth and queue capacity;
- dropped packets and retries;
- sample timestamp.

`InspectionRights::HEALTH` permits local inspection. Cluster inspection also
requires `AUDIT_WORLD`. Local reports retain only the capability home node.
`SHOW-HEALTH` exposes the structured aggregate with `CLUSTER` as its optional
scope selector. Output status is normal, degraded, or unsafe from the report.

Publishing a report through `TelemetryStore::publish_health` emits two bounded
audit records per transport: one for state and queue values, and one for packet
drops, retries, and degraded mode. The split keeps every audit event within the
four-field trace-event limit while preserving exact counters.

Regression coverage is in `crates/observability/tests/coverage_59_9.rs` and
`crates/ghostos-inspect/tests/health.rs`. Compile evidence is recorded in
`TODO.md`; test execution remains pending.
