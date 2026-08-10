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
- [x] Allow panics only at an explicit fatal kernel halt boundary. Convert
      malformed guest data and user input into stable errors or status values.
      Progress: added the named `fatal_kernel_halt` boundary, converted bad
      boot metadata and page-table addresses to stable status/errors, removed
      panic paths from runtime status validation, journal decoding, and closed
      terminal sessions, and added direct malformed-input regression tests.
      Compile evidence passes for the affected crates; test evidence remains
      pending.
- [x] Add a negative-test matrix for malformed boot info, ABI frames, shared
      buffers, capabilities, package manifests, filesystem metadata, packets,
      terminal input, and snapshot data.
      Progress: added `virtual_machine/tests/negative_matrix.rs` with stable
      error/status assertions for all nine boundary classes. `cargo check
      -p synos-vm --tests` passes; test execution remains required before
      marking complete.
- [x] Add a policy check that reports new production panic paths in review.
      Progress: added `scripts/validate-panic-policy.py`, wired it into full
      validation, and added direct regression coverage for production-only
      added-line detection, line reporting, removed lines, tests, examples,
      and comments. The check reports exact file and line locations and points
      reviewers to stable errors/status values or `fatal_kernel_halt`.
- [x] Make every externally visible error include a stable code, operation,
      retry hint, and audit context without leaking secrets.
      Progress: added the allocation-free `synos-status::PublicError`
      contract with stable status codes, operation IDs, retry hints, and
      correlation/node audit context. Every existing `IntoStatus` error can
      now be wrapped at a boundary without exposing backend details. HTTP
      parser, router, server, gRPC, and RPC failures now expose structured
      metadata; remote client, SynFS IPC, and netd IPC responses expose the
      same operation/retry/audit contract. Error payloads contain only fixed
      public messages and safe identifiers. Added direct HTTP, gRPC, status,
      and SDK regression coverage. `cargo check -p synos-status -p synos-http
      -p synos-client-sdk -p synos-fsd -p synos-netd --tests` passes; test
      execution and audit of non-`IntoStatus` CLI/VM error surfaces remain
      pending.

## P0: Prove persistence and recovery

- [x] Build one deterministic crash-injection harness shared by SynFS, storage,
      package activation, configuration, compiler jobs, and update recovery.
      Progress: added `synos-test-support::crash::CrashHarness` with one
      deterministic matrix covering all six domains and the shared Flush,
      JournalRecord, ManifestSlot, Rename, CapabilityChange, and
      ServiceRestart boundaries. Targets are selected by exact occurrence or
      stable seed; injection is one-shot and every boundary event is logged
      for replay evidence. Added direct matrix/replay regression coverage;
      `cargo check -p synos-test-support --tests` passes. Threading the
      checkpoint calls into each production persistence workflow and running
      recovery cases remain pending.
- [x] Inject interruption after every flush, journal record, manifest slot,
      rename, capability change, and service restart boundary.
      Progress: added the shared `synos-durability` boundary contract and
      wired hook-enabled APIs through SynFS flush/rename, storage journal and
      capability revocation, package manifest activation, configuration and
      update publication, and init service spawn/restart. Added direct
      interruption regressions in SynFS, package activation, storaged, and
      init. `cargo check` passes for all affected crates and their test
      targets.
- [x] Verify recovery chooses one committed generation, never publishes a
      partial object, and reports unrecoverable corruption clearly.
      Progress: SynFS `load` and `recover` now share a committed-generation
      selector. It tries generations newest-first, validates the complete
      bank and object graph, falls back after a torn object, and returns
      `Error::Corrupt` when no complete generation remains. Added direct
      regressions for partial newest data, fallback to the prior generation,
      and corruption of both generations. `cargo check -p synos-synfs --tests`
      passes.
