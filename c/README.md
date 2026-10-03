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
types. Foundation modules do not allocate memory or require a hosted C library.
Host VM modules use the host allocator and console callbacks. `CC`, `AR`,
`CPPFLAGS`, `CFLAGS`, and `BUILD_DIR` may be set for cross-compilation.

| Rust source | C implementation | Public header |
| --- | --- | --- |
| `crates/policy/src/lib.rs` (active snapshot storage, fingerprints, and all read-only simulations) | `src/policy.c` | `include/ghostos/policy.h` |
| `crates/numa/src/lib.rs` (active topology construction, placement, cursors, and locality counters) | `src/numa.c` | `include/ghostos/numa.h` |
| `crates/platform-io/src/lib.rs` (active queue metadata, token/state transitions, and I/O/media validation; generic payloads stay caller-owned) | `src/platform_io.c` | `include/ghostos/platform_io.h` |
| `crates/durability/src/lib.rs` (active bounded trace recording, ordering/recovery verification, and C layer contracts) | `src/durability.c` | `include/ghostos/durability.h` |
| `crates/status/src/lib.rs` | `src/status.c` | `include/ghostos/status.h` |
| `crates/abi/src/generated.rs` | `src/abi.c` | `include/ghostos/abi.h` |
| `crates/api-compat/src/lib.rs` (active version-check and migration policy, plus C contracts/formatting) | `src/api_compat.c` | `include/ghostos/api_compat.h` |
| `crates/protocol/src/lib.rs` (active transport guard state and policy behind Rust adapters) | `src/protocol.c` | `include/ghostos/protocol.h` |
| `crates/boot-protocol/src/lib.rs` | `src/boot_protocol.c` | `include/ghostos/boot_protocol.h` |
| `virtual_machine/src/replay.rs` (active session state, owned events, replay policy, DMA and file/header codecs) | `src/vm_replay.c` | `include/ghostos/vm_replay.h` |
| `virtual_machine/src/devices/display.rs` (active VGA/VESA state, rendering, snapshots, PPM, and BIOS video dispatch) | `src/vm_display.c` | `include/ghostos/vm_display.h` |
| `virtual_machine/src/devices/net/e1000.rs` (active registers, RX queue, descriptor processing, errors, and IRQ decisions) | `src/vm_e1000.c` | `include/ghostos/vm_e1000.h` |
| `virtual_machine/src/devices/storage/nvme.rs` (active registers, admin/I/O queues, commands, identification, DMA, and completions) | `src/vm_nvme.c` | `include/ghostos/vm_nvme.h`, `include/ghostos/vm_storage_io.h` |
| `virtual_machine/src/devices/storage/ahci.rs` (active registers, ATA commands, identification, PRD DMA, FIS, and IRQ decisions) | `src/vm_ahci.c` | `include/ghostos/vm_ahci.h`, `include/ghostos/vm_storage_io.h` |
| `virtual_machine/src/net/backend.rs` (active shared-segment/loopback queues, delivery, port state, limits, loss, and counters) | `src/vm_segment.c` | `include/ghostos/vm_segment.h` |
| `virtual_machine/src/net/backend.rs` (active host-backend admin/promiscuous state, packet framing, receive filtering, and counters) | `src/vm_host_net.c` | `include/ghostos/vm_host_net.h` |
| `virtual_machine/src/hardware_acceleration.rs` (active backend negotiation, probing policy, KVM ioctl, fallback reporting, and status formatting) | `src/vm_acceleration.c` | `include/ghostos/vm_acceleration.h` |
| `virtual_machine/src/terminal.rs` (active input policy, resize timing, transcripts, EOF, and counters) | `src/vm_terminal.c` | `include/ghostos/vm_terminal.h` |
| `virtual_machine/src/terminal_platform.rs` (Unix/Windows terminal modes, restoration, signal guards, and size queries) | `src/vm_terminal_platform.c` | `include/ghostos/vm_terminal_platform.h` |
| `virtual_machine/src/firmware/bios.rs` (ROM, POST tables, disk/memory/keyboard services) | `src/vm_bios.c` | `include/ghostos/vm_bios.h` |
| `virtual_machine/src/migration.rs` (frame decoding, tag assembly, validation sequencing) | `src/vm_migration.c` | `include/ghostos/vm_migration.h` |
| `virtual_machine/src/snapshot.rs` (SHA-256, multipart HMAC, authentication comparisons) | `src/vm_snapshot_auth.c` | `include/ghostos/vm_snapshot_auth.h` |
| `virtual_machine/src/net/mac.rs` | `src/vm_mac.c` | `include/ghostos/vm_mac.h` |
| `virtual_machine/src/net/packet.rs` | `src/vm_packet.c` | `include/ghostos/vm_packet.h` |
| `virtual_machine/src/devices/power.rs` | `src/vm_power.c` | `include/ghostos/vm_power.h` |
| `virtual_machine/src/devices/interrupt_controller.rs` | `src/vm_interrupt_controller.c` | `include/ghostos/vm_interrupt_controller.h` |
| `virtual_machine/src/devices/virtio.rs` | `src/vm_virtio.c` | `include/ghostos/vm_virtio.h` |
| `virtual_machine/src/devices/net/virtio.rs` | `src/vm_virtio_net.c` | `include/ghostos/vm_virtio_net.h` |
| `virtual_machine/src/devices/virtio_queue.rs` | `src/vm_virtio_queue.c` | `include/ghostos/vm_virtio_queue.h` |
| `virtual_machine/src/net/backend.rs` (packet validation, UDP host-frame encoding/decoding, and deterministic segment recipient filtering), `src/net/dhcp.rs` (IPv4 integer conversion and Internet checksum), and `src/net/mod.rs` (`align_up` helper) | `src/vm_net.c` | `include/ghostos/vm_net.h` |
| `virtual_machine/src/net/dhcp.rs` (configuration validation, lease policy/storage, request parsing, reply encoding, and bounded DHCP options) | `src/vm_dhcp.c` | `include/ghostos/vm_dhcp.h` |

