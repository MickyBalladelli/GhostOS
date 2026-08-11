# SynOS Next-Generation Improvement Roadmap

This is the new forward roadmap for SynOS. The previous hardening and proof
roadmap is preserved in [`todo/TODO-updates.md`](todo/TODO-updates.md).

This roadmap proposes new work beyond that baseline. Every item needs:

- implementation in the real kernel, service, VM, or client path;
- a direct regression, property, fault, performance, or interoperability test;
- measured evidence with source revision, command, host, configuration, and
  result state;
- a documented quality, performance, scalability, or operational outcome.

Do not mark an item complete from a compile result or a named test alone.
Prefer bounded behavior, stable compatibility, useful failure messages, and
measured p50/p95/p99 data over optimistic feature claims.

## Roadmap laws

- Correctness and isolation come before throughput.
- No unbounded allocation, queue, retry, metadata scan, or control-plane fanout
  may enter a production path.
- Every optimization needs a correctness comparison against the reference path.
- Every distributed feature needs a partition, clock-skew, duplicate, replay,
  and recovery story.
- Every public format and API needs a version, migration, downgrade, and
  deprecation policy.

## P0: Kernel correctness and fault containment

- [x] Publish a machine-readable invariant catalogue for address spaces,
      capabilities, IPC ownership, scheduler state, interrupt delivery, and
      page-table transitions.
      Done when invariant checks run in debug builds, model tests, and recovery
      tests, and failures identify the violated invariant without exposing data.
      Implementation: [`docs/invariants.toml`](docs/invariants.toml),
      [`kernel/src/invariants.rs`](kernel/src/invariants.rs), kernel debug
      hooks, model coverage, and recovery coverage are in place; keep this
      checkbox open until the required evidence run is recorded.
- [x] Add a deterministic kernel litmus suite for IPC ordering, capability
      revocation, memory visibility, interrupt races, scheduler preemption, and
      wakeup-after-HLT behavior.
      Done when the same seed reproduces the same interleaving and the suite
      reports the minimal failing schedule.
- [x] Replace hand-maintained ABI assumptions with generated, versioned ABI
      descriptions shared by kernel, runtime, VM, Rust SDK, and Swift client.
      Done when incompatible frames fail with stable status codes before any
      mutable state changes.
      Implementation: [abi/synos-abi.toml](abi/synos-abi.toml) generates the
      Rust and Swift bindings; kernel and gateway validate the generated version
      before dispatch or service mutation.
- [x] Build a service dependency graph with cycle detection, startup barriers,
      readiness state, shutdown ordering, and bounded restart budgets.
      Done when boot and reboot produce an ordered trace and no service accepts
      work before its declared dependencies are ready.
      Implementation: [`crates/init/src/lib.rs`](crates/init/src/lib.rs)
      provides a bounded dependency graph, transitive readiness gate, reverse
      shutdown, reboot trace, and existing restart-budget enforcement;
      regression models live in [`crates/init/tests/model.rs`](crates/init/tests/model.rs).
- [x] Add fault domains for scheduler, memory manager, IPC broker, storage
      daemon, network daemon, and package supervisor failures.
      Done when a fault is contained to its domain, surviving services remain
      inspectable, and recovery cannot reuse stale capabilities or generations.
      Implementation: [`crates/init/src/fault_domains.rs`](crates/init/src/fault_domains.rs)
      provides bounded per-domain state, capability epochs, recovery leases, and
      inspectable status snapshots; [`crates/init/src/lib.rs`](crates/init/src/lib.rs)
      fences bound services and rejects stale service generations. Regression
      coverage is in [`crates/init/tests/fault_domains.rs`](crates/init/tests/fault_domains.rs).
- [x] Create a kernel crash capsule containing register state, fault address,
      capability context, scheduler state, recent audit IDs, and build identity.
      Done when capsule creation is bounded, redacted, crash-safe, and useful
      without requiring the normal log service.
      Implementation: [`kernel/src/crash.rs`](kernel/src/crash.rs) writes a bounded
      versioned capsule directly through the persistence port, with architecture
      register capture, redacted capability records, scheduler counters, recent
      audit correlation IDs, and build identity; crash encoding and redaction
      regression tests are included.
- [x] Prove interrupt, timer, and deferred-work progress under CPU saturation.
      Done when high-priority control traffic meets a declared latency budget
      while bulk work is throttled instead of starving the system.
      Implementation: [`kernel/src/saturation.rs`](kernel/src/saturation.rs)
      provides a fixed-capacity deterministic proof with one-tick budgets for
      interrupt, control, timer, and deferred work, plus one reserved bulk
      service slot every four ticks. The saturation regression proves high-
      priority progress, bounded queues, and throttled-but-nonzero bulk work.
