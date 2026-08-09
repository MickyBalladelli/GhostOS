# SynOS Improvement TODO

This is the next roadmap after `TODO.md`, `TODO-compiler.md`, and
`TODO-network.md`. The old roadmaps have no open checkbox items, so this file
tracks hardening and proof work. Do not mark an item complete from a code
claim alone. Each item needs code, a direct regression test, and recorded
evidence.

## Review baseline

- [x] Run Bonsai over the repository and inspect the generated project map.
- [x] Inspect every existing TODO file, including the compiler and network
      roadmaps.
- [x] Inspect the workspace layout, test inventory, coverage rules, kernel
      boundaries, runtime services, storage, networking, and VM integration
      surfaces.
- [x] Keep this review baseline current when the workspace or roadmap schema
      changes.

## P0: Make roadmap status truthful

- [x] Merge the duplicate root sections 37, 38, and 39 while
      preserving their links and inventory mappings.
- [x] Replace the empty `60.0 True VT100 emulator` heading with a real scoped
      feature, or move it to a dedicated terminal roadmap.
- [x] Add a roadmap validator that rejects duplicate IDs, empty feature
      bodies, stale links, and TODO headings missing from the inventory.
- [x] Separate `planned`, `running`, `passed`, `failed`, and `blocked` in the
      machine-readable inventory. A named test must not count as evidence.
- [x] Require a result artifact, source revision, command, host, and timestamp
      before a roadmap checkbox can be marked `[x]`.
- [x] Generate a status report showing feature, code owner, last evidence,
      skipped prerequisites, and stale evidence age.
- [x] Add local validation coverage for the new roadmap files and make roadmap changes fail
      when their evidence mappings are missing.

## P0: Remove unsafe boundary behavior

- [x] Audit production `panic!`, `unwrap`, and `expect` calls at boot, parser,
      IPC, device, storage, package, and network boundaries.
      Progress: implementation pass completed across the audited boot,
      parser, IPC, device, storage, package, network, shell, snapshot, and
      replay paths. Malformed wire data, guest descriptors, filesystem records,
      and invalid queue state now return stable errors or status values.
      Added direct malformed-input regression tests for ELF loader fields and
      firewall policy decoding. Changed crates pass compile validation.
- [ ] Allow panics only at an explicit fatal kernel halt boundary. Convert
      malformed guest data and user input into stable errors or status values.
- [ ] Add a negative-test matrix for malformed boot info, ABI frames, shared
      buffers, capabilities, package manifests, filesystem metadata, packets,
      terminal input, and snapshot data.
- [ ] Add a policy check that reports new production panic paths in review.
- [ ] Make every externally visible error include a stable code, operation,
      retry hint, and audit context without leaking secrets.

## P0: Prove persistence and recovery

- [ ] Build one deterministic crash-injection harness shared by SynFS, storage,
      package activation, configuration, compiler jobs, and update recovery.
- [ ] Inject interruption after every flush, journal record, manifest slot,
      rename, capability change, and service restart boundary.
- [ ] Verify recovery chooses one committed generation, never publishes a
      partial object, and reports unrecoverable corruption clearly.
- [ ] Add long-run retention and garbage-collection tests with bounded work,
      restart checkpoints, and no orphaned blocks or capabilities.
- [ ] Document backup compatibility, format migration, downgrade behavior,
      and recovery evidence for every persistent format.

## P1: Strengthen capability and supply-chain security

- [ ] Trace one capability from creation to kernel IPC, daemon authorization,
      shell output, audit record, and revocation. Repeat for filesystem,
      network, process, storage, cluster, and compiler operations.
- [ ] Add confused-deputy tests where a service receives a valid capability
      for the wrong object, namespace, generation, tenant, or operation.
- [ ] Add key rotation, revocation, replay, rollback, downgrade, and trust-root
      recovery tests for boot, packages, applications, compiler toolchains,
      cluster membership, and confidential-computing evidence.
- [ ] Record a signed provenance chain from source snapshot through toolchain,
      dependencies, compiler result, package, activation, and running process.
- [ ] Make audit records tamper-evident, bounded, exportable, and usable during
      recovery when the normal log service is unavailable.

## P1: Make time, limits, and concurrency deterministic

- [ ] Route timers, leases, retries, scheduler deadlines, DHCP renewal,
      cancellation grace, and cluster health through injectable monotonic
      clocks.
- [ ] Add model tests for queue saturation, duplicate completion, cancellation
      races, stale generations, timeout boundaries, and restart storms.
- [ ] Define memory, CPU, IPC, storage, network, log, and audit quotas in one
      policy vocabulary and expose consumption and rejection reasons.
- [ ] Add backpressure tests for every producer/consumer queue and document
      whether each operation blocks, drops, retries, or fails fast.
- [ ] Add bounded soak runs for boot, shell, filesystem, network, compiler,
      cluster, and VM workflows with leak and resource-drift reports.

## P1: Finish the native compiler proof

- [ ] Replace the host/service contract claim with a booted SynOS acceptance
      artifact for the real native `std`, executable loader, dynamic artifact
      policy, and `synos-rustd` process image.
- [ ] Compile and run the same small `std` application entirely inside a
      booted SynOS instance, then preserve its source, package, and audit
      evidence across reboot.
- [ ] Exercise build scripts and proc macros as isolated guest processes with
      explicit filesystem, network, device, secret, and process-control grants.
- [ ] Prove stage-2 reproducibility across fresh SynFS roots, stable paths,
      locale, time, entropy, parallelism, and host platforms.
- [ ] Test compiler cancellation at every tool boundary and prove no partial
      package, cache entry, dynamic artifact, or trusted-state mutation leaks.
- [ ] Add target compatibility tests for x86_64 first and aarch64 only when its
      runtime, linker, loader, and boot evidence are real.

## P1: Harden networking and federation

- [ ] Add packet-capture evidence for network command changes, DHCP lease
      lifecycle, firewall decisions, rollback, and link-down recovery.
- [ ] Add DHCP conflict detection, lease persistence, renewal race, server
      identity, route replacement, and stale-configuration tests.
- [ ] Define and test protocol version negotiation, replay windows, message
      size limits, authentication failure, backpressure, and reconnect behavior
      for HTTP, gRPC, SDK, remote terminal, mesh, and cluster traffic.
- [ ] Add partition and clock-skew tests that prove fencing happens before
      mutable storage or memory ownership is recovered.
- [ ] Expose structured health, queue depth, dropped packets, retries, and
      degraded-mode state through the inspection and audit surfaces.

## P2: Improve developer and operator experience

- [ ] Generate crate, service, protocol, capability, storage-format, and test
      inventory diagrams from source metadata instead of maintaining duplicate
      lists by hand.
- [ ] Add one reproducible developer bootstrap command that checks toolchains,
      optional QEMU tools, target availability, and expected image versions.
- [ ] Make structured JSON output available for diagnostics, disk, network,
      cluster, compiler, package, and recovery commands.
- [ ] Add redaction tests for logs, snapshots, migration streams, diagnostics,
      and audit exports.
- [ ] Publish a compatibility table for on-disk formats, wire protocols,
      snapshots, package schemas, target triples, and client SDK versions.
- [ ] Keep README, book, source maps, and test inventory synchronized through
      documentation checks.

## Completion gate

- [ ] Every item above has an owner, issue link, risk rating, and evidence ID.
- [ ] Every changed boundary has success, malformed-input, authorization,
      limit, restart, observability, and compatibility coverage as applicable.
- [ ] Full validation passes with no unexplained skip and produces a signed or
      otherwise integrity-protected evidence manifest.