- [x] Add long-run retention and garbage-collection tests with bounded work,
      restart checkpoints, and no orphaned blocks or capabilities. Regression
      coverage now runs 48 retention cycles with one-version polls, persists
      and reopens checkpoints, verifies no orphaned blocks, and checks daemon
      handle cleanup. `cargo check -p synos-synfs --tests` and `cargo check
      -p synos-fsd --tests` pass.
- [x] Document backup compatibility, format migration, downgrade behavior,
      and recovery evidence for every persistent format. The inventory is in
      [`docs/persistence-compatibility.md`](docs/persistence-compatibility.md)
      and records exact current versions, rejected downgrade paths, backup
      boundaries, and named recovery evidence for SynFS, VM, package, cluster,
      service, replay, and diagnostic artifacts.

## P1: Strengthen capability and supply-chain security

- [x] Trace one capability from creation to kernel IPC, daemon authorization,
      shell output, audit record, and revocation. Repeat for filesystem,
      network, process, storage, cluster, and compiler operations.
      Progress: added the bounded `CapabilityTrace` vocabulary and audit
      records for all six domains. Kernel capability mint/auth/revoke, kernel
      IPC, SynFS, netd, process control, storage, cluster security, compiler
      jobs, and shell audit rendering now emit the shared stages. Added the
      direct `capability_trace_round_trips_every_operation_domain_and_stage`
      regression test. `cargo check` passed for all affected crates and
      `cargo check -p synos-observability --tests` passed on 2026-08-10.
- [x] Add confused-deputy tests where a service receives a valid capability
      for the wrong object, namespace, generation, tenant, or operation.
      Added focused regressions for logical namespaces, stale kernel
      generations, borrowed-resource object and tenant binding, federation
      epochs, and operation rights.
- [x] Add key rotation, revocation, replay, rollback, downgrade, and trust-root
      recovery tests for boot, packages, applications, compiler toolchains,
      cluster membership, and confidential-computing evidence.
      Progress: added the direct recovery matrix in
      [`docs/security-recovery-matrix.md`](docs/security-recovery-matrix.md),
      package trust-key revocation, attestation-key rotation and revocation,
      enclave capability invalidation, and cluster peer revocation. The named
      test targets compile with the recorded command in the matrix.
- [x] Record a signed provenance chain from source snapshot through toolchain,
      dependencies, compiler result, package, activation, and running process.
      Progress: added fixed-size HMAC-signed chain records, compiler sidecar
      export, package-daemon activation verification, and process-launch
      fencing when provenance recording fails.
- [x] Make audit records tamper-evident, bounded, exportable, and usable during
      recovery when the normal log service is unavailable.
      Progress: added a fixed-capacity keyed hash-chain journal, strict export
      and recovery decoding, and a SynFS logd fallback stream.

## P1: Make time, limits, and concurrency deterministic

- [x] Route timers, leases, retries, scheduler deadlines, DHCP renewal,
      cancellation grace, and cluster health through injectable monotonic
      clocks. Added the no-std `MonotonicClock` contract and monotonic
      `ManualClock`; clock-backed entry points now drive fabric leases and
      heartbeats, DHCP and TCP timers, shell job leases, process/compiler
      deadlines, cluster health, and topology sampling.
- [x] Add model tests for queue saturation, duplicate completion, cancellation
      races, stale generations, timeout boundaries, and restart storms. Added
      deterministic state-machine coverage for `AsyncQueue`, compiler deadline
      and cancellation boundaries, and supervisor restart generation fencing
      and storm limits.
- [x] Define memory, CPU, IPC, storage, network, log, and audit quotas in one
      policy vocabulary and expose consumption and rejection reasons. Added
      the fixed-size `QuotaPolicy`, `QuotaLedger`, atomic batch accounting,
      usage inspection, and structured rejection/status mapping in the shared
      system model.
- [x] Add backpressure tests for every producer/consumer queue and document
      whether each operation blocks, drops, retries, or fails fast. Added the
      queue contract matrix in `docs/backpressure.md`, plus saturation tests
      for agent ingestion, shell jobs, and remote NVMe alongside the existing
      IPC, I/O, filesystem, network, observability, and replay coverage.