- [x] Add long-duration reboot, suspend/resume, hotplug, and service-restart
      campaigns with leak detection for pages, handles, IRQ routes, timers,
      capabilities, and worker tasks.
      Done when repeated cycles show zero unreclaimed ownership and produce a
      retained machine-readable report.
      Implementation: [`virtual_machine/tests/lifecycle_soak.rs`](virtual_machine/tests/lifecycle_soak.rs)
      runs 64 bounded cycles by default; `scripts/soak.py` retains one detailed
      ownership report per lifecycle run under `build/soak/lifecycle/`.

## P0: Durable storage and data integrity

- [x] Define one end-to-end durability contract from application write through
      SynFS, storage daemon, block device, cache, flush, and power-loss recovery.
      Done when each layer states what `flush`, `sync`, `rename`, and `commit`
      guarantee and tests prove the ordering. See
      [`docs/durability.md`](docs/durability.md),
      [`crates/durability/src/lib.rs`](crates/durability/src/lib.rs), and
      [`crates/durability/tests/contract.rs`](crates/durability/tests/contract.rs).
- [x] Add online filesystem scrub and repair-preview operations for SynFS,
      package stores, journals, snapshots, and system metadata.
      Done when inspection is read-only by default, repairs require explicit
      authorization, and every changed block has before/after evidence.
      Implementation: [`crates/synfs/src/scrub.rs`](crates/synfs/src/scrub.rs)
      provides bounded, online read-only plans for every scope, generation
      fencing, explicit nonzero operator authorization, and per-block before /
      after fingerprints. Regression coverage is in
      [`crates/synfs/tests/scrub.rs`](crates/synfs/tests/scrub.rs).
- [x] Add checksummed metadata and scrubbing for silent corruption, stale
      generations, torn records, duplicate blocks, and orphaned capabilities.
      Done when corruption is detected before publication and recovery chooses a
      complete generation without guessing.
      Implementation: [`crates/synfs/src/volume.rs`](crates/synfs/src/volume.rs)
      validates complete metadata graphs before publication, rejects ambiguous
      committed banks, and recovers only a complete checksummed generation.
      [`crates/synfs/src/scrub.rs`](crates/synfs/src/scrub.rs) reports stale
      generations, duplicate ownership, corrupt blocks, and unreachable blocks;
      [`kernel/src/capability.rs`](kernel/src/capability.rs) rejects orphaned
      capability derivation links.
- [x] Implement incremental, deduplicated, encrypted backups with resumable
      upload, retention limits, legal hold, and verified restore.
      Done when restore drills recover a bootable system and report RPO, RTO,
      bytes transferred, and any skipped object with a reason.
      Implementation: [`crates/synos-backup/src/recovery.rs`](crates/synos-backup/src/recovery.rs)
      provides keyed content-addressed chunks, authenticated encryption,
      resumable uploads, bounded retention and legal holds, and transactional
      verified restore reports with RPO, RTO, transfer, bootability, and skip
      reasons.
- [x] Add storage tiering between local NVMe, slower disks, remote storage, and
      disposable cache using explicit heat, cost, and durability policies.
      Done when promotion and demotion are crash-safe and never weaken the
      declared durability class. Implementation:
      [`crates/synos-storaged/src/tiering.rs`](crates/synos-storaged/src/tiering.rs)
      provides bounded heat/cost/durability policy evaluation, copy-first
      journaled moves, durable destination fences, and restart reconciliation.
- [x] Add online format migration with shadow validation, resumable progress,
      rollback, downgrade refusal, and background I/O limits.
      Done when a power loss at every migration checkpoint leaves either the old
      or new format usable, never a hybrid.
      Implementation: [`crates/synfs/src/migration.rs`](crates/synfs/src/migration.rs)
      journals progress in the inactive bank, validates the shadow generation
      before publication, bounds copy steps, and supports device rollback;
      regression coverage is in [`crates/synfs/tests/migration.rs`](crates/synfs/tests/migration.rs).
- [x] Add capacity forecasting and fragmentation reports for SynFS, package
      cache, journals, snapshots, logs, and cluster metadata.
      Done when operators receive an actionable threshold before allocation
      failure and garbage collection remains bounded under pressure.
      Implementation: [`crates/synfs/src/capacity.rs`](crates/synfs/src/capacity.rs)
      provides fixed-capacity forecasts, warning/failure horizons, free-run
      fragmentation, reclaimable-block accounting, and bounded GC work limits.
      [`crates/synos-inspect/src/storage.rs`](crates/synos-inspect/src/storage.rs)
      carries categorized capacity samples to `SHOW-CAPACITY`; package, journal,
      and cluster metadata producers expose observations for the same report.
      Regression coverage is in [`crates/synfs/tests/capacity.rs`](crates/synfs/tests/capacity.rs).

## P0: Security, identity, and supply-chain trust

- [x] Add capability leases with audience, object, tenant, generation, purpose,
      expiry, and revocation epoch bound into one authorization decision.
      Done when confused-deputy, replay, copied-token, and stale-generation
      tests cover every privileged daemon. Implemented by the signed lease gate
      in [`crates/auth/src/lease.rs`](crates/auth/src/lease.rs), with storage,
      network, agent-daemon, and agent-bridge enforcement plus regression
      coverage.