The VM packet queue uses the host allocator and is compiled into the VM's C
archive by `virtual_machine/build.rs` and the root C archive. Kernel consumers
use only the freestanding foundation objects.
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
| `crates/ras/` (active hardware telemetry, poison quarantine/admission, and budget/workload policy) | `src/ras.c` | `include/ghostos/ras.h` |
| `crates/service-scale/` (membership decisions, target selection, snapshot digest) | `src/service_scale.c` | `include/ghostos/service_scale.h` |
| `crates/posix-compat/` (descriptor metadata and pseudo-path parsing) | `src/posix_compat.c` | `include/ghostos/posix_compat.h` |
| `crates/power/` (thermal/event queues, cluster selection, frequency, idle, device sleep decisions) | `src/thermal.c`, `src/power_policy.c` | `include/ghostos/thermal.h`, `include/ghostos/power_policy.h` |
| `kernel/src/dma.rs` (active caller-owned mapping tables, staged allocation, capability-checked DMA mapping and IOMMU callbacks) | `src/dma.c`, `src/dma_state.c` | `include/ghostos/dma.h` |
| `kernel/src/driver_capabilities.rs` | `src/driver_capabilities.c`, `src/driver_resources.c` | `include/ghostos/driver_capabilities.h` |
| `kernel/src/hot_allocator.rs` | `src/hot_allocator.c` | `include/ghostos/hot_allocator.h` |
| `kernel/src/invariants.rs` (active address-space/page-table checks and failure formatting; const APIs/panic policy stay in Rust) | `src/invariants.c` | `include/ghostos/invariants.h` |
| `kernel/src/ipc.rs` | `src/ipc.c` | `include/ghostos/ipc.h` |
| `kernel/src/keyboard.rs` (active x86 controller, port I/O, decoding, and boot singleton) | `src/keyboard.c` | `include/ghostos/keyboard.h` |
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
| `kernel/src/persistence.rs` (bounded port I/O and checksummed boot/crash record container) | `src/persistence.c`, `src/persistence_records.c` | `include/ghostos/persistence.h` |
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
| `virtual_machine/src/clock.rs` (host/manual clock state and time arithmetic) | `src/vm_clock.c` | `include/ghostos/vm_clock.h` |
| `virtual_machine/src/cluster.rs` (bounded deterministic network, shared-memory fixture, membership, heartbeats, faults, and scale evidence) | `src/vm_cluster.c` | `include/ghostos/vm_cluster.h` |
| `virtual_machine/src/devices/apic.rs` (active xAPIC state, interrupts, IPI routing, and timer) | `src/vm_apic.c` | `include/ghostos/vm_apic.h` |
| `virtual_machine/src/devices/hpet.rs` (active HPET state, comparator scheduling, and IRQ routing) | `src/vm_hpet.c` | `include/ghostos/vm_hpet.h` |
| `virtual_machine/src/devices/pit.rs` (active counters, port protocols, and timer pulses; Rust routes APIC delivery) | `src/vm_pit.c` | `include/ghostos/vm_pit.h` |
| `virtual_machine/src/input.rs` (resize encoding/filtering and ASCII to PS/2 conversion) | `src/vm_input.c` | `include/ghostos/vm_input.h` |
| `virtual_machine/src/devices/input.rs` (active PS/2 commands, output/pending queues, mouse packets, and IRQ decisions) | `src/vm_ps2.c` | `include/ghostos/vm_ps2.h` |
| `virtual_machine/src/devices/guest.rs` (active mailbox queues/registers, hotplug state, and pvclock state/encoding) | `src/vm_guest.c` | `include/ghostos/vm_guest.h` |
| `virtual_machine/src/driver_capabilities.rs` (active availability/fallback decisions and report text) | `src/vm_driver_capabilities.c` | `include/ghostos/vm_driver_capabilities.h` |
| `virtual_machine/src/devices/serial.rs` (active UART, input/output queues, authentication prompts, and spinner policy) | `src/vm_serial.c` | `include/ghostos/vm_serial.h` |
| `virtual_machine/src/devices/storage/persistence.rs` (active filesystem persistence ports and tail-region format) | `src/vm_persistence.c` | `include/ghostos/vm_persistence.h` |