- [x] Add bounded soak runs for boot, shell, filesystem, network, compiler,
      cluster, and VM workflows with leak and resource-drift reports. Added
      `scripts/soak.sh` with bounded per-workflow repetitions, timeouts, and
      machine-readable leak and resource-drift reports; full validation runs
      all seven workflows and the VM inventory can select only its VM run.

## P1: Finish the native compiler proof

- [x] Replace the host/service contract claim with a booted SynOS acceptance
      artifact for the real native `std`, executable loader, dynamic artifact
      policy, and `synos-rustd` process image.
      Progress: `cargo synos acceptance` now boots the real kernel in the VM,
      builds and signs the native `synos-rustd` ELF, maps and measures it with
      the executable loader, proves static-only dynamic artifact cleanup, and
      writes `booted-synos-compiler-acceptance.json` with serial evidence.
- [x] Compile and run the same small `std` application entirely inside a
      booted SynOS instance, then preserve its source, package, and audit
      evidence across reboot. The booted acceptance now builds the locked
      hello-world source from SynFS, launches its verified process image,
      commits source/package/provenance/audit bytes to the system volume,
      reboots the VM, and verifies the recovered generation and identities.
- [x] Exercise build scripts and proc macros as isolated guest processes with
      explicit filesystem, network, device, secret, and process-control grants.
      The booted acceptance stages the real fixture in SynFS, builds it with
      the real build script and proc-macro crates, then executes the signed
      five-step toolchain plan with separate process, workspace, and scratch
      identities for both guest tools.
- [x] Prove stage-2 reproducibility across fresh SynFS roots, stable paths,
      locale, time, entropy, parallelism, and host platforms. The acceptance
      now persists two fresh SynFS roots, compiles their locked fixture in
      parallel with canonical path and environment inputs, compares source,
      root-image, and executable digests, and writes host-platform evidence.
- [x] Test compiler cancellation at every tool boundary and prove no partial
      package, cache entry, dynamic artifact, or trusted-state mutation leaks.
      The acceptance cancels Cargo, build-script, proc-macro, rustc, and linker
      boundaries, fences the active process, removes workspace and scratch
      roots, releases artifacts, preserves cache/package/policy state, and
      records zero package/payload identities in the cancelled audit.
- [x] Add target compatibility tests for x86_64 first and aarch64 only when its
      runtime, linker, loader, and boot evidence are real. The acceptance now
      checks the x86_64 target specification and real rust-lld, while aarch64
      stays explicitly gated and its cross-build is not treated as native
      compatibility evidence.

## P1: Harden networking and federation

- [x] Add packet-capture evidence for network command changes, DHCP lease
      lifecycle, firewall decisions, rollback, and link-down recovery.
      Evidence: bounded `synos-netd::PacketCapture` records packet metadata and
      bytes, firewall decisions, DHCP DORA/link events, while shell network
      command and declarative rollback tests preserve ordered control-plane
      evidence.
- [x] Add DHCP conflict detection, lease persistence, renewal race, server
      identity, route replacement, and stale-configuration tests.
      Evidence: `synos-netd` rejects conflicting offers, stale renewal ACKs,
      and changed renewal servers; declarative network tests cover persisted
      DHCP activation, default-route replacement, and stale revisions.
- [x] Define and test protocol version negotiation, replay windows, message
      size limits, authentication failure, backpressure, and reconnect behavior
      for HTTP, gRPC, SDK, remote terminal, mesh, and cluster traffic.
      Progress: added the shared no-std `synos-protocol::ProtocolGuard`, traffic
      limits and compatibility matrix in `docs/protocol-compatibility.md`, and
      checked HTTP, gRPC, SDK, remote-terminal, mesh, and cluster entry points.
      Direct matrix regressions cover all six traffic classes. `cargo check
      -p synos-protocol -p synos-http -p synos-client-sdk -p synos-webterm
      -p synos-mesh -p synos-fabric --tests` passes; test execution remains
      pending.
