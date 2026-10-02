# C migration

This directory contains the C replacements being built for the Rust project.
The operating system and virtual machine still use Rust. The existing C boot
services use the generated syscall ABI header and shared status constants.
The C crash capsule encoder, DLM diagnostics, and DMA manager are linked into
kernel builds. C also owns the active VM xAPIC and HPET devices behind Rust
ownership and device-trait adapters. Many other foundation library modules
still need consumer integration.
See `TODO.md` for migration status.

Run `make c-library` at the repository root to build `build/c/libghostos.a`.
The library uses C11 and the standard freestanding integer, boolean, and size
types. It does not allocate memory or require a hosted C library. `CC`, `AR`,
`CPPFLAGS`, `CFLAGS`, and `BUILD_DIR` may be set for cross-compilation.

| Rust source | C implementation | Public header |
| --- | --- | --- |
| `crates/status/src/lib.rs` | `src/status.c` | `include/ghostos/status.h` |
| `crates/abi/src/generated.rs` | `src/abi.c` | `include/ghostos/abi.h` |
| `crates/api-compat/src/lib.rs` | `src/api_compat.c` | `include/ghostos/api_compat.h` |
| `crates/protocol/src/lib.rs` | `src/protocol.c` | `include/ghostos/protocol.h` |
| `crates/boot-protocol/src/lib.rs` | `src/boot_protocol.c` | `include/ghostos/boot_protocol.h` |
| `virtual_machine/src/net/mac.rs` | `src/vm_mac.c` | `include/ghostos/vm_mac.h` |
| `virtual_machine/src/net/packet.rs` | `src/vm_packet.c` | `include/ghostos/vm_packet.h` |