`make c-vm-test-binaries` builds `build/c/vm-device-contracts` without executing
it. This standalone C binary checks APIC priority, level-triggered delivery,
masking, NMI routing, countdown timers, shared manual-clock behavior, and HPET
interrupt delivery into the C APIC. It also contains the ported PIT, PS/2, and
driver-report cases; the newly added PS/2 and driver cases have not been executed.
It does not boot an OS or replace the VM
executable. The active VM still requires Cargo and the remaining Rust modules.

The APIC and HPET structures are native C/Rust ABI state, not disk formats.
VM adapters use `repr(C)` and check the C structure size before initialization.
APIC snapshots retain the existing field order and optional host timestamp;
restoring a snapshot re-seeds the host clock as before. The device ports preserve
the existing VM register decoding and timing semantics, including their current
limitations; these checks do not establish hardware-spec conformance.

The PS/2 controller and guest mailbox use host allocation for pending queues.
PS/2 direct injection keeps the original drop-oldest 64-byte output policy;
lossless injection retains excess keyboard bytes until guest data-port reads
make room. Guest mailbox queues retain their existing unbounded host-memory
policy. Allocation failures surface through the Rust adapters rather than
silently discarding lossless input. C IRQ callbacks run synchronously; the
PS/2 adapter preserves the existing retry when shared APIC borrowing is busy.
The pvclock invokes the host/replay wall-time callback after its two system-time
page writes, then writes the wall-clock page in the original little-endian
format. Device reset semantics and guest-visible errors remain unchanged.

The serial controller also owns its host-output buffer and authentication
presentation in C. Console writes, flushes, and monotonic timestamps are
synchronous host callbacks. The active Rust adapter connects stdout and the
shared APIC; C decides receive IRQs and the 80 ms authentication spinner policy.
Raw guest output and translated host output remain separate buffers. The
fourteen retained Rust serial cases have C equivalents in
`c/tests/vm_serial_contracts.c`; `make c-vm-test-binaries` also builds that
standalone binary. These newly ported serial checks have not been executed.

The VM persistence port retains its 64 KiB tail region, 512-byte header sector,
60 KiB payload bound, SYNOPS01 magic, version 1, little-endian fields, and FNV-1a
checksum. The C implementation performs sector I/O and final sync through the
DiskImage adapter's callbacks. Invalid magic/version/length/checksum loads an
empty state; a failed read/write/sync preserves the existing command-transition
behavior. A checked C result represents the original out-of-bounds word-read
slice panic, which the Rust adapter still raises without a C buffer overread.

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