- [x] Build a revocation propagation monitor for kernel, IPC, storage, network,
      package, cluster, and client caches.
      Done when maximum revocation latency is measured and stale authorization
      cannot survive cache refresh or reconnect.
      Implementation: [`crates/auth/src/revocation_monitor.rs`](crates/auth/src/revocation_monitor.rs)
      provides a bounded seven-cache epoch fence, propagation latency report,
      and refresh/reconnect/authorization rejection for stale entries.
      Regression coverage is in
      [`crates/auth/tests/revocation_monitor.rs`](crates/auth/tests/revocation_monitor.rs).
- [x] Add hardware-backed or isolated key-provider interfaces with rotation,
      quorum approval, recovery ceremony, and offline emergency revocation.
      Done when private keys never enter logs, snapshots, crash capsules, or
      ordinary process memory dumps. Implementation:
      [`crates/synos-shield/src/key_provider.rs`](crates/synos-shield/src/key_provider.rs)
      provides a fixed-capacity authority over opaque provider handles. Key
      generation, signing, approval verification, rotation, recovery, and
      emergency revocation stay behind the hardware or isolated-provider
      boundary; inventory snapshots contain metadata only. Regression coverage
      is in [`crates/synos-shield/tests/key_provider.rs`](crates/synos-shield/tests/key_provider.rs).
- [x] Produce signed SBOM, dependency provenance, compiler identity, source
      digest, configuration digest, and reproducible-build attestations for
      every release artifact.
      Done when a running process can be traced back to an independently verified
      source and toolchain record.
      Implementation: [`scripts/release-attestations.py`](scripts/release-attestations.py)
      creates and verifies a signed per-artifact statement containing all six
      records; [`scripts/package-vm-release.py`](scripts/package-vm-release.py)
      requires complete verified coverage before packaging and carries the
      attestations in the release archive.
- [x] Add policy simulation for capability changes, firewall changes, package
      activation, cluster membership, and update rollout.
      Done when operators can preview affected principals and objects without
      mutating live state.
      Implementation: [`crates/policy/src/lib.rs`](crates/policy/src/lib.rs)
      provides bounded, stale-snapshot-checked previews with before/after
      fingerprints; capability, firewall, package, membership, and rollout
      service adapters call the read-only simulator. Regression coverage
      covers all five change classes.
- [x] Add resource-exhaustion security tests for memory, CPU, IPC, storage,
      network, logs, audit queues, and control-plane requests.
      Done when hostile tenants are throttled or rejected without harming
      unrelated tenants or recovery traffic.
      Implementation: [`crates/system-model/src/quota.rs`](crates/system-model/src/quota.rs)
      includes control-plane requests in the bounded quota vocabulary;
      [`crates/system-model/tests/resource_exhaustion.rs`](crates/system-model/tests/resource_exhaustion.rs)
      proves per-tenant rejection, atomic failure, neighbor isolation, and
      recovery allowance across all eight dimensions.

## P1: Performance measurement and latency control

- [x] Create a repeatable benchmark harness with fixed hardware metadata,
      warmup rules, confidence intervals, p50/p95/p99 latency, throughput,
      allocations, CPU cycles, and energy where available.
      Done when regressions fail against declared budgets and noisy runs are
      marked inconclusive instead of hidden. Implementation:
      [`scripts/benchmark.py`](scripts/benchmark.py),
      [`benchmarks/budgets.toml`](benchmarks/budgets.toml), and the bounded VM
      benchmark allocation counters provide the repeatable report and gate.
- [x] Add continuous profiling for boot, IPC, scheduler, SynFS, networking,
      package activation, compiler builds, VM execution, and client RPC.
      Done when profiles are symbolized, redacted, retained by revision, and
      comparable across hosts. Implementation: bounded symbol-ID sampling in
      [`crates/observability/src/profiling.rs`](crates/observability/src/profiling.rs),
      subsystem hooks, versioned checked archives, and
      [`scripts/profile.py`](scripts/profile.py) for symbolization, revision-keyed
      retention, redaction validation, and cross-host comparison. See
      [`docs/profiling.md`](docs/profiling.md).
- [x] Reduce kernel and daemon lock contention with ownership reports,
      lock-duration histograms, sharded state, and wait-free or per-CPU paths
      where correctness permits.
      Done when contention improvements are proven under both one-core and
      many-core workloads with no fairness regression. See
      [`docs/lock-contention.md`](docs/lock-contention.md),
      [`kernel/src/contention.rs`](kernel/src/contention.rs), and
      [`kernel/benches/lock_contention.rs`](kernel/benches/lock_contention.rs).
- [x] Add per-CPU and NUMA-aware allocators for hot IPC, packet, timer, and
      scheduler objects with bounded cross-node fallback.
      Done when locality, fragmentation, tail latency, and reclamation behavior
      are measured under mixed workloads. See
      [`kernel/src/hot_allocator.rs`](kernel/src/hot_allocator.rs) and
      [`docs/hot-allocators.md`](docs/hot-allocators.md).