The VM packet queue uses the host allocator and is compiled into the VM's C
archive by `virtual_machine/build.rs`; it is not part of the freestanding kernel
foundation library.
| `kernel/src/address_space.rs` | `src/address_space.c` | `include/ghostos/address_space.h` |
| `kernel/src/allocator.rs` | `src/frame_allocator.c` | `include/ghostos/frame_allocator.h` |
| `kernel/src/arch/aarch64.rs` | `src/arch_aarch64.c` | `include/ghostos/arch_aarch64.h` |
| `kernel/src/arch/riscv64.rs` | `src/arch_riscv64.c` | `include/ghostos/arch_riscv64.h` |
| `kernel/src/arch/x86_64.rs` | `src/arch_x86_64.c` | `include/ghostos/arch_x86_64.h` |
| `kernel/src/arch/unsupported.rs` | `src/arch_unsupported.c` | `include/ghostos/arch_unsupported.h` |
| `kernel/src/arch/cpu.rs` | `src/cpu_topology.c` | `include/ghostos/cpu_topology.h` |
| `kernel/src/arch/mod.rs` | `src/arch.c` | `include/ghostos/arch.h` |
| `kernel/src/boot_diagnostics.rs` | `src/boot_diagnostics.c` | `include/ghostos/boot_diagnostics.h` |
| `kernel/src/boot_services.rs` (service registry and startup graph) | `src/boot_services.c` | `include/ghostos/boot_services.h` |
| `kernel/src/contention.rs` | `src/contention.c` | `include/ghostos/contention.h` |
| `kernel/src/cow.rs` | `src/cow.c` | `include/ghostos/cow.h` |
| `kernel/src/capability.rs` | `src/capability.c` | `include/ghostos/capability.h` |
| `kernel/src/console.rs` | `src/console.c` | `include/ghostos/console.h` |
| `kernel/src/crash.rs` (capsule encoding, build identity, one-time guard) | `src/crash.c` | `include/ghostos/crash.h` |
| `kernel/src/dlm.rs` (node/federation fences, range lock manager) | `src/dlm.c` | `include/ghostos/dlm.h` |
| `kernel/src/dma.rs` (capability-checked DMA mapping and IOMMU callbacks) | `src/dma.c` | `include/ghostos/dma.h` |
| `kernel/src/driver_capabilities.rs` | `src/driver_capabilities.c` | `include/ghostos/driver_capabilities.h` |
| `kernel/src/hot_allocator.rs` | `src/hot_allocator.c` | `include/ghostos/hot_allocator.h` |
| `kernel/src/invariants.rs` | `src/invariants.c` | `include/ghostos/invariants.h` |
| `kernel/src/ipc.rs` | `src/ipc.c` | `include/ghostos/ipc.h` |
| `kernel/src/keyboard.rs` | `src/keyboard.c` | `include/ghostos/keyboard.h` |
| `kernel/src/keyboard_stub.rs` | `src/keyboard_stub.c` | `include/ghostos/keyboard_stub.h` |
| `kernel/src/lib.rs` (C kernel API umbrella, boot flow, request-dispatch boundary, login/session state, and fatal path) | `src/kernel.c` | `include/ghostos/kernel.h` |
| `kernel/src/litmus.rs` (deterministic seeded schedules, replayable faults, and failure minimization) | `src/litmus.c` | `include/ghostos/litmus.h` |
| `kernel/src/main.rs` (boot entry and panic-handler bridge) | `src/main.c` | `include/ghostos/main.h` |
| `kernel/src/micro_silo.rs` (hardware isolation policy, tenant-only visibility, and bounded memory ranges) | `src/micro_silo.c` | `include/ghostos/micro_silo.h` |
| `kernel/src/monitor.rs` (process, CPU, DSM, and memory monitor views) | `src/monitor.c` | `include/ghostos/monitor.h` |
| `kernel/src/mouse.rs` (PS/2 packet collection and signed motion state) | `src/mouse.c` | `include/ghostos/mouse.h` |
| `kernel/src/mouse_stub.rs` (empty non-x86 mouse state) | `src/mouse_stub.c` | `include/ghostos/mouse_stub.h` |
| `kernel/src/page_fault.rs` (handler dispatch, stack growth and COW fault policy) | `src/page_fault.c` | `include/ghostos/page_fault.h` |
| `kernel/src/partition.rs` (CPU online, isolation, and housekeeping policy) | `src/partition.c` | `include/ghostos/partition.h` |
| `kernel/src/pci.rs` (x86 PCI bus scan and bounded inventory capture) | `src/pci.c` | `include/ghostos/pci.h` |
| `kernel/src/persistence.rs` (bounded port I/O and checksummed boot/crash record container) | `src/persistence.c` | `include/ghostos/persistence.h` |
| `kernel/src/persona.rs` (fixed-capacity active and disabled execution rights) | `src/persona.c` | `include/ghostos/persona.h` |
| `kernel/src/physical_storage.rs` (AHCI candidate selection and expected-volume policy) | `src/physical_storage.c` | `include/ghostos/physical_storage.h` |
| `kernel/src/power.rs` (ACPI memory/register access and VM power fallbacks) | `src/power.c` | `include/ghostos/power.h` |
| `kernel/src/process.rs` (bounded process table, identity generations, lifecycle status and transitions) | `src/process.c` | `include/ghostos/process.h` |
| `kernel/src/quota.rs` (sharded token buckets and contention reports) | `src/quota.c` | `include/ghostos/quota.h` |
| `kernel/src/random.rs` (processor-seeded ChaCha20 kernel random source) | `src/random.c` | `include/ghostos/random.h` |
| `kernel/src/runtime.rs` (runtime dispatch state, ABI and filesystem request validation, and response validation) | `src/runtime.c` | `include/ghostos/runtime.h` |
| `kernel/src/saturation.rs` (bounded housekeeping saturation proof and progress report) | `src/saturation.c` | `include/ghostos/saturation.h` |
| `kernel/src/scheduler.rs` (ready-thread selection, realtime ordering, and IPC priority inheritance) | `src/scheduler.c` | `include/ghostos/scheduler.h` |
| `kernel/src/shell.rs` (VT input decoding and command-line expansion) | `src/shell.c` | `include/ghostos/shell.h` |
| `kernel/src/syscall.rs` (user-range/request validation and scheduler sleep hints) | `src/syscall.c` | `include/ghostos/syscall.h` |
| `kernel/src/task.rs` (CPU masks, address-space IDs, and generation-tagged thread IDs) | `src/task.c` | `include/ghostos/task.h` |
| `kernel/src/time.rs` (atomic kernel clock and x86 RTC conversion) | `src/time.c` | `include/ghostos/time.h` |
| `kernel/src/tlb.rs` (bounded TLB shootdown tracking and acknowledgement policy) | `src/tlb.c` | `include/ghostos/tlb.h` |
| `kernel/src/usb_keyboard.rs` (PCI discovery and boot adapter; xHCI control, HID report decoding, terminal key sequences, and interface descriptor selection) | `src/usb_keyboard.c`, `src/usb_keyboard_controller.c` | `include/ghostos/usb_keyboard.h` |
| `kernel/src/usb_keyboard_stub.rs` (non-x86 empty USB keyboard adapter) | `src/usb_keyboard_stub.c` | `include/ghostos/usb_keyboard_stub.h` |
| `kernel/src/watchdog.rs` (service and CPU liveness tracking with one-shot stale reports) | `src/watchdog.c` | `include/ghostos/watchdog.h` |
| `kernel/src/webauthn.rs` (SHA-256 primitive used by local WebAuthn verification) | `src/webauthn.c` | `include/ghostos/webauthn.h` |
| `virtual_machine/src/boot/mod.rs` (Multiboot header discovery and information decoding) | `src/vm_boot.c` | `include/ghostos/vm_boot.h` |
| `virtual_machine/src/clock.rs` (host monotonic source and manual clock arithmetic) | `src/vm_clock.c` | `include/ghostos/vm_clock.h` |
| `virtual_machine/src/cluster.rs` (bounded deterministic network, shared-memory fixture, membership, heartbeats, faults, and scale evidence) | `src/vm_cluster.c` | `include/ghostos/vm_cluster.h` |
| `virtual_machine/src/devices/apic.rs` (active xAPIC state, interrupts, IPI routing, and timer) | `src/vm_apic.c` | `include/ghostos/vm_apic.h` |
| `virtual_machine/src/devices/hpet.rs` (active HPET state, comparator scheduling, and IRQ routing) | `src/vm_hpet.c` | `include/ghostos/vm_hpet.h` |