`vm_replay.h` / `vm_replay.c` own the active ordered replay session and its
payload storage, including recording, capacity checks, mode/cursor transitions,
first-divergence reports, host-input resize validation, and authoritative DMA
replay. SYNVMRP1 codecs retain little-endian layout and reserved-byte acceptance.
Rejected replay imports leave the existing session intact. Kind mismatches do
not consume an event; input mismatches consume it before reporting failure.
DMA encode/decode errors retain the previous error latch, matching Rust.
Session event views remain borrowed until reset/import/destruction; Rust copies
them into public typed trace snapshots. Filesystem I/O remains in the adapter.

The display port owns text, framebuffer, palette, cursor, and rendered pixel
storage. Pixels borrow C memory until the next mutation or destruction. It also
encodes legacy byte-as-character text snapshots as UTF-8 and P6 PPM output.
The BIOS video port retains the current LFB mode mask, 15-bit pitch arithmetic,
and text-scroll fill sequencing. Valid VBE mode-info requests still report the
legacy four-byte-to-two-byte copy panic through the Rust adapter; C never copies
past a buffer. Shared ownership, MMU callbacks, GOP types, and file writes remain
in Rust. The two existing display cases have C source in
`c/tests/vm_device_contracts.c`, checked for syntax only.

The e1000 port retains its 128-packet/3036-byte pending queue, bounded 4096-entry
rings, TX head traversal, RX tail-plus-one indexing, interrupt masks, and ignored
status-write failures. It shares the virtio-net host callback layout while
retaining e1000-specific behavior, including carrier-up without a backend.
Rust supplies synchronous MMU, backend, and APIC callbacks. The two existing
e1000 cases have C source in `c/tests/vm_io_contracts.c`.

Terminal C state owns input translation across chunks, the 250 ms size-poll
policy, EOF Ctrl-D bytes, poll buffers, transcript storage, and counters. Rust
supplies input threads, streams, clocks, and typed failure/trace views. Native
terminal C code saves and restores Unix termios, installs the original twelve
signal handlers, and permits one raw session. Windows code preserves console
mode setup, failed-output rollback, and output-before-input restoration.
Numeric native errors are translated by Rust without terminal data in messages.
Only the AArch64 macOS host build has been checked; Linux/Windows and PTY parity
remain unverified. Policy portions of four retained terminal cases also have C
source in `c/tests/vm_io_contracts.c`, checked for syntax only. The explicit
`make c-vm-test-binaries` target includes `build/c/vm-io-contracts` without
executing it.

The shared path matcher is in `path_pattern.h` / `path_pattern.c`. It is
freestanding and allocation-free, and ports `crates/path-pattern/src/lib.rs`
parsing, validation errors, classes, escapes, and UTF-8-width wildcard matching.
The Rust API borrows strings and calls C for all active matching operations;
filesystem, shell, and runtime callers retain their existing interface.
`c/tests/path_pattern_contracts.c` contains the seven original contract and
property cases using the C property harness. Its explicit Makefile target
builds the cases without executing them. Behavior parity remains unverified.

`vm_disk_management.h` / `vm_disk_management.c` own active VM disk-spec
validation, format/capacity policy, controller and guest identity strings,
clone selection and retry/copy/cleanup sequencing, and sorted attachment
bookkeeping. The manager retains opaque host payloads and immutable ID slices;
remove transfers payload ownership to the caller, and free invokes a supplied
destructor for every remaining payload. Rust boxes keep the current DiskInfo
reference API stable. Canonical paths, Unicode whitespace checks, native file
operations, clone-name formatting, clocks, and image lifetime use host adapters.
The two original disk-management cases have policy portions in
`c/tests/vm_disk_management_contracts.c`; filesystem ownership/cleanup fixtures
and behavior parity remain unverified.

The active disk-image adapter also delegates fixed-VHD repair eligibility,
checksum recomputation/write/sync, and exact lowercase format-name parsing to
`vm_disk_image.c`. It retains the historical 32-bit size fields at footer
offsets 36 and 40, checks type and original/current agreement before capacity,
and distinguishes invalid capacity errors from an ineligible footer. The host
still owns lock publication/recovery, image lifetime, and directory syncing.