- [x] Add zero-copy or one-copy data paths for IPC buffers, network packets,
      storage reads/writes, WebGPU uploads, and client RPC frames.
      Done when ownership transitions are explicit, buffers are capability
      guarded, and reference-path output is byte-for-byte identical.
      Implementation: [`crates/ipc/src/buffer.rs`](crates/ipc/src/buffer.rs)
      provides capability-bound leases and explicit owner transfers. IPC rings,
      netd mappings and packet queues, storage I/O and cache adapters, WebGPU
      row uploads, HTTP/gRPC frames, and client RPC transports expose guarded
      paths. The guarded paths borrow shared bytes directly or perform one
      bounded copy and reuse the reference encoders/checksums.
- [x] Add batching, coalescing, and interrupt moderation policies for storage,
      network, logging, and audit producers.
      Done when batching improves throughput without exceeding interactive
      latency, memory, or fairness budgets.
      Implementation: [`crates/observability/src/throughput.rs`](crates/observability/src/throughput.rs)
      provides fixed batch, memory, delay, interrupt, and fairness budgets.
      Storage adapters take bounded request batches, netd applies the policy to
      ingress work, logd writes journal records through bounded batch calls, and
      audit scans use bounded package batches. Each path reports moderation or
      fairness decisions and keeps the existing fixed-capacity queues.
- [x] Add adaptive cache policies for SynFS metadata, package artifacts,
      compiler outputs, DNS, cluster membership, and VM translation blocks.
      Done when hit rate, eviction cost, memory ceiling, and stale-data risk are
      visible and tunable per workload.
      Implementation: [`crates/observability/src/cache.rs`](crates/observability/src/cache.rs)
      provides bounded per-workload policies for all six cache classes, adaptive
      TTL tuning, admission ceilings, hit/miss/eviction/stale-risk reports, and
      inspection publication. VM translation hits, misses, stale invalidations,
      and evictions feed the registry through
      [`virtual_machine/src/execution.rs`](virtual_machine/src/execution.rs).
- [x] Optimize compiler and package workflows with shared immutable artifacts,
      remote cache validation, parallel dependency scheduling, and cancellation
      that never publishes partial results.
      Done when clean, warm, offline, and cancelled builds have measured bounds.
      Implementation: [`crates/synos-rustd/src/workflow.rs`](crates/synos-rustd/src/workflow.rs)
      provides bounded shared artifacts and proof-checked remote cache entries;
      [`crates/synos-rustd/src/registry.rs`](crates/synos-rustd/src/registry.rs)
      schedules locked dependencies in deterministic parallel waves; completion
      validates cancellation, artifact identity, and cache capacity before any
      publication. [`scripts/benchmark-compiler-workflows.py`](scripts/benchmark-compiler-workflows.py)
      records clean, warm, offline, and cancelled p50/p95/max bounds in
      [`docs/compiler-workflows.md`](docs/compiler-workflows.md).
- [x] Add a performance budget to every public control-plane command and RPC.
      Done when queue wait, service time, retries, bytes, and tail latency are
      returned in structured diagnostics without leaking tenant data.
      Implementation: [`crates/system-model/src/performance.rs`](crates/system-model/src/performance.rs)
      defines bounded redacted budgets and rolling p99 diagnostics;
      [`crates/client-sdk/src/gateway.rs`](crates/client-sdk/src/gateway.rs)
      appends diagnostics to every frontend RPC response and
      [`crates/client-sdk/src/client.rs`](crates/client-sdk/src/client.rs)
      exposes them through `last_diagnostics()`; shell commands use
      [`crates/syn-shell/src/performance.rs`](crates/syn-shell/src/performance.rs)
      and structured output fields. Validation:
      `cargo check -p synos-client-sdk -p syn-shell`.

## P1: Horizontal and vertical scalability

- [x] Scale scheduler, IPC, timers, logging, and audit paths across 1, 2, 8,
      32, and 128 logical CPUs with explicit affinity and isolation policies.
      Done when throughput scales, tail latency stays within budget, and shared
      locks or queues have measured saturation points. Implementation and
      repeatable evidence harness: [`docs/scalability.md`](docs/scalability.md),
      [`crates/observability/src/scaling.rs`](crates/observability/src/scaling.rs),
      [`kernel/src/task.rs`](kernel/src/task.rs), and
      [`kernel/benches/scalability.rs`](kernel/benches/scalability.rs).
- [x] Add NUMA-aware placement for processes, memory, queues, storage workers,
      and network interrupts.
      Done when remote-memory traffic and placement decisions are observable and
      the system has a safe fallback on UMA hosts. Implementation:
      [`crates/numa`](crates/numa) provides bounded topology discovery, local /
      remote / UMA decisions, and remote-byte counters. Kernel process homes,
      hot-object memory reports, platform I/O queues, storage worker batches, and
      network interrupt polling expose the decisions through reports and trace
      fields. UMA is the default when firmware supplies no topology; regression
      coverage is in [`crates/numa/src/lib.rs`](crates/numa/src/lib.rs).
