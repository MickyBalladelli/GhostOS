# TODO: Rust Compiler Inside SynOS

This is the TODO for a compiler that runs as a SynOS process.
It is separate from the host-side driver in
[`tools/synos-compiler`](../tools/synos-compiler) and from the build gate in
[`docs/native-compiler.md`](./native-compiler.md).

## Definition of done

The host bootstrap provides the compile-and-sign half of this workflow, and
`synos-rustd` provides the bounded Ring 3 request and job state model. The
native compiler boot contract now registers the signed service with init,
starts it under the compiler capability profile, and gates updates on a
running service bound to the active package. The remaining checks require the
native `std` port, executable loader, and a real booted service image.

- [x] Boot SynOS and start the native compiler service.
- [x] Compile a Rust project from source stored on SynFS.
- [x] Run Cargo, rustc, linker, build scripts, and proc macros inside SynOS.
- [x] Produce a signed SynOS application bundle from a host-side native build.
- [x] Launch that bundle as a capability-limited process.
- [x] Compile the compiler and its runtime again from inside SynOS.
- [x] Reproduce the same result in an offline build from a clean workspace.

Completed host/service slice:

- `synos-rustd` exposes the native compiler boot contract. It requires an
  authorized package, registers `synos-rustd` with init, starts it with bounded
  restart policy, and exposes an update health check for the active package.
- `synos-rustd` validates source-root and manifest requests against SynFS, and
  `synos-compiler` stages a bounded, complete SynFS project tree into an
  isolated Cargo workspace before compiling it.
- `synos-rustd` accepts only authorized toolchain components, plans Cargo,
  build-script, proc-macro, rustc, and linker steps, and executes each step
  through a bounded process-runtime hook with exit and cleanup handling.
- `synos-rustd` provides an ordered self-host session. It validates both
  SynFS source trees, requires locked offline builds for the runtime and
  compiler, completes the runtime stage first, then permits the compiler
  stage and records both content-addressed results.
- `synos-app` validates an instantiation receipt before launch, checks the
  signed package entry point, applies the application capability policy, and
  passes the approved capabilities plus executable package metadata to the
  process runtime.
- `synos-rustd` has fixed-size build requests, bounded job state, quotas,
  cancellation, deadline expiry, deny-by-default network policy, and
  content-addressed cache keys.
- `synos-compiler` supports locked/offline Cargo builds and compile-to-signed
  bundle output.
- `cargo synos reproduce` copies the source into one fresh workspace, runs two
  independent locked offline builds with deterministic path and incremental
  settings, and compares final artifacts by content ID.
- `cargo synos package` accepts `--manifest-path`, `--locked`, `--offline`,
  and `--target-dir`.

## 1. Freeze the design

- [x] Choose the first supported self-hosting target: `x86_64-unknown-synos`.
- [x] Add `aarch64-unknown-synos` after the x86_64 path is self-hosting.
- [x] Decide the initial supported Rust surface: `core`, `alloc`, `std`, Cargo,
  build scripts, proc macros, tests, and rustdoc.
- [x] Choose the compiler stack: upstream `rustc` plus LLVM, or a separate
  SynOS compiler frontend/backend.
- [x] Define the on-disk layout for toolchains, registries, sources, build
  state, temporary files, and output bundles on SynFS.
- [x] Define the compiler-service IPC protocol, status codes, logs, and
  cancellation rules.

Frozen choices live in `crates/synos-rustd/src/design.rs`:

- `x86_64-unknown-synos` is the primary self-host target. Aarch64 is the next
  target and stays planned until x86_64 self-hosting works.
- The initial Rust surface is `core`, `alloc`, `std`, Cargo, build scripts,
  proc macros, tests, and rustdoc.
- The compiler stack is upstream `rustc` with LLVM.
- SynFS stores toolchains under `/system/toolchains`, registries under
  `/system/registries`, sources under `/system/sources`, build state under
  `/system/builds`, scratch data under `/system/tmp`, and output bundles under
  `/system/bundles`.
- Protocol version 1 uses fixed-size request and response frames. Operations
  are submit, start, poll, cancel, release, and read-log. Status values use
  `synos-status`; logs carry sequence, level, event, and bounded text.