The freestanding `admission.h` / `admission.c` controller ports
`crates/admission/src/lib.rs`. C owns fixed-capacity admission and tenant state,
recovery budgets, hierarchy validation/charging, lease allocation/completion,
load shedding, saturating counters/sequences, and reports. The caller supplies
controller and lease-slot storage; C retains no pointers and allocates no
memory. Each mutation uses the same slot array/capacity as initialization.
Class/priority values use the documented byte discriminants. Invalid policy
and lease errors retain their separate codes; admit/retry reject invalid enum
inputs before indexing state. Backup, inspection, package, and storage callers
use the existing Rust API through `crates/admission/src/native.rs`. Const enum
names/policy constructors and public Rust types remain compatibility adapters.
The three original cases are in `c/tests/admission_contracts.c`; the explicit
Makefile target only builds them. Behavior parity remains unverified.

`time_sync.h` / `time_sync.c` own the active time-sync runtime: atomic manual
clocks, timestamp conversion, SPTP wire codecs, master/slave exchanges, pending
state, clock discipline and monotonic correction, and epoch fencing. Zero
initialization creates a cluster clock; daemon/epoch constructors validate node
IDs. All state is caller-owned and movable, with no allocations or retained
pointers. Portable two-word signed-magnitude arithmetic preserves the original
Rust i128 calculation range and truncation rules without compiler extensions.
Encoding retains direct-timestamp truncation/acceptance, while decoding checks
nanosecond bounds. Delay-response validation consumes pending state at the
same point as the Rust implementation, including on timestamp overflow.
Eight consumer crates use the public Rust API through allocation-free adapters;
const constructors/getters, traits, tagged-message/status translation, and Rust
build tooling remain. All five original cases have C source in
`c/tests/time_sync_contracts.c`, checked for syntax only. Behavior parity and
non-host platform builds remain unverified.

`vm_control.h` / `vm_control.c` own the active VM monitor protocol: bounded
request buffering/framing, strict UTF-8 checks, Unicode-aware command and
permission parsing, authentication sequencing, timestamp/hex validation,
FIFO replay nonces, permissions, JSON escaping, response-size policy, and
help/action/error/envelope encoding. Zero initialization creates a request
buffer; the nonce cache has an explicit allocator/free lifecycle. C callbacks
borrow the host clock and authentication key only during a request. Nonces
are remembered only after HMAC, command parsing, and both permission checks
succeed. HMAC still uses the original domain, little-endian timestamp, nonce,
and trimmed command, with the existing C constant-time tag verification.
Rust retains VM diagnostic data/redaction assembly, sockets/execution, host
clock/key callbacks, owned strings/paths/errors, and const command APIs.
The first five existing cases are in `c/tests/vm_control_contracts.c`; two VM
inspection/redaction fixtures and behavior parity remain open.

`vm_passkey.h` / `vm_passkey.c` own active passkey bridge protocol helpers:
bounded HTTP parsing, header/token/body offsets, Unicode whitespace, query
lookup, percent/hex decoding, username policy, CR and length-prefixed serial
frames, banner/challenge selection, observed guest-state transitions, and
prompt-driven input readiness. Frame functions support a null-output size query
before allocation. HTTP parsing retains the historical checked-add and wrapped
slice failure decisions; native adapters preserve their panic behavior.
`vm_passkey_assets.c` stores the original HTML, CSS, and browser script as
immutable byte arrays, verified byte-for-byte against their original source.
Rust still owns sockets/timeouts, browser launch, token entropy, request
routing/replies, the icon asset reference, VM/serial handoff, and owned buffers.
The original localhost-bind test remains Rust and unrun; behavior parity and
non-host platform coverage remain open.

`vm_execution.h` / `vm_execution.c` own active decoded-block translation and
execution loops, the instruction-boundary list, wrapping backward-loop targets,
source-range validation, code/translation version observation, saturating hot
execution counts and promotion decisions, and 64-sample retune timing. Callbacks
borrow host state only during a dispatch, without retaining those context
pointers. Separate native cache/profile owners have explicit allocation,
clear, and free lifecycles. Cache payload ownership transfers on successful
insertion and transfers back on removal/eviction; a Rust destructor callback
releases retained decoded blocks on replacement, clear, and free. Context keys
include RIP, mode, privilege, and CR3. Growing chained hash tables preserve
payload/profile addresses; FIFO keys remain separate to preserve stale keys
across reset and repeated insertion keys. Capacity zero means one entry,
eviction skips stale keys and precedes admission, and stale rejection removes
all matching FIFO keys. Profile/instruction-count namespaces are separate;
profile default-start behavior and ordered checked/wrapping counter updates
match the original source.
Translation preserves first-error versus partial-block behavior and final
wrapping source length. Execution preserves code-validation-before-translation
ordering, mode/boundary exits, zero-progress cache clearing, and execution-error
notification suppression. Rust callbacks still own CPU/MMU/device operations,
replay cleanup and interrupt-shadow restoration, decoded instructions/errors,
cache-policy admission/events, public types, profile hooks, and error adapters.
The callback ABI permits unwinding so existing Rust overflow/allocation panics can
propagate across the C dispatcher. Host builds pass; no tests were run, and
behavior parity, non-host coverage, and full Rust removal remain open.

