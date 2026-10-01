# Full Rust to C migration

- [ ] Port the entire project from Rust to C. The end state contains no Rust source, Rust build tooling, Cargo configuration, or Rust-only project workflows. Preserve public behavior, supported platforms, syscall and RPC ABI, on-disk formats, and documented contracts.
- [ ] Replace the root Cargo workspace and every nested Cargo package, lockfile, Cargo configuration, and Rust toolchain pin with the C build system. Preserve dependency management, cross-compilation targets, and release settings.
- [ ] Port all Rust project tooling and its behavior: `tools/cargo-ghostos`, `tools/ghostos-compiler`, and `crates/ghostos-rustd`.
- [ ] Port all Rust tests, benchmarks, property/model tests, and fuzz infrastructure, including `kernel/benches`, `virtual_machine/benches`, and every target under `fuzz/fuzz_targets`.
- [ ] Update scripts, documentation, packaging, boot, and release workflows to remove Cargo, Rust tools, and Rust-built artifact dependencies.
- [ ] After all replacements are complete and verified, move obsolete Rust-only generated/build/configuration files into the project `Trash/` folder. Keep source history and user data intact. Do not move active C build output or delete files.

## Migration progress

The complete migration is in progress. About 239,827 Rust source lines need to
be ported. The OS, VM, and service consumers still run Rust; no complete project
cutover has occurred. Every Rust source area is listed below, including the VM,
kernel, bootloader, services, tooling, examples, tests, and fuzz targets.

- [x] Add a freestanding C11 foundation library build (`Makefile`, `c/`).
- [x] Port status values, validation, public errors, retry advice, and operator messages to C.
- [x] Generate C syscall/RPC ABI bindings from the existing shared TOML schema.
- [x] Preserve syscall layouts and RPC encoding in the C ABI implementation.
- [x] Port API compatibility contracts, migration advice, and error formatting to C.
- [x] Port protocol version negotiation, replay protection, authentication limits, backpressure, and reconnect policy to C.
- [x] Port boot handoff structures, memory-region validation, and framebuffer validation to C.
- [x] Port foundation contract test cases to C source (not executed).
- [x] Port the shared test-support property harness, including Rust-compatible entropy, case seeds, replay controls, failure reports, generators, and queue/capability/lease models; connect the C generated-input cases to it.
- [ ] Connect the C foundations to the kernel, VM, and services as those consumers are ported.
- [x] Connect the existing C boot service, login service, and C shell source to generated syscall ABI types, operation IDs, and status constants; add shared-header dependencies to the kernel build.
- [ ] Port the kernel, virtual machine, and every remaining service consumer to C. The active Ring 3 shell is still Rust; the C shell source is not yet the active shell image.
- [ ] Establish behavior parity before checking off the module and full migration tasks.

The status, ABI, compatibility, protocol, and boot-protocol C implementations
build into `build/c/libghostos.a`. See `c/README.md` for the source mapping and
interface conventions. Rust module checklist entries remain open until consumer
cutover and behavior parity are complete.

## Rust source modules to port

### Kernel and virtual machine modules

Port each Rust source module to C and preserve its behavior.