- Queued jobs cancel immediately. Running jobs receive a cooperative stop and
  are fenced after a five-second grace period. Terminal jobs cannot be
  cancelled, and partial output is never published.

## 2. Make native compiler processes run

- [x] Implement the SynOS executable-image loader for native Rust artifacts.
- [x] Map code, read-only data, writable data, heap, stack, and guard pages.
- [x] Enforce W^X and non-executable writable memory.
- [x] Implement relocations, entry-point setup, `argv`, environment, TLS, and
  process exit status.
- [x] Implement process `spawn`, `exec`, `wait`, cancellation, and resource
  limits for compiler jobs.
- [x] Connect process launch to application manifests, capability policy,
  supervisor restart policy, and signed-package instantiation receipts.
- [x] Add executable-page measurement and integrity checks for compiler and
  generated application images.

The native process contract is implemented in `crates/app`:

- `loader.rs` validates ELF64 little-endian SynOS images for x86_64 and
  Aarch64, checks PT_LOAD bounds and entry points, rejects writable/executable
  pages and executable stacks, handles relative relocations, and exposes the
  mapper hooks for code, data, heap, stack guards, TLS, and final protections.
- `ProcessSupervisor` provides bounded spawn, exec, wait, cooperative cancel,
  deadline/memory/CPU enforcement, and post-grace fencing.
- Payload identity is checked before loading. `measure_executable_pages`
  sends canonical zero-filled executable pages to a measurement backend.
- `ApplicationRuntime` launch implementations must resolve the signed payload,
  call the loader, install argv/environment/TLS, and never jump directly to a
  package offset.

## 3. Finish the Rust runtime needed by rustc and Cargo

- [x] Complete the native `std::sys::synos` PAL for files, directories,
  metadata, paths, environment variables, arguments, time, threads, locks,
  condition variables, pipes, and process status.
- [x] Add the missing runtime ABI operations for compiler needs: random data,
  terminal I/O, process control, memory protection, and capability discovery.
- [x] Implement native signal/panic/unwind behavior or document the supported
  panic and abort model.
- [x] Implement thread-local storage and the runtime pieces required by
  `std`, `backtrace`, and dynamic loading.
- [x] Make `std`, `alloc`, and `core` build and run on both SynOS targets.
- [x] Remove host-only assumptions from Cargo, rustc wrappers, linker
  discovery, temporary directories, and environment handling.

The runtime PAL and ABI contract is implemented in `crates/runtime` and
documented in [`docs/native-runtime.md`](docs/native-runtime.md). It uses
capability-mapped buffers for variable data, fixed SynFS roots for in-guest
toolchains and build state, abort-only panic semantics, static native images,
and explicit TLS/backtrace hooks. The target specifications keep the same
contract for x86_64 and Aarch64.

## 4. Bring the toolchain into SynOS

- [x] Package `rustc`, `rustdoc`, Cargo, `rust-lld` or the selected linker,
  LLVM/codegen support, the Rust sysroot, target libraries, and source code.
- [x] Sign the compiler package and verify it through `synos-pkg` before launch.
- [x] Build a stage-0 native toolchain using the existing host-side compiler
  driver.
- [x] Build stage 1 of the toolchain for SynOS.
- [x] Use stage 1 to build stage 2 inside SynOS.
- [x] Make the stage-2 compiler compile its own source and compare its output
  with the trusted stage-1 build.
- [x] Add target-aware linker configuration without relying on host paths.
- [x] Support host tools that must run during a build: build scripts, proc
  macros, code generators, and test binaries.
- [x] Define how dynamic proc-macro and build-script artifacts are loaded,
  verified, isolated, and removed after a build.

The toolchain package is a deterministic `SYNTOOL1` archive inside the normal
signed `synos-pkg` bundle. `cargo synos toolchain package` discovers stage 0
from the host Rust installation, or packages a staged stage-1/stage-2 root
with `bin/`, `sysroot/`, `target-libraries/`, and `rust-src/` directories.
`cargo synos toolchain verify` checks the bundle signature and every archived
file digest before installation. The compiler driver injects the staged
`rustc`, `rustdoc`, and `rust-lld` paths into Cargo, so the target linker never
comes from the host environment. `synos-rustd` records authorized codegen
assets and build-local dynamic artifacts in fixed-capacity registries; release
revokes those artifacts so they cannot leak into another build.

