# SynOS testing contract

This document defines the minimum test evidence for every feature in
`TODO.md`. The machine-readable feature map is
[`test-inventory.toml`](test-inventory.toml).

## Contract

Every feature change adds or updates its inventory entry in the same change.
The entry names one test in each applicable tier:

- `unit`: direct behavior and state-transition tests.
- `integration`: the public API, protocol, or cross-crate path.
- `qemu`: booted-guest or device integration. Use `not_applicable` only when
  the feature cannot cross a guest boundary.
- `fault`: invalid input, authorization failure, resource limit, timeout,
  corruption, restart, or partial-failure behavior.
- `fuzz`: untrusted bytes, parser inputs, or state-machine sequences. Use
  `not_applicable` only when the feature has no untrusted input.
- `performance`: a bounded benchmark or regression guard. It must record the
  machine, revision, input size, and elapsed time; it must not use a flaky
  wall-clock assertion.

A feature that crosses a process, device, boot, persistence, or cluster
boundary needs the relevant integration and end-to-end evidence. A green
happy-path test alone never completes a feature.

## Feature checklist

Copy this checklist into every new feature section and keep the matching
inventory entry beside it:

```text
- [ ] parser/API and success path
- [ ] invalid input and status mapping
- [ ] authorization and isolation
- [ ] limits, overflow, cancellation, and timeout
- [ ] persistence, restart, and recovery where applicable
- [ ] observability and audit evidence
- [ ] compatibility and version behavior
- [ ] inventory entry, stable test IDs, and evidence path
```

Every public type, operation, status code, wire message, and error variant
gets one direct test and one boundary test. Every bug fix adds a regression
test before the fix is marked complete. Hardware behavior that cannot run in
CI is recorded as a manual hardware-smoke test with its required evidence;
an unexecuted or skipped test is never counted as passing.

## Hosts and tools

The deterministic host tier runs on Linux, macOS, and Windows hosts with the
stable Rust toolchain from `rust-toolchain.toml`. The bare-metal targets are
`x86_64-unknown-none`, `x86_64-unknown-uefi`, `aarch64-unknown-none`, and
`riscv64gc-unknown-none-elf`.

Required for the host tier:

- Rust stable with `rust-src` and `llvm-tools`.
- Cargo and the repository checkout.
- `rg` for the repository scripts and evidence checks.

Optional tools enable higher tiers:

- QEMU `qemu-system-x86_64` for guest tests.
- `dosfstools` and `mtools` for portable image builds.
- Docker for the reproducible QEMU environment.
- `cargo-fuzz` and LLVM tools for fuzzing and sanitizer jobs.
- A serial capture tool and supported hardware for bare-metal smoke tests.

The QEMU cluster launcher currently requires Linux because its shared
`ivshmem` setup is Linux-only. Hardware qualification needs the platform and
evidence described in `platforms/README.md`.

## Test tiers and commands

| Tier | Command or entry point | Default | Evidence |
| --- | --- | --- | --- |
| host-unit | `cargo test` | required | test output and package metadata |
| workspace | `cargo test --workspace --all-targets` | required for CI | test output and package metadata |
| vm | `cargo test --manifest-path virtual_machine/Cargo.toml` | required for VM changes | test output and VM metadata |
| qemu | `SYNOS_RUN_QEMU_TESTS=1 cargo test --manifest-path virtual_machine/Cargo.toml --test test_environments -- --ignored` | opt-in | serial log, QEMU command, exit reason |
| cluster | `scripts/qemu-cluster.sh` and `scripts/qualify-platform.sh` | opt-in | node serial logs, command logs, failover log |
| fuzz | `cargo fuzz run <target>` from `fuzz/` | opt-in | corpus, crash artifact, revision |
| performance | benchmark command named by the inventory entry | opt-in | JSON result and machine metadata |

The `workspace` command is the intended full host command. Until the VM is
promoted into the root workspace, run the `vm` command separately and keep its
result in the same evidence bundle.

## Shared harness

[`synos-test-support`](../crates/test-support) is the test-only support crate.
It is a workspace member but is deliberately absent from `default-members`,
so production crates do not inherit test helpers. Add it only as a
`dev-dependency` or use it from an integration test.

`FixtureSet::new(seed)` supplies the common deterministic fixtures. Use
`TestScope` when a test also owns external resources. The in-memory doubles
cover block I/O, network transport, IPC, key/value storage, attestation, and
accelerator queues. `FaultPlan` injects one-shot failures at named boundaries.
The cleanup guard runs actions in reverse registration order and runs again on
panic through `Drop`.

Checked-in golden inputs live in
`crates/test-support/golden/`: boot image bytes, protocol frames, filesystem
blocks, snapshots, audit records, package signatures, and terminal output.
Update a golden file only with a reason in the change description and a
corresponding compatibility decision.

## Environment variables

| Variable | Default | Use |
| --- | --- | --- |
| `SYNOS_RUN_QEMU_TESTS` | unset | Enable ignored QEMU tests when set to `1`. |
| `SYNOS_QEMU_IMAGE` | `build/bios/synos-bios.img` | Guest disk image for VM integration tests. |
| `SYNOS_QEMU_BIN` | `qemu-system-x86_64` | QEMU executable. |
| `SYNOS_QEMU_ACCEL` | `tcg` for tests | `tcg` or `kvm` acceleration. |
| `SYNOS_QEMU_EXTRA` | empty | Extra QEMU arguments. |
| `SYNOS_DISK_IMAGE` | `build/bios/synos-bios.img` | Image used by cluster and Docker flows. |
| `SYNOS_CLUSTER_NODES` | `2` in the cluster script | Cluster guest count, from 2 through 8. |
| `SYNOS_GUEST_MEMORY` | `1G` in the cluster script | Memory per guest. |
| `SYNOS_CXL_MEMORY` | `256M` | Per-node CXL backing file size. |
| `SYNOS_SHARED_MEMORY` | `256M` | Shared `ivshmem` backing file size. |
| `SYNOS_CLUSTER_BUS` | `230.0.0.1:1234` | QEMU multicast cluster bus. |

Tests must not depend on an unset variable having a hidden meaning. The test
result records the variables that were actually used.

## Names and evidence

Inventory IDs are permanent names. Use lowercase dotted names:
`<tier>.<feature-id>.<behavior>`. The Rust test function may be longer, but
the inventory ID and the evidence directory stay unchanged if the
implementation moves.

Each run writes under:

```text
build/test-evidence/<run-id>/<tier>/<test-id>/
```

The directory contains `result.json`, `metadata.json`, `stdout.log`, and
`stderr.log`. QEMU, cluster, and hardware tests also save their serial,
command, inventory, and failure-injection logs. `result.json` uses only these
states: `pass`, `fail`, `skipped`, `blocked`, or `not_implemented`.

`skipped` requires a reason and the missing prerequisite. `blocked` requires
an owner and a follow-up issue. `not_implemented` is inventory debt. None of
these states counts as `pass` in a release gate.

`metadata.json` records the Git revision, UTC start/end times, host OS and
architecture, Rust version, test command, environment-variable names and
values that affect the test, and tool versions. Secrets and capability tokens
must be redacted before saving evidence.

## Hardware and regression policy

If a behavior needs real hardware, add a smoke test with an explicit profile,
setup, expected serial markers, and evidence filenames. A passing emulation
test may support the feature, but it does not replace hardware evidence.

When a test finds a bug, keep the smallest deterministic reproduction in the
normal host tier, then link its stable ID from the inventory. Randomized and
fuzz failures record their seed or corpus input so the regression can replay
without the fuzzer.