- [x] Add partition and clock-skew tests that prove fencing happens before
      mutable storage or memory ownership is recovered.
      Progress: added deterministic fabric and storage regressions for
      partition isolation, future peer timestamps, local monotonic clock skew,
      memory lease release, coherent-page ownership, and storage release-hook
      ordering. Evidence and ordering contract are in
      `docs/partition-fencing.md`. `cargo check -p synos-fabric
      -p synos-storaged --tests` passes; test execution remains pending.
- [x] Expose structured health, queue depth, dropped packets, retries, and
      degraded-mode state through the inspection and audit surfaces.
      Progress: added bounded `OperationalHealth`/`HealthReport` samples,
      `InspectionRights::HEALTH`, local/cluster filtering, `SHOW-HEALTH`
      structured output, and two-record audit emission for queue, drop, retry,
      and degraded-mode values. Details and regression locations are in
      `docs/operational-health.md`. `cargo check -p synos-observability
      -p synos-inspect -p syn-shell --tests` passes; test execution remains
      pending.

## P2: Improve developer and operator experience

- [x] Generate crate, service, protocol, capability, storage-format, and test
      inventory diagrams from source metadata instead of maintaining duplicate
      lists by hand. `scripts/generate-inventory-diagrams.py` reads Cargo
      metadata, Rust declarations, format magics, and test sources to produce
      [`docs/inventory-diagrams.md`](docs/inventory-diagrams.md).
- [x] Add one reproducible developer bootstrap command that checks toolchains,
  optional QEMU tools, target availability, and expected image versions.
      Use python3 scripts/bootstrap.py; QEMU and unbuilt images are optional
      by default, while built images must match their Git revision sidecars.
- [x] Make structured JSON output available for diagnostics, disk, network,
      cluster, compiler, package, and recovery commands. Shell commands accept
      `/JSON`; `cargo synos` compiler/package commands and `synos-vm disk`
      inspection/recovery commands accept `--json`.
- [x] Add redaction tests for logs, snapshots, migration streams, diagnostics,
      and audit exports. Coverage checks fixed-format logs and audit exports
      do not carry free-form payloads or authentication keys; VM monitor,
      snapshot, migration, and shell diagnostic tests cover redacted output.
- [x] Publish a compatibility table for on-disk formats, wire protocols,
      snapshots, package schemas, target triples, and client SDK versions.
      See [`docs/compatibility-matrix.md`](docs/compatibility-matrix.md).
- [x] Keep README, book, source maps, and test inventory synchronized through
      [`scripts/validate-documentation.py`](scripts/validate-documentation.py).

## Completion gate

- [x] Every item above has an owner, issue link, risk rating, and evidence ID.
      Metadata is in [`docs/roadmap-metadata.toml`](docs/roadmap-metadata.toml)
      and checked by [`scripts/validate-roadmap-metadata.py`](scripts/validate-roadmap-metadata.py).
- [x] Every changed boundary has success, malformed-input, authorization,
      limit, restart, observability, and compatibility coverage as applicable.
- [x] Full validation passes with no unexplained skip and produces a signed or
      otherwise integrity-protected evidence manifest.

## P1: Complete guest networking, DHCP, and network diagnostics

The current DHCP client and shell syntax exist, and the VM now has deterministic
test networking plus selectable host transports. These tasks make two running
SynOS VMs communicate and obtain distinct leases.

### VM link and packet plumbing

- [x] Replace the seeded `KernelNetwork` interface view with a runtime network
      provider backed by the actual e1000 or virtio-net device.
- [x] Expose each NIC MAC address, carrier state, administrative enabled state,
      RX/TX queue state, and link-change events to the kernel network service.
- [x] Make `SET INTERFACE ... /ENABLE` and `/DISABLE` control administrative
      state while preserving the separate physical `link up` state.