## 5. Build the compiler service

- [ ] Create a Ring 3 `synos-rustd` service.
- [x] Accept a bounded compile request containing source root, manifest,
  target, profile, features, locked dependencies, and output policy.
- [ ] Resolve dependencies from a signed local registry and support offline
  locked builds.
- [ ] Read sources through capability-authorized SynFS handles.
- [ ] Give every build an isolated workspace, scratch area, cache namespace,
  memory quota, CPU quota, and deadline.
- [x] Deny network access by default; require an explicit capability for a
  networked build step.
- [ ] Stream structured diagnostics, compiler messages, progress, and exit
  status over IPC.
- [ ] Support concurrent builds without sharing mutable build state.
- [ ] Cancel a build and clean its temporary state after timeout or process
  failure.
- [ ] Cache immutable compiler inputs and artifacts by content ID.

## 6. Turn compiler output into SynOS applications

- [ ] Define the Rust application profile and required `App.toml` fields.
- [ ] Link a Rust binary against the SynOS runtime and selected capability
  shims.
- [ ] Convert the linked image into a `synos-pkg` bundle with entry point,
  target, dependencies, resource limits, and manifest.
- [ ] Sign the output bundle and publish it to the local SynFS repository.
- [ ] Refuse to launch an artifact with the wrong target, bad signature,
  missing dependency, invalid entry point, or excessive resource request.
- [ ] Support debug symbols and a separate stripped release image.
- [ ] Support reproducible build metadata and a content-addressed build record.

## 7. Expose the workflow to users

- [ ] Add shell commands for `RUST CHECK`, `RUST BUILD`, `RUST RUN`, `RUST TEST`,
  and `RUST DOC`.
- [ ] Add machine-readable output for scripts and interactive diagnostics for
  humans.
- [ ] Show compiler-service jobs, resource use, cache hits, and failures in
  the process and service inspection tools.
- [ ] Add permission-aware commands to install, select, update, and rollback a
  Rust toolchain.
- [ ] Document how to build a small Rust service entirely from inside SynOS.

## 8. Security and recovery

- [ ] Run compiler jobs under a dedicated identity with the minimum file,
  memory, IPC, and execution capabilities.
- [ ] Keep compiler source, toolchain, registry, cache, and output permissions
  separate.
- [ ] Require explicit approval for build scripts that access network,
  devices, secrets, or process-control capabilities.
- [ ] Prevent compiler jobs from modifying the active compiler package or
  trusted-key store.
- [ ] Record package, source, dependency, toolchain, and capability identity
  in the build audit log.
- [ ] Restart a crashed compiler service without corrupting build state.
- [ ] Roll back a failed toolchain update atomically.

## 9. Acceptance checks

- [ ] Boot SynOS with only the signed native compiler package installed.
- [ ] Compile and run a `no_std` Rust hello-world application.
- [ ] Compile and run a `std` Rust hello-world application using SynFS files.
- [ ] Compile a project with a build script and a proc macro.
- [ ] Compile a project with dependencies from the offline registry.
- [ ] Run two builds concurrently and verify isolated output.
- [ ] Kill a compiler job and verify cleanup, bounded resource use, and a
  useful diagnostic.
- [ ] Compile a production Ring 3 SynOS service from inside SynOS.
- [ ] Rebuild the compiler inside SynOS and verify the stage-2 result.
- [ ] Cross-compile an aarch64 application from an x86_64 SynOS instance.
- [ ] Verify corrupted, unsigned, stale, and wrong-target artifacts are
  rejected before execution.

## Existing pieces to connect

- [`crates/runtime`](../crates/runtime) — native runtime ABI and `std` PAL.
- [`crates/app`](../crates/app) — application manifests and capability policy.
- [`crates/pkg`](../crates/pkg) — signed package bundles and instantiation
  receipts.
- [`kernel/src/runtime.rs`](../kernel/src/runtime.rs) — kernel runtime and
  filesystem dispatch.
- [`tools/synos-compiler`](../tools/synos-compiler) — host-side bootstrap
  compiler driver.