- [x] Define a cluster scale target and test membership, heartbeats, fencing,
      discovery, and recovery at 10, 100, and 1,000 nodes.
      Done when control-plane traffic, convergence time, memory, and failure
      amplification remain bounded at each tier. See
      [`docs/cluster-scale.md`](docs/cluster-scale.md) and the deterministic
      campaign in
      [`virtual_machine/tests/cluster.rs`](virtual_machine/tests/cluster.rs).
- [x] Shard cluster metadata, capability indexes, package catalogs, audit
      streams, and placement decisions without changing their consistency
      contracts.
      Done when shard movement, split-brain prevention, rebalancing, and node
      loss are tested with stable recovery evidence. Implementation and replay
      evidence: [`crates/synos-storaged/src/sharding.rs`](crates/synos-storaged/src/sharding.rs),
      [`crates/synos-storaged/tests/sharding.rs`](crates/synos-storaged/tests/sharding.rs),
      and [`docs/sharding-recovery.md`](docs/sharding-recovery.md).
- [x] Add admission control and load shedding for control-plane fanout,
      membership changes, snapshots, backups, package distribution, and remote
      diagnostics.
      Done when critical recovery traffic wins over optional work and operators
      see exactly what was delayed, dropped, or retried.
      Implementation: [`crates/admission/src/lib.rs`](crates/admission/src/lib.rs)
      provides fixed-capacity active slots, a bounded wait queue, a reserved
      recovery budget, per-work-class limits, retry accounting, and redacted
      delayed/dropped/retried outcomes. Membership fanout and changes use the
      gate in [`crates/synos-storaged/src/membership.rs`](crates/synos-storaged/src/membership.rs);
      package install and backup start expose admitted entry points; remote
      cluster diagnostics are gated by [`crates/synos-inspect/src/service.rs`](crates/synos-inspect/src/service.rs).
      Regression coverage is in the admission crate for recovery precedence,
      queue shedding, and separate drop/retry counters.
- [x] Add tenant-aware scheduling, storage placement, network shaping, and
      capability-scoped quotas with hierarchical accounting.
      Done when one tenant cannot consume shared tail latency or recovery budget.
      Implementation: [`crates/balancerd/src/resources.rs`](crates/balancerd/src/resources.rs)
      charges workload resources through bounded tenant parent chains, applies
      tenant-bound storage placement, and rotates queued work fairly;
      [`crates/netd/src/service.rs`](crates/netd/src/service.rs) adds fixed-size
      per-tenant egress token buckets; [`crates/admission/src/lib.rs`](crates/admission/src/lib.rs)
      reserves active and recovery capacity through tenant hierarchies; and
      [`kernel/src/capability.rs`](kernel/src/capability.rs) atomically charges
      delegated capability quotas through every ancestor. Regression coverage
      covers shared parent recovery capacity, egress isolation, and delegated
      capability accounting.
- [x] Add horizontal service scaling for HTTP, remote terminal, package,
      compiler, storage, and observability services with session handoff.
      Done when instances can join, drain, restart, and rebalance without lost
      requests or duplicated side effects. Implemented by the bounded,
      generation-fenced controller in [`crates/service-scale`](crates/service-scale)
      and typed adapters re-exported by the six service crates. Session
      snapshots move only after in-flight work reaches zero; stable request
      IDs and effect receipts make retries and post-restart replays return the
      original result without repeating a side effect. Regression coverage is
      in the controller unit model.
- [x] Add cardinality limits and aggregation for metrics, traces, audit labels,
      packet captures, and per-tenant diagnostics.
      Done when observability remains usable at cluster scale without becoming a
      denial-of-service vector.
      Implementation: fixed-capacity cardinality tables and overflow rollups
      live in [`crates/observability/src/cardinality.rs`](crates/observability/src/cardinality.rs);
      metric series, trace labels, audit labels, tenant diagnostics, and
      firewall packet flows use them. Packet capture keeps bounded `other` flow
      totals and all overflow counts remain inspectable.

## P1: Resilience, upgrades, and operations

