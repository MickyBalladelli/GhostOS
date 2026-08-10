# Partition and clock-skew recovery contract

Recovery has one ordering rule: no mutable storage, memory lease, or coherent
page ownership may be released until the failed node is fenced or an equivalent
isolation proof is present.

The fabric path enforces this in `recover_failed_node`: it checks
`NodeIsolation` first, then marks the node failed, releases its memory leases,
and removes its coherent-page ownership. A failed node with a mirror resolves
to the mirror only after the failed-node mark is committed.

The storage path uses two phases. `FailureController::apply(Fence)` records the
fenced state, and `Recover` is the only action that invokes shared-memory,
storage, job, capability, DLM, SynFS, log, reservation, and workload release
hooks. A partition or clock-skew observation cannot skip the fence phase.

Clock decisions use local monotonic sample time and received heartbeat age.
Peer-supplied timestamps do not extend a timeout. This prevents a partitioned
or clock-skewed node from retaining ownership by claiming a future timestamp.

Direct regressions:

- `crates/fabric/tests/coverage_59_7.rs`
  - `partition_recovery_does_not_release_memory_before_fencing`
  - `future_peer_timestamp_cannot_extend_partition_recovery_deadline`
- `crates/synos-storaged/tests/coverage_59_5.rs`
  - `partition_fences_before_storage_and_memory_release`
  - `clock_skew_is_detected_at_local_sample_time_before_recovery`
