# Rust to C migration

- [ ] Migrate the entire project from Rust to C. Port every Rust module listed below, including its public interfaces, behavior, platform-specific code, tests, benchmarks, build scripts, and examples. Preserve existing ABI, on-disk formats, and documented behavior.
- [ ] Replace the root Cargo workspace and all Cargo package/build configuration with the C build system, including dependency management, cross-compilation targets, and release settings.
- [ ] Port Rust tooling: `tools/cargo-ghostos`, `tools/ghostos-compiler`, and `crates/ghostos-rustd`.
- [ ] Port Rust test and fuzz infrastructure, including Rust integration/unit tests, `kernel/benches`, `virtual_machine/benches`, and all targets under `fuzz/fuzz_targets`.
- [ ] Update scripts, documentation, packaging, boot, and release workflows that invoke Cargo, Rust tools, or Rust-built artifacts.
- [ ] After the C replacements are complete and verified, move obsolete Rust build output and other obsolete Rust-only generated/configuration folders into the project `Trash/` folder. Keep source history and user data intact. No `target`, `build`, `dist`, or `node_modules` directories were present during this inventory.

## Migration progress

The complete migration is in progress. About 239,827 Rust source lines need to
be ported. The OS, VM, and service consumers still run Rust; no complete project
cutover has occurred.

- [x] Add a freestanding C11 foundation library build (`Makefile`, `c/`).
- [x] Port status values, validation, public errors, retry advice, and operator messages to C.
- [x] Generate C syscall/RPC ABI bindings from the existing shared TOML schema.
- [x] Preserve syscall layouts and RPC encoding in the C ABI implementation.
- [x] Port API compatibility contracts, migration advice, and error formatting to C.
- [x] Port protocol version negotiation, replay protection, authentication limits, backpressure, and reconnect policy to C.
- [x] Port boot handoff structures, memory-region validation, and framebuffer validation to C.
- [x] Port foundation contract test cases to C source (not executed).
- [ ] Port the shared test-support property harness; C generated-input cases currently use a local deterministic generator.
- [ ] Connect the C foundations to the kernel, VM, and services after those consumers are ported.
- [ ] Establish behavior parity before checking off the module and full migration tasks.

The status, ABI, compatibility, protocol, and boot-protocol C implementations
build into `build/c/libghostos.a`. See `c/README.md` for the source mapping and
interface conventions. Rust module checklist entries remain open until consumer
cutover and behavior parity are complete.

## Rust source inventory

Each entry names the project area containing Rust source files. Port every `.rs` file in the listed directory tree, including nested `src`, `tests`, `benches`, `examples`, and build-script files.

