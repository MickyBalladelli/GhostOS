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

- [ ] Create a repeatable benchmark harness with fixed hardware metadata,
      warmup rules, confidence intervals, p50/p95/p99 latency, throughput,
      allocations, CPU cycles, and energy where available.
      Done when regressions fail against declared budgets and noisy runs are
      marked inconclusive instead of hidden.
- [ ] Add continuous profiling for boot, IPC, scheduler, SynFS, networking,
      package activation, compiler builds, VM execution, and client RPC.
      Done when profiles are symbolized, redacted, retained by revision, and
      comparable across hosts.
- [ ] Reduce kernel and daemon lock contention with ownership reports,
      lock-duration histograms, sharded state, and wait-free or per-CPU paths
      where correctness permits.
      Done when contention improvements are proven under both one-core and
      many-core workloads with no fairness regression.
- [ ] Add per-CPU and NUMA-aware allocators for hot IPC, packet, timer, and
      scheduler objects with bounded cross-node fallback.
      Done when locality, fragmentation, tail latency, and reclamation behavior
      are measured under mixed workloads.
- [ ] Add zero-copy or one-copy data paths for IPC buffers, network packets,
      storage reads/writes, WebGPU uploads, and client RPC frames.
      Done when ownership transitions are explicit, buffers are capability
      guarded, and reference-path output is byte-for-byte identical.
- [ ] Add batching, coalescing, and interrupt moderation policies for storage,
      network, logging, and audit producers.
      Done when batching improves throughput without exceeding interactive
      latency, memory, or fairness budgets.
- [ ] Add adaptive cache policies for SynFS metadata, package artifacts,
      compiler outputs, DNS, cluster membership, and VM translation blocks.
      Done when hit rate, eviction cost, memory ceiling, and stale-data risk are
      visible and tunable per workload.
- [ ] Optimize compiler and package workflows with shared immutable artifacts,
      remote cache validation, parallel dependency scheduling, and cancellation
      that never publishes partial results.
      Done when clean, warm, offline, and cancelled builds have measured bounds.
- [ ] Add a performance budget to every public control-plane command and RPC.
      Done when queue wait, service time, retries, bytes, and tail latency are
      returned in structured diagnostics without leaking tenant data.

## P1: Horizontal and vertical scalability

- [ ] Scale scheduler, IPC, timers, logging, and audit paths across 1, 2, 8,
      32, and 128 logical CPUs with explicit affinity and isolation policies.
      Done when throughput scales, tail latency stays within budget, and shared
      locks or queues have measured saturation points.
- [ ] Add NUMA-aware placement for processes, memory, queues, storage workers,
      and network interrupts.
      Done when remote-memory traffic and placement decisions are observable and
      the system has a safe fallback on UMA hosts.
- [ ] Define a cluster scale target and test membership, heartbeats, fencing,
      discovery, and recovery at 10, 100, and 1,000 nodes.
      Done when control-plane traffic, convergence time, memory, and failure
      amplification remain bounded at each tier.
- [ ] Shard cluster metadata, capability indexes, package catalogs, audit
      streams, and placement decisions without changing their consistency
      contracts.
      Done when shard movement, split-brain prevention, rebalancing, and node
      loss are tested with stable recovery evidence.
- [ ] Add admission control and load shedding for control-plane fanout,
      membership changes, snapshots, backups, package distribution, and remote
      diagnostics.
      Done when critical recovery traffic wins over optional work and operators
      see exactly what was delayed, dropped, or retried.
- [ ] Add tenant-aware scheduling, storage placement, network shaping, and
      capability-scoped quotas with hierarchical accounting.
      Done when one tenant cannot consume shared tail latency or recovery budget.
- [ ] Add horizontal service scaling for HTTP, remote terminal, package,
      compiler, storage, and observability services with session handoff.
      Done when instances can join, drain, restart, and rebalance without lost
      requests or duplicated side effects.
- [ ] Add cardinality limits and aggregation for metrics, traces, audit labels,
      packet captures, and per-tenant diagnostics.
      Done when observability remains usable at cluster scale without becoming a
      denial-of-service vector.

## P1: Resilience, upgrades, and operations

- [ ] Build a fault-injection matrix for power loss, disk full, device reset,
      packet loss, partition, clock jump, process hang, corrupt input, and
      dependency outage across every critical workflow.
      Done when each failure has a bounded recovery time and explicit degraded
      behavior.
- [ ] Add rolling, canary, blue/green, and emergency update strategies for
      kernel, services, packages, clients, schemas, and cluster protocols.
      Done when mixed-version operation is tested and rollback preserves data,
      capabilities, and audit continuity.
- [ ] Add a coordinated drain protocol for processes, sockets, queues, storage
      leases, terminal sessions, and cluster ownership before maintenance.
      Done when drain completion is provable and forced termination leaves no
      live lock or partial publication.
- [ ] Add an operator runbook generator from service health, dependency,
      recovery, quota, and compatibility metadata.
      Done when every alert links to diagnosis, safe action, rollback, and proof
      of recovery.
- [ ] Add SLOs for boot, interactive shell, IPC, storage commit, DHCP, RPC,
      package activation, snapshot restore, and cluster convergence.
      Done when the system reports error budget consumption and refuses release
      claims without fresh evidence.

## P2: Developer and user-facing quality

- [ ] Version every public Rust, Swift, wire, shell, package, snapshot, and
      configuration API with compatibility tests and deprecation warnings.
      Done when old clients receive stable errors and supported migrations are
      documented with examples.
- [ ] Add generated API documentation and executable cookbook examples for
      boot, storage, networking, capabilities, clusters, compiler jobs, and
      recovery.
      Done when examples build in CI-equivalent local validation and use only
      public interfaces.
- [ ] Add deterministic replay bundles that include input events, clock values,
      random seeds, device completions, scheduler decisions, and configuration
      digests while excluding secrets.
      Done when a failure can be replayed on another supported host.
- [ ] Add compatibility and differential tests against independent filesystem,
      network, terminal, firmware, and serialization references.
      Done when divergences produce minimized inputs and a documented decision.
- [ ] Add fuzzing with retained corpus and triage metadata for every untrusted
      parser, including manifests, ABI frames, packets, snapshots, CLI input,
      terminal bytes, and migration streams.
      Done when crashes, hangs, excessive allocation, and timeout cases are
      reproducible and assigned.
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
