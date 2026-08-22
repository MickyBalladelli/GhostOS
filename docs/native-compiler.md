# Native Compiler

`cargo-ghostos compile-all` is the native-target build gate for the TODO
roadmap. It builds every production Ring 0 and Ring 3 library crate for a
GhostOS target with `core`, `alloc`, position-independent code, and the bundled
Rust linker.

```sh
cargo run -p cargo-ghostos -- ghostos compile-all --target x86_64 --release
cargo run -p cargo-ghostos -- ghostos compile-all --target aarch64 --release
```

Excluded packages are host-side tools or test machines:

- `cargo-ghostos`
- `ghostos-compiler`
- `ghostos-test-support`
- `ghostos-uefi`
- `ghostos-vm`

The compiler driver runs on the build host. The output is native GhostOS code.
Self-hosting Rust compilation inside a running GhostOS instance needs a GhostOS
`std` port, process loader, dynamic library support, compiler package, and a
filesystem-backed compiler service. Those are separate runtime work, not a
property that can be claimed from a host Cargo command.

The current service boundary is represented by `ghostos-rustd`. Its requests are
fixed-size and include the source root, manifest, target, profile, lock policy,
network policy, and resource limits. The default policy denies network access.
During user-space boot, `CompilerServiceBoot` verifies the package gate,
registers the service with init, and starts it under the compiler capability
profile. `CompilerServiceHealthCheck` makes a system update fail and roll back
when the service is not running or the active root is not bound to that
package. The host driver accepts matching `--locked` and `--offline` flags and
can write a signed package atomically after a successful build. Its
`compile_ghostfs` path validates the source root and manifest through GhostFS,
copies the bounded project tree into an isolated Cargo workspace, and compiles
that snapshot with the same target and lock policy.

## Frozen design

The primary self-hosting target is `x86_64-unknown-ghostos`. The
`aarch64-unknown-ghostos` target follows after x86_64 self-hosting works. The
initial Rust surface includes `core`, `alloc`, `std`, Cargo, build scripts, proc
macros, tests, and rustdoc. The compiler stack is upstream `rustc` plus LLVM.

GhostFS layout is fixed: `/system/toolchains` contains stage-0, stage-1, and
stage-2 toolchains; `/system/registries` contains signed registries;
`/system/sources` contains source snapshots; `/system/builds` contains build
state; `/system/tmp` contains scratch data; and `/system/bundles` contains
output bundles. These paths are exported as constants by `ghostos-rustd`.

Compiler IPC is protocol version 1 with fixed-size frames. It supports
`Submit`, `Start`, `Poll`, `Cancel`, `Release`, and `ReadLog`. Responses use
existing `ghostos-status` values. Log records contain a job id, sequence number,
level, event, and bounded message. Queued jobs cancel immediately; running
jobs get a cooperative stop and are fenced after a five-second grace period;
terminal jobs reject cancellation; partial output is never published.

## Native process contract

`ghostos-app` now owns the Ring 3 side of native process loading. Its ELF64
loader accepts little-endian x86_64 and Aarch64 images, validates PT_LOAD
bounds, entry points, TLS, relative relocations, and package payload identity.
W^X is mandatory: writable/executable segments and executable stacks are
rejected. The `ImageMapper` backend maps segments, zero-fills BSS, applies
relocations before final protection, creates a non-executable heap and guarded
stack, allocates TLS, and installs the entry context.

`ProcessSupervisor` provides bounded `spawn`, `exec`, `wait`, cancellation,
deadline, CPU, memory, and fencing state. Cancellation is cooperative first;
the backend is fenced after the configured grace period. Application launch
already passes the signed package identity and capability-approved request to
the runtime. The runtime must resolve the immutable payload, verify its
measurement, use `load_image`, and report the resulting process exit status.
Executable pages can be measured through `measure_executable_pages` before
the process is made runnable.

`ghostos-rustd` also has a signed `ToolchainManifest`. It requires Cargo, rustc,
and the linker, optionally requires build-script and proc-macro runners, then
executes the ordered plan through a process-runtime hook. Every tool receives
the bounded build request, and failed tools are fenced before the job fails.
The native loader and GhostOS `std` PAL still need to provide that hook for true
in-guest execution.

Signed compiler output can now be launched through `ghostos-app`. The supervisor
validates the package instantiation receipt and entry point, authorizes the
manifest's requested capabilities against policy, and passes only the approved
capabilities plus the package payload identity to the process runtime. The
native executable-image loader still must map that payload before the process
can run on a booted GhostOS instance.

The self-host path is represented by `SelfHostSession`. It submits and validates
the runtime build first, refuses unlocked or networked requests, then submits
the compiler build only after the runtime artifact is complete. Both results
must carry non-zero package and payload identities for the session to finish.