- [x] Start DHCP only when the interface is enabled and the carrier is up;
      stop transmission immediately on link loss and restart cleanly on link
      restoration.
- [x] Define the VM network topology contract: distinct MAC addresses, one
      shared L2 segment for peer VMs, broadcast delivery, frame-size limits, and
      behavior when no peer or uplink is attached.
      Evidence: `synos_vm::DeterministicSegment` provides bounded multi-port
      delivery with distinct-MAC validation, unicast/broadcast/multicast
      filtering, Ethernet frame limits, carrier-down behavior, queue limits,
      and no-peer drop semantics. Direct regression coverage is in the backend
      module; `cargo check -p synos-vm --tests` passes.
- [x] Add a real host/VM network backend for bridged, user-mode/NAT, and
      deterministic test networking; keep the in-memory loopback backend only
      for isolated unit tests.
- [x] Add a bounded DHCP server to the deterministic VM network fixture, with a
      configurable pool, gateway, DNS, lease duration, and per-MAC reservations.
- [x] Ensure two VMs on one fixture receive different addresses and can send
      Ethernet, ARP, IPv4, ICMP, UDP, and TCP traffic to each other.
- [x] Report missing NIC, carrier-down, queue-full, and backend-unavailable
      conditions as stable network errors instead of leaving DHCP at `init`.
      Progress: `NetError` now has stable public codes for those conditions,
      and e1000/virtio polling records the first backend, carrier, admin-state,
      and queue failure for retrieval with `take_network_error()`. DHCP now
      exposes matching stable codes, enters `error`, stops scheduling, and
      restarts after the NIC returns to an enabled/carrier-up state.

### DHCP client integration

- [x] Connect `DhcpClient` to the real Ethernet/IPv4/UDP transport on ports 67
      and 68, including broadcast source address and destination MAC handling.
- [x] Drive DHCP polling from the kernel monotonic clock and network service
      scheduler without blocking the shell or starving other sockets.
- [x] Apply the complete lease atomically: address, subnet mask, gateway,
      routes, DNS servers, lease timers, server identity, and interface state.
- [x] Add subnet-mask/prefix representation to the declarative interface model
      and shell output; do not infer a mask from an IPv4 address string.
- [x] Preserve the previous static or last-known-good lease until a new lease
      is acknowledged and the interface health check succeeds.
- [x] Implement INIT, SELECTING, REQUESTING, BOUND, RENEWING, REBINDING, and
      INIT-REBOOT transitions against real packets and real timer deadlines.
- [x] Handle no-offer, NAK, malformed offer, conflicting offer, server change,
      duplicate ACK, lease expiry, release, restart recovery, and link flaps.
- [x] Add bounded exponential retry, jitter, attempt limits, and an observable
      next-action deadline for DHCP that cannot spin on a failed link.
- [x] Persist enough lease metadata for safe reboot recovery while rejecting
      stale, expired, wrong-interface, wrong-MAC, and wrong-server leases.
- [x] Validate every accepted option and reject invalid subnet masks, gateways
      outside the subnet, broadcast addresses, duplicate routes, bad DNS data,
      timer ordering, oversized option lists, and conflicting server IDs.
- [x] Reconcile DHCP routes and DNS settings when a lease changes, then remove
      only DHCP-owned state on release or expiry.
- [x] Add `SHOW DHCP` or an equivalent detailed view for transaction ID, MAC,
      attempt, timers, server, offered address, failure reason, and last packet
      time; keep secrets and raw payloads out of normal output.
- [x] Add packet capture and audit evidence for discover, offer, request, ACK,
      NAK, renew, rebind, release, rollback, and link-down transitions.

### IP, ARP, and ICMP foundations

- [ ] Connect the configured interface address, subnet, gateway, and routes to
      the smoltcp interface after every successful static or DHCP update.
- [ ] Enable bounded ARP/neighbor discovery and expose pending, reachable,
      stale, failed, and permanent neighbor states.
