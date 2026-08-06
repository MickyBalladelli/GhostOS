# Native Compiler

`cargo-synos compile-all` is the native-target build gate for the TODO
roadmap. It builds every production Ring 0 and Ring 3 library crate for a
SynOS target with `core`, `alloc`, position-independent code, and the bundled
Rust linker.

```sh
cargo run -p cargo-synos -- synos compile-all --target x86_64 --release
cargo run -p cargo-synos -- synos compile-all --target aarch64 --release
```

Excluded packages are host-side tools or test machines:

- `cargo-synos`
- `synos-compiler`
- `synos-test-support`
- `synos-uefi`
- `synos-vm`

The compiler driver runs on the build host. The output is native SynOS code.
Self-hosting Rust compilation inside a running SynOS instance needs a SynOS
`std` port, process loader, dynamic library support, compiler package, and a
filesystem-backed compiler service. Those are separate runtime work, not a
property that can be claimed from a host Cargo command.

The current service boundary is represented by `synos-rustd`. Its requests are
fixed-size and include the source root, manifest, target, profile, lock policy,
network policy, and resource limits. The default policy denies network access.
During user-space boot, `CompilerServiceBoot` verifies the package gate,
registers the service with init, and starts it under the compiler capability
profile. `CompilerServiceHealthCheck` makes a system update fail and roll back
when the service is not running or the active root is not bound to that
package. The host driver accepts matching `--locked` and `--offline` flags and
can write a signed package atomically after a successful build. Its
`compile_synfs` path validates the source root and manifest through SynFS,
copies the bounded project tree into an isolated Cargo workspace, and compiles
that snapshot with the same target and lock policy.

## Frozen design

The primary self-hosting target is `x86_64-unknown-synos`. The
`aarch64-unknown-synos` target follows after x86_64 self-hosting works. The
initial Rust surface includes `core`, `alloc`, `std`, Cargo, build scripts, proc
macros, tests, and rustdoc. The compiler stack is upstream `rustc` plus LLVM.

SynFS layout is fixed: `/system/toolchains` contains stage-0, stage-1, and
stage-2 toolchains; `/system/registries` contains signed registries;
`/system/sources` contains source snapshots; `/system/builds` contains build
state; `/system/tmp` contains scratch data; and `/system/bundles` contains
output bundles. These paths are exported as constants by `synos-rustd`.

Compiler IPC is protocol version 1 with fixed-size frames. It supports
`Submit`, `Start`, `Poll`, `Cancel`, `Release`, and `ReadLog`. Responses use
existing `synos-status` values. Log records contain a job id, sequence number,
level, event, and bounded message. Queued jobs cancel immediately; running
jobs get a cooperative stop and are fenced after a five-second grace period;
terminal jobs reject cancellation; partial output is never published.

## Native process contract

`synos-app` now owns the Ring 3 side of native process loading. Its ELF64
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

`synos-rustd` also has a signed `ToolchainManifest`. It requires Cargo, rustc,
and the linker, optionally requires build-script and proc-macro runners, then
executes the ordered plan through a process-runtime hook. Every tool receives
the bounded build request, and failed tools are fenced before the job fails.
The native loader and SynOS `std` PAL still need to provide that hook for true
in-guest execution.

Signed compiler output can now be launched through `synos-app`. The supervisor
validates the package instantiation receipt and entry point, authorizes the
manifest's requested capabilities against policy, and passes only the approved
capabilities plus the package payload identity to the process runtime. The
native executable-image loader still must map that payload before the process
can run on a booted SynOS instance.

The self-host path is represented by `SelfHostSession`. It submits and validates
the runtime build first, refuses unlocked or networked requests, then submits
the compiler build only after the runtime artifact is complete. Both results
must carry non-zero package and payload identities for the session to finish.

Reproducibility is checked with:

```sh
cargo run -p cargo-synos -- synos reproduce --target x86_64 --release
```

The command creates one fresh source workspace, builds it twice from clean
target state with `--locked --offline`, disables incremental state, remaps
workspace paths, and compares the final `.rlib`, `.rmeta`, `.a`, `.o`, and
`.so` artifacts by content ID. For `.rlib` files, it hashes code archive
members while ignoring Cargo metadata whose generated declaration order is
not stable.

The boot contract is now wired, but the actual native executable still needs
the SynOS `std` port, process loader, and service image described in
`TODO-compiler.md`.

The compiler service boundary now includes the operational Ring 3 path:
signed local-registry pins are resolved by content ID, source reads require a
capability-bound `SourceGrant`, and each job receives independent build and
scratch roots. `CompilerService::handle_ipc` streams state and structured log
records; `tick` handles deadline expiry and cooperative cancellation; a
`BuildWorkspaceRuntime` implementation performs the actual SynFS cleanup.
Successful results enter the immutable cache only after source, lockfile,
toolchain, target, profile, and feature identities all match.

For the complete host-side step, use `cargo-synos package` with
`--manifest-path`, `--locked`, `--offline`, `--key`, and `--output`.

## Rust application bundles

`cargo-synos package --app-manifest` turns a linked ELF into a signed SynOS
application. The profile requires `schema`, application `name`, `image`,
`kind`, `target`, and `entry_offset`, plus all three resource fields:

```toml
schema = 1

[application]
name = "demo"
image = "0x1"
kind = "service"
target = "x86_64-unknown-synos"
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
cargo synos package --bin demo --app-manifest App.toml \
  --target x86_64 --release --locked --offline \
  --key compiler.key --output demo.synapp \
  --debug-symbols demo.debug --stripped-output demo.stripped
```

The runtime publishes this output with
`PackageDaemon::install_application_bundle`. Launch checks the trusted
signature, target, entry point, dependency closure, and resource limits before
the normal ELF loader receives the image.

## Toolchain package

The host bootstrap can package the complete Rust toolchain needed by the
native service:

```sh
cargo run -p cargo-synos -- synos toolchain package \
  --stage 0 --target x86_64 --key compiler.key --output stage-0.synpkg
cargo run -p cargo-synos -- synos toolchain verify \
  --bundle stage-0.synpkg --key compiler.key
```

Stage 1 and stage 2 roots use the same format. Their root contains `bin/`
with Cargo, rustc, rustdoc, and rust-lld, plus `sysroot/`,
`target-libraries/`, and `rust-src/`; pass that root with `--root` and the
matching stage number. The archive uses stable sorted paths, content IDs for
every file, and no symbolic links. `synos-pkg` verifies the signed outer
bundle before SynFS installation, while `synos-rustd` authorizes each
component and revokes build-script and proc-macro images when the build ends.