`make c-vm-test-binaries` builds `build/c/vm-device-contracts` without executing
it. This standalone C binary checks APIC priority, level-triggered delivery,
masking, NMI routing, countdown timers, shared manual-clock behavior, and HPET
interrupt delivery into the C APIC. It does not boot an OS or replace the VM
executable. The active VM still requires Cargo and the remaining Rust modules.

The APIC and HPET structures are native C/Rust ABI state, not disk formats.
VM adapters use `repr(C)` and check the C structure size before initialization.
APIC snapshots retain the existing field order and optional host timestamp;
restoring a snapshot re-seeds the host clock as before. The device ports preserve
the existing VM register decoding and timing semantics, including their current
limitations; these checks do not establish hardware-spec conformance.

The ABI files are generated from `abi/ghostos-abi.toml`. Run
`python3 tools/generate_abi.py` to regenerate C, Rust, and Swift bindings together,
or `make generate-c-abi` to regenerate only C.

The syscall request and response keep their `repr(C)` layouts. The boot handoff
keeps its 16-byte alignment and native pointer-size region count. Compile-time
assertions enforce the structure sizes and offsets. RPC frames are encoded field
by field in network byte order, including the two reserved zero bytes. Do not
send a C RPC header structure directly over a transport.

The AArch64 kernel implementation is enabled for AArch64 ELF builds. The host
build keeps inert architecture stubs so the portable foundation library builds
on development machines. Exception callbacks must be installed before enabling
the AArch64 vector table.

The shared architecture wrapper in `src/arch.c` selects evidence at compile
time and calls an explicit backend table for boot, interrupts, TLB invalidation,
user access, and user entry. Kernel integration supplies the validation and
backend callbacks.

The x86 interrupt entry calls a registered dispatcher with the saved frame.
Kernel scheduling and page-fault policy must be supplied by that
dispatcher when the C kernel is connected.