- [ ] `boot/uefi/` — UEFI bootloader.
- [ ] `crates/abi/` — ABI definitions and generated ABI bindings.
- [ ] `crates/actors/` — actor runtime.
- [ ] `crates/admission/` — admission control.
- [ ] `crates/api-compat/` — API compatibility.
- [ ] `crates/app/` — application loading, manifests, and supervision.
- [ ] `crates/auth/` — authentication.
- [ ] `crates/balancerd/` — balancing daemon.
- [ ] `crates/boot-protocol/` — boot protocol and handoff.
- [ ] `crates/client-sdk/` — client SDK and wire protocol.
- [ ] `crates/compute/` — tensor and accelerator compute.
- [ ] `crates/durability/` — durability contracts.
- [ ] `crates/fabric/` — cluster fabric, memory, CXL, and DSM.
- [ ] `crates/fsd/` — filesystem daemon.
- [ ] `crates/ghostfs/` — GhostFS storage and volume management.
- [ ] `crates/ghostos-agent-bridge/` — agent bridge.
- [ ] `crates/ghostos-agentd/` — agent daemon.
- [ ] `crates/ghostos-audit/` — audit subsystem.
- [ ] `crates/ghostos-backup/` — backup and recovery.
- [ ] `crates/ghostos-confidential/` — confidential computing.
- [ ] `crates/ghostos-debug/` — debugging subsystem.
- [ ] `crates/ghostos-declarative/` — declarative configuration.
- [ ] `crates/ghostos-embedded-script/` — embedded scripting.
- [ ] `crates/ghostos-heal/` — health and recovery.
- [ ] `crates/ghostos-inference/` — inference service and protocol.
- [ ] `crates/ghostos-inspect/` — inspection and diagnostics.
- [ ] `crates/ghostos-kvd/` — key-value daemon.
- [ ] `crates/ghostos-mesh/` — mesh networking.
- [ ] `crates/ghostos-remote-display/` — remote display and WebRTC.
- [ ] `crates/ghostos-replay/` — deterministic replay.
- [ ] `crates/ghostos-rustd/` — Rust toolchain daemon and workflows; replace its Rust-specific role as part of the migration.
- [ ] `crates/ghostos-script/` — script parser, wire format, reflection, and runtime.
- [ ] `crates/ghostos-shell/` — shell interpreter and commands.
- [ ] `crates/ghostos-shield/` — security shield, attestation, and response.
- [ ] `crates/ghostos-storaged/` — storage daemon, protocol, and tiering.
- [ ] `crates/ghostos-top/` — system dashboard and telemetry.
- [ ] `crates/ghostos-update/` — update, rollout, and hot-swap.
- [ ] `crates/ghostos-wasm-script/` — WebAssembly scripting.
- [ ] `crates/ghostos-webterm/` — web terminal and SSH.
- [ ] `crates/host-filesystems/` — FAT32, NTFS, and ext4 host filesystem support.
- [ ] `crates/http/` — HTTP implementation.
- [ ] `crates/init/` — initialization and fault domains.
- [ ] `crates/ipc/` — inter-process communication.
- [ ] `crates/legacy-pc-drivers/` — legacy PC block, PCI, storage, USB, and Ethernet drivers.
- [ ] `crates/llm-runtime/` — LLM inference runtime and KV cache.
- [ ] `crates/logd/` — logging daemon.
- [ ] `crates/netd/` — network daemon.
- [ ] `crates/numa/` — NUMA support.
- [ ] `crates/observability/` — profiling, telemetry, SLOs, and scaling.
- [ ] `crates/path-pattern/` — path pattern matching.
- [ ] `crates/pkg/` — package management and signatures.
- [ ] `crates/platform-io/` — platform I/O abstraction.
- [ ] `crates/policy/` — policy engine.
- [ ] `crates/posix-compat/` — POSIX compatibility.
- [ ] `crates/power/` — power management.
- [ ] `crates/protocol/` — shared protocol definitions.
- [ ] `crates/ras/` — reliability, availability, and serviceability.
- [ ] `crates/rms/` — record management and storage.
- [ ] `crates/runtime/` — core runtime.
- [ ] `crates/service-scale/` — service scaling.
- [ ] `crates/status/` — status types and reporting.
- [ ] `crates/system-model/` — system models.
- [ ] `crates/test-support/` — test support and crash harnesses; replace with C test support.
- [ ] `crates/time-sync/` — time synchronization and PTP wire support.
- [ ] `examples/compiler-acceptance/` — compiler acceptance example, including its proc-macro subcrate and build script.
- [ ] `examples/cookbook/` — cookbook binaries and examples.
- [ ] `examples/hello-world/` — hello-world application.
- [ ] `fuzz/` — Rust fuzz package, targets, and associated build configuration.
- [ ] `kernel/` — kernel, architecture support, tests, benchmarks, and `build.rs`.
- [ ] `tools/cargo-ghostos/` — Cargo subcommand; replace with C project tooling.
- [ ] `tools/ghostos-compiler/` — GhostOS compiler tooling.
- [ ] `userspace/boot-services/rust-shell/` — Rust shell boot service; integrate its behavior into the C boot services.
- [ ] `virtual_machine/` — virtual machine, devices, CPU, memory, tests, and benchmarks.

Inventory: 545 Rust source files and 75 Cargo manifests were found. The root workspace lists 71 members; the remaining manifests include the fuzz package and nested example/proc-macro packages.