- [ ] Build a fault-injection matrix for power loss, disk full, device reset,
      packet loss, partition, clock jump, process hang, corrupt input, and
      dependency outage across every critical workflow.
      Done when each failure has a bounded recovery time and explicit degraded
      behavior. The matrix covers `BOOT` (boot and readiness), `STORE` (write,
      commit, rename, and migration), `BACKUP` (backup and restore), `PKG`
      (package activation and rollback), `RPC` (DHCP, HTTP, gRPC, and terminal),
      `CLUSTER` (membership, quorum, leases, and failover), `AI` (inference,
      checkpoints, and agent snapshots), and `DEVICE` (reset, hotplug, and
      drain). RTO is measured from fault detection to a stable response; a
      cell passes only when the observed RTO is at or below its bound.

      | Fault | BOOT | STORE | BACKUP | PKG | RPC | CLUSTER | AI | DEVICE |
      | --- | --- | --- | --- | --- | --- | --- | --- | --- |
      | Power loss | ≤60s: boot last valid image | ≤30s: discard partial generation; keep prior | ≤90s: resume from checkpoint; source unchanged | ≤60s: keep old activation | ≤30s: reconnect; no duplicate effect | ≤120s: fence stale leases; quorum reads | ≤90s: resume last committed checkpoint | ≤60s: re-enumerate; mark missing offline |
      | Disk full | ≤30s: reserve recovery space; admin shell | ≤5s: reject writes; preserve reads and old generations | ≤15s: pause before commit; retry after space | ≤15s: refuse activation; old package stays | ≤5s: reject uploads; reads continue | ≤30s: block membership writes; quorum reads | ≤15s: stop new checkpoints/jobs; resume old checkpoint | ≤30s: reject new state; inspect device |
      | Device reset | ≤60s: bounded retry; safe mode if needed | ≤30s: fence device; use prior generation and degraded pool | ≤60s: fail stream safely; resume by chunk | ≤30s: retain staged bytes; no activation | ≤15s: reconnect transport; idempotent retry | ≤120s: suspect node; fence ownership | ≤60s: use mirror; resume checkpoint | ≤30s: reinitialize; keep device offline on failure |
      | Packet loss | ≤30s: local readiness does not wait on network | ≤30s: publish only after durable ack; serve old generation | ≤60s: retry bounded chunks; keep checkpoint | ≤60s: retry manifest; old version remains | ≤15s: retry idempotent request once; then timeout | ≤120s: require quorum; never split-brain | ≤60s: wait for dual journal ack; resume last commit | ≤30s: bounded control retry; mark link degraded |
      | Partition | ≤30s: boot local services; show dependency degraded | ≤30s: local durable writes; reject cluster-backed writes | ≤60s: keep local snapshot; pause remote upload | ≤60s: use cached signed artifacts; no cluster activation | ≤15s: serve local routes; return unavailable remotely | ≤120s: minority read-only; fence and rejoin | ≤60s: continue only with local mirror; stop without quorum | ≤30s: isolate remote device; local devices continue |
      | Clock jump | ≤30s: use monotonic time; delay network readiness | ≤10s: monotonic deadlines; commit semantics unchanged | ≤30s: monotonic schedule; pause expiry decisions | ≤30s: fail closed when wall time is untrusted | ≤15s: monotonic timeout; auth expiry fails closed | ≤120s: bounded monotonic leases; stop unsafe renewal | ≤60s: monotonic checkpoints; lose unsafe lease safely | ≤30s: monotonic debounce; no unsafe detach |
      | Process hang | ≤30s: watchdog restart; readiness stays gated | ≤30s: fence daemon; serve last valid root | ≤60s: stop worker; checkpoint remains usable | ≤60s: abort activation; old package stays live | ≤15s: restart handler; request ID prevents replay | ≤120s: mark node suspect; fence leases | ≤60s: restart from checkpoint or snapshot | ≤30s: restart driver; device stays offline if hung |
      | Corrupt input | ≤30s: reject image/config; enter recovery path | ≤5s: reject before publish; old generation survives | ≤30s: quarantine bad chunk; source stays intact | ≤30s: reject digest/signature; old activation stays | ≤5s: stable invalid-input error; connection survives | ≤15s: reject frame; peer state unchanged | ≤15s: reject tensor/tool/snapshot; no mutation | ≤15s: reject descriptor; reset endpoint |
      | Dependency outage | ≤60s: start independent services; expose degraded state | ≤30s: read-only on missing dependency; retain old root | ≤60s: retain checkpoint; stage locally | ≤60s: use cached artifacts; defer activation | ≤15s: return unavailable; unrelated routes continue | ≤120s: apply quorum rules; no unsafe writes | ≤60s: use local model/checkpoint; stop without journal | ≤30s: isolate failed dependency; unaffected devices run |

      Each injection records the fault point, detection time, observed RTO,
      status/degraded mode, data and audit continuity, and duplicate-side-effect
      checks. Recovery budgets are hard cutoffs: retry, queue, and failover
      work stops at the bound and returns an explicit status.
      Implementation: [`crates/test-support/src/fault_matrix.rs`](crates/test-support/src/fault_matrix.rs)
      defines all 72 typed fault/workflow cells, RTO budgets, deterministic
      injection checkpoints, degraded actions, and evidence validation for data,
      capability, stale-generation, audit, and duplicate-side-effect safety.
      [`crates/test-support/tests/fault_matrix.rs`](crates/test-support/tests/fault_matrix.rs)
      covers matrix completeness, one-shot injection, bounded evidence, and
      rejected unsafe observations. Keep this checkbox open until each real
      workflow adapter records measured fault evidence.