The C boot service registry preserves all 13 service descriptors and their
restart settings. Its startup planner uses stable, lowest-ID topological order,
invokes a kernel launch callback, and records per-service startup diagnostics.
The boot state owns a filesystem context through explicit adopt/release hooks,
provides transactional first-admin provisioning and recovery, persists passkey
records and monotonic sign counters, and enforces per-session shell filesystem
rights through the filesystem daemon callbacks. The active kernel consumer is
still Rust; it must be ported before this C boot runtime replaces the running
system boot path.

`userspace/boot-services/service.c`, `login.c`, and `shell.c` now use the generated
request/response types and operation IDs instead of private copies. The kernel
build adds the shared include directory and rebuilds the C service images when
the headers change. These consumers use header definitions and need no C library
link yet. The kernel currently builds the C service/login images and the Rust
Ring 3 shell; the C shell source is not the active shell image.

Other Rust structures did not promise a C layout. Their C replacements use
explicit tags for optional/error values, and split 128-bit correlation IDs into
low and high 64-bit words. These structures are not serialized representations.
All status codes, facility values, messages, and compatibility identifiers keep
their existing values. The legacy Rust API contract identifier stays stable.

C functions replace methods with `ghostos_*` names. Raw status values are the
`uint32_t` status value itself. For `IntoStatus`/`IntoPublicError`, callers convert
their error to a status and call `ghostos_status_public_error`. Protocol guard
fields expose the former read-only accessors; callers must not mutate them
outside the guard functions. Callers serialize mutable access and provide valid
non-null object pointers and suitably aligned storage. Raw integer enum values
must pass their validation functions before use. C explicitly rejects invalid
integer discriminants that Rust's type system made impossible to construct.

`tests/foundation.c` ports the foundation contract cases, including the bounded
generated status and boot-region cases. `make c-test-binaries` builds the test
executable without running it. The generated-input cases now use the shared
property harness in `test_support/property.c`, declared in
`include/ghostos/test_property.h`. Its case seeds, xorshift entropy, little-endian
byte generation, and status/boot input draws match the Rust implementation.

`tests/kernel_contracts.c`, `tests/kernel_frame_contracts.c`, and
`tests/kernel_ipc_contracts.c` port direct C-backed kernel contract cases from
`kernel/src/tests.rs`. `make c-test-binaries` compiles these executables; it does
not run them.

Run `make c-test-support` to build `build/c/libghostos-test-support.a`. This is a
separate hosted library for tests. It uses allocation, environment variables,
and string formatting; it is not part of the freestanding production library.
Other test-support fixtures, crash helpers, and differential helpers still need
their own ports.

The property harness supports `run` callbacks returning an error message and
`run_assert` predicates returning a boolean. It stops at the first failure and
reports the property name, seed, case index, case seed, and replay command.
Failure strings are copied and must be released with
`ghostos_property_failure_dispose`. Byte/ASCII generators return heap-owned
outputs. Queue and capability model storage has explicit dispose and clone
functions; lease models can be copied by value.

`ghostos_property_config_from_env` uses the Rust defaults: seed
`0x53594e4f535f5445` and 256 cases. Environment controls are
`GHOSTOS_PROPERTY_SEED` (decimal or `0x`/`0X` hexadecimal),
`GHOSTOS_PROPERTY_CASES` (positive decimal), and `GHOSTOS_PROPERTY_CASE`
(a decimal case index). Invalid controls retain the defaults. A replay runs
exactly one indexed case, with the same case seed as a full run.

The C foundation cases use `ghostos_property_config_with_env` to keep their
original seed `0x593` and case counts (256 for status, 128 for boot regions)
unless environment controls override them. Explicit `config_new` and
`config_replay` constructors do not read the environment, matching Rust.
For example, setting `GHOSTOS_PROPERTY_SEED=0x593 GHOSTOS_PROPERTY_CASE=17`
selects case 17 for each generated-input property when the foundation binary
is executed.

No Rust source or build configuration is obsolete yet. Move it to `Trash/` only
after the C consumers replace it and behavior preservation is established. The
new `build/c/` output belongs to the active C build.
