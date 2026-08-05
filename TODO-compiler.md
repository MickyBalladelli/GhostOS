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
- [ ] Launch that bundle as a capability-limited process.
- [ ] Compile the compiler and its runtime again from inside SynOS.
- [ ] Reproduce the same result in an offline build from a clean workspace.

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
- `synos-rustd` has fixed-size build requests, bounded job state, quotas,
  cancellation, deadline expiry, deny-by-default network policy, and
  content-addressed cache keys.
- `synos-compiler` supports locked/offline Cargo builds and compile-to-signed
  bundle output.
- `cargo synos package` accepts `--manifest-path`, `--locked`, `--offline`,
  and `--target-dir`.

## 1. Freeze the design

- [ ] Choose the first supported self-hosting target: `x86_64-unknown-synos`.
- [ ] Add `aarch64-unknown-synos` after the x86_64 path is self-hosting.
- [ ] Decide the initial supported Rust surface: `core`, `alloc`, `std`, Cargo,
  build scripts, proc macros, tests, and rustdoc.
- [ ] Choose the compiler stack: upstream `rustc` plus LLVM, or a separate
  SynOS compiler frontend/backend.
- [ ] Define the on-disk layout for toolchains, registries, sources, build
  state, temporary files, and output bundles on SynFS.
- [ ] Define the compiler-service IPC protocol, status codes, logs, and
  cancellation rules.

The bounded request and job protocol model is implemented in
`crates/synos-rustd`; the real SynOS IPC service is still open.

## 2. Make native compiler processes run

- [ ] Implement the SynOS executable-image loader for native Rust artifacts.
- [ ] Map code, read-only data, writable data, heap, stack, and guard pages.
- [ ] Enforce W^X and non-executable writable memory.
- [ ] Implement relocations, entry-point setup, `argv`, environment, TLS, and
  process exit status.
- [ ] Implement process `spawn`, `exec`, `wait`, cancellation, and resource
  limits for compiler jobs.
- [ ] Connect process launch to application manifests, capability policy,
  supervisor restart policy, and signed-package instantiation receipts.
- [ ] Add executable-page measurement and integrity checks for compiler and
  generated application images.

## 3. Finish the Rust runtime needed by rustc and Cargo

- [ ] Complete the native `std::sys::synos` PAL for files, directories,
  metadata, paths, environment variables, arguments, time, threads, locks,
  condition variables, pipes, and process status.
- [ ] Add the missing runtime ABI operations for compiler needs: random data,
  terminal I/O, process control, memory protection, and capability discovery.
- [ ] Implement native signal/panic/unwind behavior or document the supported
  panic and abort model.
- [ ] Implement thread-local storage and the runtime pieces required by
  `std`, `backtrace`, and dynamic loading.
- [ ] Make `std`, `alloc`, and `core` build and run on both SynOS targets.
- [ ] Remove host-only assumptions from Cargo, rustc wrappers, linker
  discovery, temporary directories, and environment handling.

## 4. Bring the toolchain into SynOS

- [ ] Package `rustc`, `rustdoc`, Cargo, `rust-lld` or the selected linker,
  LLVM/codegen support, the Rust sysroot, target libraries, and source code.
- [ ] Sign the compiler package and verify it through `synos-pkg` before launch.
- [ ] Build a stage-0 native toolchain using the existing host-side compiler
  driver.
- [ ] Build stage 1 of the toolchain for SynOS.
- [ ] Use stage 1 to build stage 2 inside SynOS.
- [ ] Make the stage-2 compiler compile its own source and compare its output
  with the trusted stage-1 build.
- [ ] Add target-aware linker configuration without relying on host paths.
- [ ] Support host tools that must run during a build: build scripts, proc
  macros, code generators, and test binaries.
- [ ] Define how dynamic proc-macro and build-script artifacts are loaded,
  verified, isolated, and removed after a build.

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