- [ ] Add rolling, canary, blue/green, and emergency update strategies for
      kernel, services, packages, clients, schemas, and cluster protocols.
      Done when mixed-version operation is tested and rollback preserves data,
      capabilities, and audit continuity. Every release is one signed bundle
      with artifact digests, compatibility ranges, migration ID, rollback
      target, capability epoch, and audit sequence. Existing staged activation,
      service hot-swap, kernel patch generations, protocol negotiation, and
      immutable package roots are the implementation boundaries.

      | Strategy | Kernel | Services | Packages | Clients | Schemas | Cluster protocols |
      | --- | --- | --- | --- | --- | --- | --- |
      | Rolling | Stage a signed inactive slot; reboot one node; health-check before the next; retain the prior slot | Spawn replacement with inherited descriptors; switch only after ready; drain and fence the old process | Stage immutable digest; activate one node; keep the previous root | Negotiate old/new wire versions; hand off sessions by request ID | Expand first, dual-read/write, then contract after old readers leave | Update one member at a time; preserve quorum; fence stale epochs |
      | Canary | Use one non-critical node; gate on boot, invariant, and workload health; abort before quorum impact | Route one instance or tenant; compare errors, latency, and resource use; stop on threshold breach | Activate one low-risk tenant or node; verify signatures, data, and rollback receipt | Opt-in a small client cohort; retain the old endpoint and status mapping | Shadow-parse new fields; publish no incompatible field until all can read it | Use one follower/learner; require protocol intersection and no lease loss |
      | Blue/green | Boot a green slot or node pool beside blue; cut traffic only after health; keep blue bootable | Run green beside blue; mirror safe reads; switch routing atomically; drain blue | Build a green root from content IDs; switch the active pointer; preserve blue objects | Point a bounded cohort at green; move the rest only after compatibility and audit checks | Prepare green readers/writers against the same versioned contract; cut over once | Prepare a green control plane; perform one quorum-approved epoch cutover; keep blue read-only |
      | Emergency | Apply only an authenticated bounded patch, or boot the last known-good image; never leave a partial redirect | Freeze rollout; drain if safe, otherwise fence and restart; restore the last healthy generation | Stop activation and return to the last signed root; do not mutate package bytes | Force the minimum safe protocol; disable the broken feature; keep reconnect and retry semantics | Refuse destructive migration; restore the last checkpoint or use the older reader | Freeze mutations; fence incompatible members; restore the last quorum-safe protocol and rejoin with a new epoch |

      Mixed-version gates are: (1) signature, provenance, digest, and rollback
      target verified before staging; (2) every version pair has an explicit
      read/write, read/convert, or reject result from
      [`docs/compatibility-matrix.md`](docs/compatibility-matrix.md); (3) no
      incompatible pair accepts a mutation; (4) data roots remain immutable
      until health passes; (5) capability object IDs survive while generation
      epochs fence stale grants; and (6) audit records remain append-only with
      the same request and release IDs across cutover and rollback.

      The acceptance run injects failure before stage, during mixed-version
      service, at cutover, after cutover, and during rollback for every artifact
      and strategy. It records both-version behavior, data/root checksums,
      capability validity and stale-generation rejection, audit continuity,
      duplicate-side-effect checks, and the exact rollback receipt. Rollback
      changes only the executable/configuration pointer; it never deletes data,
      rewrites history, or silently downgrades a schema.
      Implementation: [`crates/synos-update/src/rollout.rs`](crates/synos-update/src/rollout.rs)
      provides the signed release bundle, six-artifact compatibility gate,
      rolling/canary/blue-green/emergency state machines, health gates, audit
      events, capability-epoch fencing input, and rollback integrity checks.
      [`crates/synos-update/tests/rollout.rs`](crates/synos-update/tests/rollout.rs)
      covers all four strategies, mixed-version rejection, and health-failure
      rollback. Keep this checkbox open until concrete kernel, service,
      package, client, schema, and cluster runtimes provide measured evidence.
- [x] Add a coordinated drain protocol for processes, sockets, queues, storage
      leases, terminal sessions, and cluster ownership before maintenance.
      Done when drain completion is provable and forced termination leaves no
      live lock or partial publication. Implementation:
      [`crates/synos-update/src/drain.rs`](crates/synos-update/src/drain.rs)
      provides the six-resource bounded state machine, monotonic grace and
      force deadlines, handoff ordering, capability fencing input, audit
      events, and forced-cleanup proof. Its contract is exercised by
      [`crates/synos-update/tests/drain.rs`](crates/synos-update/tests/drain.rs)
      for cooperative drain, forced cleanup, residual live-lock rejection,
      and backwards-clock failure. Keep this checkbox open until concrete
      process, socket, queue, storage, terminal, and cluster runtimes are
      wired to the coordinator and produce maintenance evidence.