- [x] `kernel/src/address_space.rs`
- [x] `kernel/src/allocator.rs`
- [x] `kernel/src/arch/aarch64.rs`
- [x] `kernel/src/arch/cpu.rs`
- [x] `kernel/src/arch/mod.rs`
- [x] `kernel/src/arch/riscv64.rs`
- [x] `kernel/src/arch/unsupported.rs`
- [x] `kernel/src/arch/x86_64.rs`
- [x] `kernel/src/boot_diagnostics.rs`
- [x] `kernel/src/boot_services.rs` — C port has the 13-service registry and dependency ordering, kernel launch callback integration, owned filesystem lifecycle, transactional first-admin provisioning and recovery, durable passkey key/counter records, filesystem-rights enforcement, and session-scoped shell authority. The C runtime consumes kernel and filesystem callbacks so platform implementations provide process launch and filesystem operations.
- [x] `kernel/src/capability.rs`
- [x] `kernel/src/console.rs`
- [x] `kernel/src/contention.rs`
- [x] `kernel/src/cow.rs` — C reference tracker, sharing and release, allocator-backed write faults, and page-copy callback.
- [x] `kernel/src/crash.rs` — kernel builds and links the C capsule encoder and one-time crash guard; the kernel supplies register, capability, scheduler, audit, and persistence data.
- [ ] `kernel/src/dlm.rs` — C fence tables, capability checked range locks, lease epochs, FIFO promotion, and contention reports are implemented and built. Shell diagnostics read the C singleton, but kernel lock operations still use the Rust manager; complete consumer cutover and behavior parity remain.
- [ ] `kernel/src/dma.rs` — C port adds capability checked buffer/device authorization, bounded IOVA allocation, IOMMU map/unmap callbacks, and fixed-capacity records. Rust kernel callers remain active; behavior parity and consumer cutover remain.
- [ ] `kernel/src/driver_capabilities.rs` — C port builds driver resource manifests, creates DMA/MMIO capabilities, and assigns validated MMIO mappings from the PCI inventory. Rust kernel callers remain active; behavior parity and consumer cutover remain.
- [ ] `kernel/src/hot_allocator.rs` — C port implements bounded per-CPU/per-node object pools, local and remote fallback, reclaim validation, placement/probe counters, and fragmentation reports. Rust kernel callers remain active; behavior parity and consumer cutover remain.
- [ ] `kernel/src/invariants.rs` — C port provides the stable six-entry invariant catalogue, redacted failure identifiers/codes, formatting, debug trap hook, and address-space, interrupt, and page-table checks. Rust kernel call sites remain active; behavior parity and consumer cutover remain.
- [ ] `kernel/src/ipc.rs` — C port implements bounded lock-free MPMC channels, capability-checked send/receive and transfers, quotas, mapped endpoints, close/owner cleanup, partition and scheduler callbacks, diagnostics, and stuck reports. Rust kernel consumers remain active; behavior parity and consumer cutover remain.
- [ ] `kernel/src/keyboard.rs` — C port includes the PS/2 controller setup, Set 1 key decoding, modifiers, extended-key sequences, ACK filtering, and mouse-byte callback. Rust kernel and shell callers remain active; behavior parity and consumer cutover remain.
- [ ] `kernel/src/keyboard_stub.rs` — C stub initializes empty state and always reports no key, matching non-x86 kernel behavior. Rust callers remain active; behavior parity and consumer cutover remain.
- [ ] `kernel/src/lib.rs` — C port adds the aggregate C kernel API, callback-driven boot coordinator, boot validation and stage reporting, dispatch boundary, service readiness, fatal/crash path, and login throttling/session/quote state. The Rust syscall dispatcher, hardware consumers, process/scheduler integration, and shell remain active; parity and consumer cutover remain.
- [ ] `kernel/src/litmus.rs` — C port includes deterministic seeded schedules, all six kernel ordering models, fault replay, and failure minimization. Rust callers remain active; behavior parity and caller cutover remain.
- [ ] `kernel/src/main.rs` — C port owns the target `_start` entry symbol and forwards boot info to `kernel_entry`. Rust keeps only the compiler-required panic ABI hook, which forwards to kernel panic reporting; full panic-handler cutover remains.
- [ ] `kernel/src/micro_silo.rs` — C port includes hardware protection selection, tenant-only authorization, bounded non-overlapping memory maps, borrowed-range cleanup, and address lookup. Rust callers remain active; behavior parity and caller cutover remain.
- [ ] `kernel/src/monitor.rs` — C port includes monitor view state, generation-1 process snapshots, switch-history CPU utilization, live kernel DLM lock summaries, empty DSM page stats, and all four text views. Rust scheduler and shell callers remain active; behavior parity and caller cutover remain.
- [ ] `kernel/src/mouse.rs` — active x86 kernel wrapper now calls the C atomic three-byte PS/2 collector and decoder; non-x86 keeps `mouse_stub.rs`. Rust keyboard caller remains through the stable `ingest`/`state` API; target build and behavior parity remain.
- [ ] `kernel/src/mouse_stub.rs` — non-x86 Rust API now calls a C stub that always returns an empty mouse state. Caller and target build parity remain.
- [ ] `kernel/src/page_fault.rs` — C now owns x86 fault decoding, one-time handler install and dispatch, quota consume/refund ordering, stack-map-before-commit ordering, and COW write sequencing. Rust adapters retain the native capability, allocator, and address-space objects; moving those objects and full cutover remain.
- [ ] `kernel/src/partition.rs` — active kernel methods call C for CPU mask queries, housekeeping policy, and online/isolate/release transitions. Host builds retain Rust-only fallback; scheduler still owns the partition state, so full state cutover remains.
- [ ] `kernel/src/pci.rs` — C owns the x86 CF8/CFC scan and bounded 64-device inventory. Rust adapts entries to the existing boot-facing iterator and logging; driver and storage consumers still use that adapter.
- [ ] `kernel/src/persistence.rs` — C performs bounded x86 persistence-port load/save/flush. Rust retains record-container encoding and boot/crash record policy through the C storage adapter.
- [x] `kernel/src/persona.rs` — C owns fixed-capacity active/disabled rights state operations. Rust keeps typed identity/right wrappers and the iterator used by scheduler and authentication callers.
- [ ] `kernel/src/physical_storage.rs` — C selects bounded AHCI candidates and owns missing-volume policy. Rust still owns the AHCI controller, GhostFS mount, and service-image handoff because those APIs remain Rust-only.
- [ ] `kernel/src/power.rs` — C owns checked ACPI memory/register access, ACPI enable/event/sleep/reset control, and VM shutdown/reboot fallbacks. Rust keeps ACPI table discovery and battery AML parsing through the existing power crate.
- [ ] `kernel/src/process.rs` — C owns bounded process records, ID generations, status, cancellation, exec updates, exits, and fencing. Rust retains ELF loading and scheduler/capability/memory resource callbacks.
- [ ] `kernel/src/quota.rs` — C owns sharded token buckets, memory accounting, refill/throttle policy, and ticket-lock contention reports. Rust keeps the public quota API and adapts results for capability and allocator callers.
- [ ] `kernel/src/random.rs` — C owns processor entropy detection, seed mixing, ChaCha20 blocks, counter allocation, and bounded output. Rust keeps the slice and `Option<u64>` API.
- [ ] `kernel/src/runtime.rs` — C owns bounded filesystem process registration, runtime clock, ABI and filesystem request validation, buffer descriptor validation, response validation, and operation routing. Rust retains typed ABI translation and capability-protected IPC adapters.
- [ ] `kernel/src/saturation.rs` — C owns the queue model and deterministic CPU saturation proof. Rust keeps the public config/report types and budget predicates.
- [ ] `kernel/src/scheduler.rs`
- [ ] `kernel/src/shell.rs`
- [ ] `kernel/src/syscall.rs`
- [ ] `kernel/src/task.rs`
- [ ] `kernel/src/tests.rs`
- [ ] `kernel/src/time.rs`
- [ ] `kernel/src/tlb.rs`
- [ ] `kernel/src/usb_keyboard.rs`
- [ ] `kernel/src/usb_keyboard_stub.rs`
- [ ] `kernel/src/watchdog.rs`
- [ ] `kernel/src/webauthn.rs`
- [ ] `virtual_machine/src/boot/mod.rs`
- [ ] `virtual_machine/src/clock.rs`
- [ ] `virtual_machine/src/cluster.rs`
- [ ] `virtual_machine/src/control.rs`
- [ ] `virtual_machine/src/cpu/decoder.rs`
- [ ] `virtual_machine/src/cpu/executor.rs`
- [ ] `virtual_machine/src/cpu/mod.rs`
- [ ] `virtual_machine/src/devices/apic.rs`
- [ ] `virtual_machine/src/devices/display.rs`
- [ ] `virtual_machine/src/devices/guest.rs`
- [ ] `virtual_machine/src/devices/hpet.rs`
- [ ] `virtual_machine/src/devices/input.rs`
- [ ] `virtual_machine/src/devices/interrupt_controller.rs`
- [ ] `virtual_machine/src/devices/mod.rs`
- [ ] `virtual_machine/src/devices/net/e1000.rs`
- [ ] `virtual_machine/src/devices/net/mod.rs`
- [ ] `virtual_machine/src/devices/net/virtio.rs`
- [ ] `virtual_machine/src/devices/pit.rs`
- [ ] `virtual_machine/src/devices/power.rs`
- [ ] `virtual_machine/src/devices/serial.rs`
- [ ] `virtual_machine/src/devices/storage/ahci.rs`
- [ ] `virtual_machine/src/devices/storage/disk_image.rs`
- [ ] `virtual_machine/src/devices/storage/management.rs`
- [ ] `virtual_machine/src/devices/storage/mod.rs`
- [ ] `virtual_machine/src/devices/storage/nvme.rs`
- [ ] `virtual_machine/src/devices/storage/persistence.rs`
- [ ] `virtual_machine/src/devices/storage/system_disk.rs`
- [ ] `virtual_machine/src/devices/virtio.rs`
- [ ] `virtual_machine/src/devices/virtio_queue.rs`
- [ ] `virtual_machine/src/driver_capabilities.rs`
- [ ] `virtual_machine/src/execution.rs`
- [ ] `virtual_machine/src/firmware/bios.rs`
- [ ] `virtual_machine/src/firmware/mod.rs`
- [ ] `virtual_machine/src/firmware/uefi.rs`
- [ ] `virtual_machine/src/hardware_acceleration.rs`
- [ ] `virtual_machine/src/input.rs`
- [ ] `virtual_machine/src/integration.rs`
- [ ] `virtual_machine/src/lib.rs`
- [ ] `virtual_machine/src/main.rs`
- [ ] `virtual_machine/src/memory/mod.rs`
- [ ] `virtual_machine/src/migration.rs`
- [ ] `virtual_machine/src/net/backend.rs`
- [ ] `virtual_machine/src/net/dhcp.rs`
- [ ] `virtual_machine/src/net/mac.rs`
- [ ] `virtual_machine/src/net/mod.rs`
- [ ] `virtual_machine/src/net/packet.rs`
- [ ] `virtual_machine/src/passkey_bridge.rs`
- [ ] `virtual_machine/src/replay.rs`
- [ ] `virtual_machine/src/snapshot.rs`
- [ ] `virtual_machine/src/terminal.rs`
- [ ] `virtual_machine/src/terminal_platform.rs`

Each entry names a project area containing Rust source files. Port every `.rs` file in each listed tree, including nested source, test, benchmark, example, and build-script files. These are all in-scope port targets, not optional cleanup.

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

## Rust build and workflow files to replace

- [ ] Root `Cargo.toml`, `Cargo.lock`, `.cargo/config.toml`, and `rust-toolchain.toml`.
- [ ] Every nested `Cargo.toml` and `Cargo.lock`, including manifests in the listed modules, fuzz package, and nested examples/proc-macro packages.
- [ ] Every Rust compiler/build-script dependency in project scripts, Makefiles, CI or release workflows, packaging, boot image creation, and documentation.
- [ ] Any Rust-only generated source, build output, or configuration not listed as a source module; inventory it before moving it to `Trash/`.

Inventory: 545 Rust source files and 75 Cargo manifests were found. The root
workspace lists 71 members; the other manifests include the fuzz package and
nested example/proc-macro packages. Port every file in these trees before
claiming the project has no Rust left.