`vm_uefi.h` / `vm_uefi.c` own active PE32+ parsing/section validation, guest
image mapping and zero-fill, relocation traversal, stable memory-map encoding,
GetMemoryMap/ExitBootServices status and map-key policy, boot-service
classification, and service-stub/table-header/RSDP encoding. PE parsing returns
borrowed raw-section offsets; Rust adapters copy them into owned decoded-image
buffers. Zero-length sections retain unchecked raw pointers. Relocations use
virtual value reads/writes (failed reads become zero, failed writes are ignored)
while image mapping and descriptor writes use physical guest writes. Unknown
relocation types, odd/trailing relocation bytes, modulo-width deltas, and
checked-debug/wrapping-release address calculations retain existing behavior.
Memory-map ordering is stable for equal image bases and saturates image ends;
firmware reserves and descriptor virtual/physical addresses match the original.
GetMemoryMap preserves pointer/status order, partial descriptor writes on error,
ignored metadata-write errors, and native-width wrapping map keys. RSDP bytes
retain the existing legacy field offsets. Native functions borrow all buffers
and MMU callbacks synchronously and allocate nothing. Rust still owns context,
images, guest callbacks, table assembly, CPU handoff, console/protocol/runtime
services, profile-independent replay/clock access, and error adapters. Host
library/VM builds pass; no tests were run, and platform coverage, behavior
parity, and complete Rust removal remain open.

The shared policy engine stores fixed-capacity C records in caller-owned arrays;
its borrowed snapshot views are retained only for the duration of a call.
Simulation preserves the original read-only epoch/fingerprint contract and
bounded, ordered affected lists. Rust adapters convert public records/reports
and preserve the const constructors and debug views.

Admission and time-sync use `ghostos/memory.h` for freestanding zero/copy/equality
operations. Their C sources no longer require hosted string headers.
The remaining Rust adapter build scripts use `tools/c_archive.rs` to select
archive format and translate the bare-metal RV64GC target spelling for Clang.
Explicit `AR`/`CLANG` overrides remain available. The macOS Apple Clang frontend
supports RISC-V syntax/IR but lacks its machine-code backend; target verification
used a temporary LLVM `llc` wrapper. A normal RISC-V build needs a compiler with
that backend. These adapters/build workflows still await complete C cutover.

The active micro-silo adapter uses caller-owned C range tables with arbitrary
capacity, including zero. Hardware policy, overlap checks, borrowed cleanup,
and lookup run in C. The adapter retains public const operations and converts
ordered overflow reports into Rust debug panics. Standalone bounded silo APIs
remain available with their existing validation rules.

Active kernel DMA callers use `dma_state.c` with caller-owned record arrays.
Preparation validates and selects a mapping without mutation; the adapter
commits after Rust capability checks and IOMMU approval. IOMMU calls remain
outside the C stack, preserving host unwind behavior. Generic capacities are
not capped by the standalone C manager's 256 slots. CPU partition queries and
transitions also use C in host builds, including VM consumers.

Active hot allocator consumers use borrowed C views over their generic
kind/node/CPU pool arrays. Fixed-capacity standalone callers use the same C
allocation, reclaim, and reporting core with explicit array strides. Rust
retains ownership-token types, const APIs, remote-memory trace emission, and
conversion of checked arithmetic failures into host-compatible panics.

`driver_resources.c` owns PCI role/BAR matching, range validation, MMIO
placement, image-overlap checks, and resource manifest appends. Kernel
adapters retain capability minting so native error details and partial-grant
side effects remain intact. Standalone grants share this selection policy.

Active litmus callers use C schedule generation, models, replay, and
minimization. The checked replay API handles malformed public schedule lengths
in execution order; Rust adapts reports and bounds panic results. Public const
values and existing Rust contract source remain until final migration.