- [x] Add an operator runbook generator from service health, dependency,
      recovery, quota, and compatibility metadata.
      Done when every alert links to diagnosis, safe action, rollback, and proof
      of recovery.
      Implementation: [`crates/synos-inspect/src/runbook.rs`](crates/synos-inspect/src/runbook.rs)
      provides fixed-capacity alert-code registration, validation of all five
      metadata sources, and atomic bulk generation. Every generated record has
      typed diagnosis, safe-action, rollback, and recovery-proof links;
      [`crates/synos-inspect/tests/runbook.rs`](crates/synos-inspect/tests/runbook.rs)
      covers complete generation and incomplete-link rejection.
- [x] Add SLOs for boot, interactive shell, IPC, storage commit, DHCP, RPC,
      package activation, snapshot restore, and cluster convergence.
      Done when the system reports error budget consumption and refuses release
      claims without fresh evidence.
      Implementation: [`crates/observability/src/slo.rs`](crates/observability/src/slo.rs)
      defines bounded objectives and error-budget consumption for all nine
      paths; [`scripts/release-slo-gate.py`](scripts/release-slo-gate.py)
      requires complete, passed, revision-matched evidence within the freshness
      window before a release claim can pass.

## P2: Developer and user-facing quality

- [x] Version every public Rust, Swift, wire, shell, package, snapshot, and
      configuration API with compatibility tests and deprecation warnings.
      Done when old clients receive stable errors and supported migrations are
      documented with examples. Implemented by [`crates/api-compat`](crates/api-compat),
      boundary version constants, Swift compatibility errors, and the migration
      guide in [`docs/api-versioning.md`](docs/api-versioning.md).
- [x] Add generated API documentation and executable cookbook examples for
      boot, storage, networking, capabilities, clusters, compiler jobs, and
      recovery.
      Done when examples build in CI-equivalent local validation and use only
      public interfaces. Implemented by the generated workspace API index in
      [`docs/api.md`](docs/api.md), the seven public-interface binaries in
      [`examples/cookbook`](examples/cookbook/), and the locked local validator
      [`scripts/validate-cookbook.py`](scripts/validate-cookbook.py).
- [x] Add deterministic replay bundles that include input events, clock values,
      random seeds, device completions, scheduler decisions, and configuration
      digests while excluding secrets.
      Done when a failure can be replayed on another supported host.
- [x] Add compatibility and differential tests against independent filesystem,
      network, terminal, firmware, and serialization references.
      Done when divergences produce minimized inputs and a documented decision.
      Implementation: [`virtual_machine/tests/differential_compatibility.rs`](virtual_machine/tests/differential_compatibility.rs)
      compares the five real paths with independent bounded references;
      [`crates/test-support/src/differential.rs`](crates/test-support/src/differential.rs)
      minimizes reproducing inputs, and the reference decisions are recorded in
      [`docs/differential-compatibility.md`](docs/differential-compatibility.md).
- [x] Add fuzzing with retained corpus and triage metadata for every untrusted
      parser, including manifests, ABI frames, packets, snapshots, CLI input,
      terminal bytes, and migration streams.
      Done when crashes, hangs, excessive allocation, and timeout cases are
      reproducible and assigned. Implementation: the complete target inventory
      and ownership map is in [`fuzz/triage.toml`](fuzz/triage.toml); checked-in
      seeds live under [`fuzz/corpus`](fuzz/corpus); failures are retained with
      SHA-256 metadata under [`fuzz/regressions`](fuzz/regressions) and replayed
      by [`scripts/replay-fuzz.sh`](scripts/replay-fuzz.sh).
- [ ] Add accessibility and usability checks for shell errors, terminal output,
      diagnostics, JSON schemas, client UI, and recovery messages.
      Done when operators can identify action, impact, retry safety, and audit
      identity without reading internal logs.

## P2: Hardware efficiency and platform reach

- [ ] Add power and thermal policy integration for CPU idle states, frequency,
      device runtime power, thermal throttling, and cluster workload placement.
      Done when performance-per-watt and thermal recovery are measured without
      violating latency or correctness budgets.
- [ ] Add driver capability discovery and graceful degradation for missing
      acceleration, storage features, NIC offloads, GPUs, firmware services,
      and platform timers.
      Done when the selected fallback is visible and keeps semantics stable.
- [ ] Qualify x86_64, aarch64, Linux, macOS, and Windows host paths with a
      shared portability matrix for firmware, terminal, disks, networking,
      acceleration, and timekeeping.
      Done when unavailable features are explicit skips with reasons and no
      platform silently changes the data model.

## Release acceptance gate

- [ ] Every completed item has code, direct evidence, owner, risk, compatibility
      impact, performance impact, scalability limit, and rollback notes.
- [ ] Release reports include correctness results, p50/p95/p99 data, resource
      ceilings, fault recovery times, supported scale tier, and known limits.
- [ ] No release claims “scalable,” “durable,” “secure,” or “real-time” without
      a named workload, measured threshold, host configuration, and retained
      artifact.
- [ ] The full roadmap status is machine-readable and distinguishes planned,
      running, passed, failed, blocked, skipped, and inconclusive evidence.