Reproducibility is checked with:

```sh
cargo run -p cargo-ghostos -- ghostos reproduce --target x86_64 --release
```

The command creates one fresh source workspace, builds it twice from clean
target state with `--locked --offline`, disables incremental state, remaps
workspace and target paths, fixes locale/time/entropy inputs, and compares the
final `.rlib`, `.rmeta`, `.a`, `.o`, and `.so` artifacts by content ID. For
`.rlib` files, it hashes code archive members while ignoring Cargo metadata
whose generated declaration order is not stable.

## Acceptance gate

Run the compiler acceptance matrix with:

```sh
cargo run -p cargo-ghostos -- ghostos acceptance --target x86_64 --release
```

The gate checks signed boot and storage contracts, offline locked requests,
isolated concurrent jobs, cancellation and crash recovery, stage-2 ordering,
signed/corrupt/unsigned package handling, hello-world compilation and host
execution, build scripts, proc macros, application bundles, production Ring 3
builds, reproducibility, fresh persisted GhostFS roots, and target compatibility.
The x86_64 check validates the admitted target specification and rust-lld; the
full x86_64 gate also supplies the real runtime, loader, and boot evidence.
The aarch64 target remains gated: its target JSON may be used for cross-build
work, but cross-compilation alone cannot admit native compatibility.
The fresh-root check compiles two identical GhostFS snapshots in parallel, then
writes `stage2-reproducibility-evidence.json` with canonical inputs, source,
root-image, executable digests, and the executing host platform. Use
`--skip-build` for the fast in-memory contract pass and `--json` for local
evidence records. Pass `--clean-root` to choose a new artifact root explicitly.
Use `--boot-only` to run just the booted native compiler artifact without the
longer host build matrix. Run the gate on each additional native host to extend
the recorded host-platform matrix.

The cancellation contract runs the five-step toolchain plan with cancellation
injected at Cargo, build-script, proc-macro, rustc, and linker boundaries. Each
case fences the active tool, removes its GhostFS workspace and scratch roots,
releases dynamic artifacts, leaves both cache counts unchanged, keeps the
verified package and compiler policy unchanged, and records only a cancelled
audit with zero package/payload identities.

The acceptance gate also produces a booted evidence artifact under its chosen
root. It boots the real kernel in the VM, builds the native `ghostos-rustd`
process image with the runtime PAL linked in, signs and authorizes that image,
maps and measures it through `ghostos-app`, verifies static-only dynamic artifact
cleanup, starts the service through init, and preserves the kernel serial log.
It also stages the locked hello-world project in GhostFS, builds its release
image, launches it through the application supervisor, commits the source,
signed package, provenance, build audit, and output to the installed system
volume, reboots from that disk, and verifies the recovered bytes and content
IDs. The same booted artifact stages `examples/compiler-acceptance` in GhostFS,
builds its build-script and proc-macro crates, and runs the signed toolchain
plan with separate guest process/workspace/scratch identities. Each tool
record carries explicit filesystem, network, device, secret, and
process-control grants. The artifact is the acceptance boundary for the
native `std` PAL and process image; it does not treat a host Cargo run as an
in-guest execution result.

The compiler service boundary now includes the operational Ring 3 path:
signed local-registry pins are resolved by content ID, source reads require a
capability-bound `SourceGrant`, and each job receives independent build and
scratch roots. `CompilerService::handle_ipc` streams state and structured log
records; `tick` handles deadline expiry and cooperative cancellation; a
`BuildWorkspaceRuntime` implementation performs the actual GhostFS cleanup.
Successful results enter the immutable cache only after source, lockfile,
toolchain, target, profile, and feature identities all match.

For the complete host-side step, use `cargo-ghostos package` with
`--manifest-path`, `--locked`, `--offline`, `--key`, and `--output`.

## Rust application bundles

`cargo-ghostos package --app-manifest` turns a linked ELF into a signed GhostOS
application. The profile requires `schema`, application `name`, `image`,
`kind`, `target`, and `entry_offset`, plus all three resource fields:

```toml
schema = 1

[application]
name = "demo"
image = "0x1"
kind = "service"
target = "x86_64-unknown-ghostos"
entry_offset = 0

[resources]
memory_bytes = 16777216
cpu_time_us = 1000000
heap_bytes = 65536
```

Dependencies use repeated `[[dependency]]` sections with a 64-digit package
content ID. Capability requests stay in repeated `[[capability]]` sections.
The compiler checks the ELF machine, signs a `SYNAPP01` envelope containing
the target, entry point, dependencies, resource limits, and manifest data,
then writes a content-addressed `.build-record` beside the bundle.