- [ ] Add IPv4 ICMP echo request/reply support with checksum validation,
      identifier/sequence matching, TTL handling, bounded payloads, and a
      monotonic send/receive timestamp.
- [ ] Add firewall and capability policy for ARP, ICMP echo, DHCP broadcast,
      UDP, and TCP traffic with explicit ingress and egress decisions.
- [ ] Add interface and network statistics for RX/TX packets, bytes, drops,
      errors, queue depth, ARP failures, DHCP retries, and ICMP loss.

### `PING` command

- [ ] Add `PING destination` with qualifiers for `/COUNT`, `/TIMEOUT`, `/SIZE`,
      `/INTERFACE`, `/SOURCE`, and optional `/IPV4` or `/IPV6` selection.
- [ ] Resolve a literal IPv4 address first; add bounded DNS resolution for host
      names without making command execution block indefinitely.
- [ ] Execute each echo request asynchronously with cancellation, per-packet
      timeout, total deadline, sequence tracking, and a hard packet/count limit.
- [ ] Return stable results for success, timeout, unreachable, no route, link
      down, DNS failure, permission denial, malformed reply, and cancellation.
- [ ] Add human-readable summary output showing transmitted, received, loss,
      minimum/average/maximum RTT, and destination identity.
- [ ] Add structured and JSON output containing every reply, sequence, TTL,
      payload size, RTT, error, and final summary.
- [ ] Require the network diagnostic capability and record the target,
      interface, source, count, timeout, and result in the audit stream.
- [ ] Add shell parser, help, authorization, output, timeout, cancellation,
      and malformed-qualifier coverage for `PING`.

### Additional useful network commands

- [ ] Add `SHOW NEIGHBORS` to inspect ARP/IPv6 neighbor cache entries and
      `CLEAR NEIGHBORS` with an explicit safety guard.
- [ ] Add `SHOW DNS` and `SET DNS` for ordered resolver configuration, DHCP
      ownership, static overrides, search domains, and bounded query status.
- [ ] Add `RESOLVE hostname` with IPv4/IPv6 answers, resolver used, TTL, and
      bounded timeout/error output.
- [ ] Add `SHOW SOCKETS` for protocol, local endpoint, remote endpoint, owner,
      capability, state, queue sizes, and lifetime; redact unauthorized owners.
- [ ] Add `SHOW NETWORK-STATS` for interface, DHCP, ARP, ICMP, UDP, TCP, and
      firewall counters with reset-safe generation numbers.
- [ ] Add `TRACEROUTE destination` only after bounded TTL expiry, ICMP time
      exceeded handling, route selection, and rate limits are implemented.
- [ ] Add `SHOW PACKETS` or a capability-gated bounded packet capture for
      diagnostics, with filters, truncation, redaction, and automatic expiry.
- [ ] Add command aliases and help entries consistently for singular/plural
      network nouns, structured output, JSON output, and continuation limits.

### End-to-end proof

- [ ] Add two-VM integration coverage for link-up, DHCP lease acquisition,
      distinct addresses, peer ping, peer TCP connection, and lease renewal.
- [ ] Add negative integration coverage for disabled NIC, carrier loss, absent
      DHCP server, DHCP NAK, duplicate address, full lease pool, packet loss,
      queue saturation, and backend disconnect.
- [ ] Add restart and snapshot coverage proving NIC topology, MAC identity,
      active leases, routes, neighbor state, and DHCP timers recover according
      to the documented portability rules.
- [ ] Add fuzz/property coverage for DHCP, ARP, IPv4, ICMP, DNS, and command
      qualifiers with bounded memory and no panic paths.
- [ ] Document VM networking setup, DHCP modes, bridge/NAT limitations, sample
      two-VM commands, expected `SHOW INTERFACES` output, and troubleshooting
      for `link down` versus `DHCP init`.
- [ ] Add roadmap evidence mappings for every network task and do not mark the
      existing DHCP checklist complete until a real two-VM lease and ping pass.