Kernel persistence container consumers use `persistence_records.c` for
SYNREC01 serialization, payload checksums, legacy raw-record loading, slot
updates, and boot extraction. The hardware port remains in `persistence.c`.
Both the record ABI and boot/crash size limits have compile-time checks.

Active boot diagnostics use C attempt transitions and the 56-byte diagnostic
codec. The raw encoder preserves public Rust records with arbitrary raw status
words, while decode retains strict status/checksum/reserved-byte validation.
Rust keeps const definitions, typed translation, and persistence orchestration.

RAS consumers use caller-owned C event and poison tables with generic
capacities. C owns ring placement, sequence IDs, saturating counters, node-scoped
overlap validation, and quarantine insertion. Rust retains typed const APIs,
trace emission, and debug overflow panic conversion. Budget arbitration,
controller effects, and persistent-pool recovery remain Rust.

RAS budget consumers also use C for signed saturated predictions, threshold
selection, workload tables, eviction selection, and post-approval commits.
Rust retains controller calls and last-reading storage. Throttling precedes
eviction, and each successful eviction commits separately, preserving partial
failure and host unwind behavior. Public policy validation remains const Rust.

RAS persistent pools now use C dirty-page tables, flush transitions, clean-slot
cleanup, recovery reset, and generation rollover. Rust retains backend/journal
calls and marker conversion. The port fixes the prior Rust copy-mutation bug
that kept flushed pages Dirty: C updates the stored slot before and after the
backend call. Barrier and journal ordering remain explicit in the adapter.
C also decides whether AER requires reporting and segment isolation.
The existing three RAS fixtures plus a flush-state regression are available in
`c/tests/ras_contracts.c`; the optional `ras-contracts` target builds them.

POSIX compatibility consumers use `posix_compat.c` for caller-owned descriptor
metadata and pseudo-path parsing. Rust keeps opaque runtime file ownership,
service calls, and typed APIs. The crate build script links the same C source
as the root archive. Close removes metadata only after the runtime succeeds.
Pseudo-path parsing preserves invalid-component and length-error ordering.

Power thermal consumers link `thermal.c` through `crates/power/build.rs`.
The native decision function preserves trip priority, saturated hysteresis,
and event classification. Native queue metadata supports arbitrary capacities,
zero capacity, oldest-first partial drains, and saturating drop counts. Rust
retains typed event storage and sensor/actuator traits; C/Rust layout assertions
protect the shared trip and queue state.

Power policy consumers also link `power_policy.c`. Caller-built checked-layout
cluster views preserve eligibility, preferred-cluster fallback, and first-best
tie selection. Debug overflow is returned to the Rust panic adapter; release
scores wrap. Frequency calculations saturate each original intermediate.
Idle and device sleep decisions preserve boundary comparisons and saturated
time differences. Rust owns configuration/state and invokes hardware traits.

Service scaling consumers link `service_scale.c` through the crate build script.
Checked-layout instance views preserve membership error ordering and generation
fences. The native target selector preserves first-best ties, and digest uses
unsigned wrapping FNV arithmetic. Typed records, commits, session handoffs,
request/effect ledgers, counters, and public APIs remain in Rust. Temporary
instance views use stack space proportional to generic instance capacity.

Service scaling also calls C for session-close and handoff-preparation checks
and checked-layout token field matching. In-flight error precedence and lazy
snapshot digest/slice evaluation stay intact. Rust retains typed commits,
copying, counters, and request/effect transitions.

Service scaling request routing uses checked-layout request/effect views for
native duplicate/conflict decisions, receipt source selection, retry routing,
and reservation counts. Request-ID precedence and error ordering remain
unchanged. Rust owns receipt conversion, commits, session checks, and counters.
The adapter uses stack space proportional to generic ledger capacities.

Service-scale request completion/failure fences and checked/saturating counter
arithmetic also run in C. Rust preserves the original individual commit points,
including partial updates on capacity errors, and retains typed tables and
snapshot aggregation. Retry attempts keep their checked 32-bit limit; session
and in-flight counters keep checked 16-bit increments and saturated decrements.

Service-scale snapshots aggregate native instance/effect views in C, retaining
instance-counter totals and slot order. Debug overflow returns to the Rust
panic adapter; release sums wrap. Rust converts the snapshot to its public
type and keeps the service kind and typed table storage.