```sh
cargo ghostos package --bin demo --app-manifest App.toml \
  --target x86_64 --release --locked --offline \
  --key compiler.key --output demo.synapp \
  --debug-symbols demo.debug --stripped-output demo.stripped
```

The runtime publishes this output with
`PackageDaemon::install_application_bundle`. Launch checks the trusted
signature, target, entry point, dependency closure, and resource limits before
the normal ELF loader receives the image.

## User workflow

The host command mirrors the GhostOS shell vocabulary:

```sh
cargo ghostos check --manifest-path service/Cargo.toml --bin service --locked --offline
cargo ghostos build --manifest-path service/Cargo.toml --bin service --release --locked --offline
cargo ghostos run --manifest-path service/Cargo.toml --bin service
cargo ghostos test --manifest-path service/Cargo.toml --locked --offline
cargo ghostos doc --manifest-path service/Cargo.toml --offline
```

Add `--json` to `check`, `build`, `test`, `doc`, or `run` for Cargo JSON
diagnostics and a final machine-readable command record. Without it, Cargo's
normal diagnostics stay visible to a human.

Inside the shell, `ghostos-shell::rust::register_rust_commands` exposes the same
operations as `RUST CHECK`, `RUST BUILD`, `RUST RUN`, `RUST TEST`, and `RUST
DOC`, plus `SHOW-RUST-JOBS` and `TOOLCHAIN INSTALL|SELECT|UPDATE|ROLLBACK`.
The adapter takes a capability-bound `RustSource`; it does not grant the
shell direct access to compiler files or processes.

`CompilerService::snapshot` reports queued, running, completed, failed, and
cancelled jobs; requested memory and CPU budgets; active artifacts; cache
entries; cache hits; and cache misses. `CompilerService::jobs` provides the
bounded per-job records for detailed inspection.

Toolchain management requires an explicit capability root:

```sh
cargo ghostos toolchain install --bundle stage-2.synpkg --key compiler.key \
  --root /system/toolchains --stage 2 --target x86_64
cargo ghostos toolchain select --root /system/toolchains --stage 2
cargo ghostos toolchain rollback --root /system/toolchains
```

The manager verifies the signed bundle before install, keeps the previous
bundle for rollback, and refuses `/` as a toolchain root.

Small service recipe from inside GhostOS:

```text
CREATE-DIRECTORY /sources/hello
RUST CHECK /MANIFEST=/sources/hello/Cargo.toml /BIN=hello /LOCKED /OFFLINE
RUST BUILD /MANIFEST=/sources/hello/Cargo.toml /BIN=hello /RELEASE /LOCKED /OFFLINE
RUST RUN /MANIFEST=/sources/hello/Cargo.toml /BIN=hello
```

## Security and recovery

The compiler service runs under the dedicated `SYNORUST` identity. Its
minimum capability set covers source, toolchain, registry, build, cache,
output, IPC, and approved tool execution. Source, toolchain, registry, build,
cache, output, active-package, and trusted-key roots are separate. Write
authorization allows only build, cache, and output roots; active compiler and
trusted-key roots are fenced.

Build-script and proc-macro plans carry an explicit capability mask. Network,
device, secret, and process-control capabilities are denied unless both the
policy and the request grant them. Every job exposes a bounded
`BuildAuditRecord` containing source, dependency, toolchain, package, payload,
capability, identity, and final status fields.

If `ghostos-rustd` crashes, init fences the old process and applies its bounded
restart policy. `CompilerServiceBoot::recover_after_crash` marks queued and
running jobs failed, releases dynamic artifacts, leaves workspace cleanup
pending, and starts only the next process generation. The workspace runtime
then calls `cleanup_job` for each `cleanup_pending` job.

Toolchain installation and rollback write a transaction journal before any
rename. `ToolchainManager::recover` restores the last known active state after
an interrupted operation, so a failed update cannot publish a half-swapped
toolchain.

## Toolchain package

The host bootstrap can package the complete Rust toolchain needed by the
native service:

```sh
cargo run -p cargo-ghostos -- ghostos toolchain package \
  --stage 0 --target x86_64 --key compiler.key --output stage-0.synpkg
cargo run -p cargo-ghostos -- ghostos toolchain verify \
  --bundle stage-0.synpkg --key compiler.key
```

Stage 1 and stage 2 roots use the same format. Their root contains `bin/`
with Cargo, rustc, rustdoc, and rust-lld, plus `sysroot/`,
`target-libraries/`, and `rust-src/`; pass that root with `--root` and the
matching stage number. The archive uses stable sorted paths, content IDs for
every file, and no symbolic links. `ghostos-pkg` verifies the signed outer
bundle before GhostFS installation, while `ghostos-rustd` authorizes each
component and revokes build-script and proc-macro images when the build ends.