Service-scale table lookup and free-slot selection use native slot views with
explicit occupancy. First-match/free order, occupied closed/completed records,
and error/commit ordering stay intact. Rust retains typed payloads and converts
selected indices to records; views use stack space proportional to capacity.

Whole-file block-backed volume reads use `ghostos_volume_read_blocks` in `volume_range.c`. The
caller supplies the record checksum in `ghostos_volume_range_file.checksum`.
The function rejects short buffers before copying, validates every block and
the full record checksum, and rejects short, oversized, or cyclic chains.
The required size is reported separately; the read count changes only on
success. Existing ranged reads still allow a partial buffer. Both APIs share
component-by-component resolution for relative and absolute symlinks, with a
40-pass limit. Targets may come from checksum-validated blocks or the inline
data view. Invalid UTF-8 input paths are invalid paths; invalid UTF-8 or
oversized link targets are corrupt. Joining preserves the existing path
component rules, version selectors, and lookup error ordering.

`ghostos_volume_read_version_blocks` reads a selected nonzero version from
the same block/record views and validates the complete chain and record
checksum. It does not follow symlinks, and its path must have no version
suffix. Unlike the older contiguous-data `ghostos_volume_read_version`
helper, it validates stored block data. Exact-version payloads, including
symlink payloads, must be supplied through the block chain. Filesystem
consumer integration and behavior verification remain open.

`volume_reader.h` connects decoded volume records and a decoded type map to
whole-file, ranged, and exact-version block reads. The caller supplies raw
4096-byte blocks and storage for the borrowed file/block views. Names and data
remain borrowed; rebuild the reader after changing records, blocks, or kinds.
Block IDs map to the raw array at ID minus one. Non-data kinds cannot be read
as payload blocks. Checksums and chain validation remain in the read path.
This reader does not perform daemon capability, mode, or lock authorization.
The contiguous snapshot live read keeps `ghostos_volume_read`; the block-backed
read uses `ghostos_volume_read_blocks` so both APIs can link together.

`fsd_read.h` adds the C daemon read boundary over process registrations, open
file slots, and lock views. `ghostos_fsd_read` checks the 64 KiB buffer limit,
file token slot/generation/owner/READ rights, symlink-followed metadata, process
mode access, and other-owner exclusive locks before reading the block view.
Admin bypasses mode bits but still needs a READ file handle and obeys locks.
Process 1 uses owner permissions; other processes use other permissions.
Lock matching uses the stored open path and the starting record offset.
`ghostos_fsd_read_status` maps local results to the existing protocol status
values. The read boundary uses the shared handle and mode validators in
`fsd_handles.c` and the lock conflict checker in `fsd_resources.c`.

`fsd_handles.h` manages caller-owned process, file, mapping, lock, and snapshot
tables. Initialize fresh tables with `ghostos_fsd_handles_init`, then register
processes and validate authority before installing an already-authorized file
handle. Registration updates rights in place; slot reuse increments generation
and skips zero. Installation copies the resolved file name. Close invalidates
only mappings and locks tied to that exact file token. Process teardown clears
all owner slots and releases their checkpoints before removing registration.
`ghostos_fsd_handles_read_state` connects these tables to the existing C read
boundary. Callers serialize operations and keep all borrowed storage alive.

`fsd_resources.c` allocates and releases file locks, file mapping capabilities,
and snapshots. Locks copy their paths and preserve shared/exclusive, whole-file,
record-offset, and same-owner rules. Mapping tokens use a separate low-word bit;
admission checks alignment, file rights, mount policy, resolved metadata, modes,
locks, and capacity in the original order. Admission permits a mapping beyond
EOF when its start is at or before EOF; it does not install hardware pages.
Explicit snapshot release keeps the handle if the checkpoint release fails;
process teardown ignores release failure and clears the owner slot.

`fsd_checkpoint.h` connects snapshot callbacks to the C volume pin table. Set
`create_checkpoint = ghostos_fsd_checkpoint_create`,
`release_checkpoint = ghostos_fsd_checkpoint_release`, and `context` to a
`ghostos_fsd_checkpoint_backend` holding pins, next ID, and current generation.
Capacity and version-overflow results retain their filesystem status values.
The full open/create/truncate path, namespace selection, service dispatch,
tracing, and active service cutover remain unfinished.
