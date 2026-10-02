# Full Rust to C migration

- [ ] Port the entire project from Rust to C. The end state contains no Rust source, Rust build tooling, Cargo configuration, or Rust-only project workflows. Preserve public behavior, supported platforms, syscall and RPC ABI, on-disk formats, and documented contracts.
- [ ] Everything needs to be ported, there is no middle state with half rust and half C. Everything is ported to C.
- [ ] Replace the root Cargo workspace and every nested Cargo package, lockfile, Cargo configuration, and Rust toolchain pin with the C build system. Preserve dependency management, cross-compilation targets, and release settings.
- [ ] Port all Rust project tooling and its behavior: `tools/cargo-ghostos`, `tools/ghostos-compiler`, and `crates/ghostos-rustd`.
- [ ] Port all Rust tests, benchmarks, property/model tests, and fuzz infrastructure, including `kernel/benches`, `virtual_machine/benches`, and every target under `fuzz/fuzz_targets`.
- [ ] Update scripts, documentation, packaging, boot, and release workflows to remove Cargo, Rust tools, and Rust-built artifact dependencies.
- [ ] After all replacements are complete and verified, move obsolete Rust-only generated/build/configuration files into the project `Trash/` folder. Keep source history and user data intact. Do not move active C build output or delete files.

## Migration progress

The complete migration is in progress. About 235,559 Rust source lines need to
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

VM verification on 2026-10-01: `make c-library`, `make c-vm-test-binaries`,
`build/c/vm-device-contracts`, and `cargo build -p ghostos-vm` passed on the
AArch64 macOS host. `cargo test -p ghostos-vm` passed 274 tests with 17 opt-in
QEMU tests ignored. The full VM remains a mixed Rust/C executable, not a
standalone C VM. The host kernel C modules now propagate as a static library
dependency into the VM; their previous package-local link arguments caused
undefined quota, scheduler, and task symbols in the VM executable.

The status, ABI, compatibility, protocol, and boot-protocol C implementations
build into `build/c/libghostos.a`. See `c/README.md` for the source mapping and
interface conventions. Rust module checklist entries remain open until consumer
cutover and behavior parity are complete.

Migration build check on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with the active C PIT and guest-input ports.
The existing eight PIT cases also have C contract source in
`c/tests/vm_device_contracts.c`; these checks were not executed. The VM still
requires Rust adapters, and the full migration remains incomplete.

Further build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C PS/2, driver-report, and
guest-integration ports. The three existing PS/2 cases and three driver-report
cases were ported to C contract source and checked for compilation syntax only.
No new behavior-parity results are claimed; remaining Rust adapters still
prevent a full C cutover.

Serial/persistence build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with the active C serial and persistence
ports. All fourteen existing serial cases have C source in
`c/tests/vm_serial_contracts.c`, checked for compilation syntax only. Full C
cutover and behavior-parity verification remain open.

Virtio build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C block, console, RNG, and
Virtio-net controllers. Production queue processing now stays in C. Rust
host adapters and existing test source remain; behavior parity and full
project cutover are still open. Build logs are in `temp/`.

Migration/authentication build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with C migration framing and snapshot
SHA-256/HMAC implementations. No behavior-parity checks were executed.
Snapshot parsing and the full project cutover remain open. Build logs are
in `temp/c-library-migration.log` and `temp/vm-build-migration.log`.

DHCP build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with the active C DHCP configuration,
lease-policy, and reply encoder. Rust segment integration remains. No new
behavior-parity results are claimed. Build logs are in
`temp/c-library-dhcp.log` and `temp/vm-build-dhcp.log`.

BIOS build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C ROM/POST templates and
legacy disk, memory-map, and keyboard services. CPU/MMU integration and
boot handoff still use Rust. Behavior parity remains unverified. Build
logs are in `temp/c-library-bios.log` and `temp/vm-build-bios.log`.

Replay wire build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C replay file and event
header codecs. Reserved-byte acceptance, error ordering, and little-endian
layout are preserved in source. Behavior parity remains unverified. Build
logs are in `temp/c-library-replay.log` and `temp/vm-build-replay.log`.

Replay-session/display/terminal/e1000 build checks on 2026-10-02:
`make c-library` and `cargo build -p ghostos-vm` passed together with
these active C consumers on AArch64 macOS. The two existing display cases,
two e1000 cases, and policy portions of four terminal cases have C source
checked for compilation syntax only. No behavior-parity execution is claimed.
Final build logs are in `temp/c-library-final-ports.log` and
`temp/vm-build-final-ports.log`; syntax logs are in
`temp/display-contract-syntax.log` and `temp/io-contract-syntax.log`.

Storage-controller build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C NVMe and AHCI controllers.
C now owns registers, queues/command slots, identification, DMA processing,
completion records, reset state, and interrupt decisions. Rust retains disk
images, MMU/APIC adapters, and resource attachment APIs. The four existing
controller cases have C source in `c/tests/vm_storage_contracts.c`, checked
for compilation syntax only. Behavior parity and full cutover remain open.
Build logs are in `temp/c-library-storage.log` and `temp/vm-build-storage.log`;
the syntax log is `temp/storage-contract-syntax.log`.

Network-segment build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C shared Ethernet segments and
two-port loopback hubs. C owns packet queues, carrier state, segment port
identity/admin state, delivery/filtering, queue limits, loss injection,
disconnects, and transmission counters. Rust retains shared-owner/backend
adapters and independent loopback-handle settings. The two existing shared
segment cases have C source in `c/tests/vm_io_contracts.c`, checked for
compilation syntax only. Host UDP/raw transports, behavior parity, and full
cutover remain open. Logs are in `temp/c-library-segment.log`,
`temp/vm-build-segment.log`, and `temp/segment-contract-syntax.log`.

Host-network build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with the active C host-backend packet path
on AArch64 macOS. C owns admin/promiscuous state, validation ordering, UDP
framing and raw-frame padding, receive decoding/filtering, and saturating
transmit/receive counters. Rust retains socket construction/ownership,
send/receive calls, Linux carrier queries, and public backend adapters.
UDP short-send acceptance, raw short-send rejection, single-datagram receive,
and error ordering are preserved in source. Behavior parity and full cutover
remain open. Logs are in `temp/c-library-host-net.log` and
`temp/vm-build-host-net.log`.

Acceleration build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed on AArch64 macOS with active C backend
selection, platform probing order, HAXM path fallback, KVM API version policy,
native-handle rejection/close ordering, capability/fallback reporting, and
status formatting. The Linux KVM ioctl has C source; Linux/Windows builds
remain unverified. Rust retains file existence/open/ownership adapters,
typed handles/errors, attempt snapshots, and public const enum-name APIs.
Acquiring a native handle still leaves software guest execution active,
matching the existing behavior. No behavior-parity execution is claimed.
Logs are in `temp/c-library-acceleration.log` and
`temp/vm-build-acceleration.log`.

Path-pattern build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-path-pattern -p ghostos-fsd -p ghostos-ghostfs
-p ghostos-shell -p ghostos-runtime` passed with the active freestanding C
matcher. Existing validation/error ordering, byte-based classes, escaped
literals, and UTF-8 wildcard consumption are preserved in source. The seven
existing cases have C source in `c/tests/path_pattern_contracts.c`; no tests
were executed. Rust adapters, build tooling, and behavior parity remain open.
Build logs are in `temp/c-library-path-pattern.log` and
`temp/path-pattern-consumers-build.log`.

Disk-management build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C attachment management
and fixed-VHD repair eligibility/checksum repair. The historical 32-bit VHD
size offsets are retained; checksum repair syncs before the host directory sync.
Path canonicalization stays lazy and ordered; Unicode blank-ID checks and
native path equality remain in host adapters. Duplicate-ID/location precedence,
format-before-capacity validation, stable ID ordering, read-only clone bypass,
and copy-error-before-cleanup behavior are preserved in source. Policy portions
of the two existing cases are in `c/tests/vm_disk_management_contracts.c`,
checked for compilation syntax only. No tests were executed. Native filesystem
fixtures, behavior parity, and full Rust removal remain open. Logs are in
`temp/c-library-disk-management.log`, `temp/vm-build-disk-management.log`, and
`temp/disk-management-contract-syntax.log`.

Admission build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-admission -p ghostos-backup -p ghostos-inspect
-p ghostos-pkg -p ghostos-storaged` passed with the active freestanding,
allocation-free C controller. Slot storage stays caller-owned and movable;
no pointers are retained. Compile-time C/Rust layout checks cover the shared
controller, policy, lease, tenant, outcome, and report types on 64-bit targets.
Tenant-before-class-before-global error ordering, recovery class-limit bypass,
global queued-count consumption, first-free slots, and saturating counters
and sequence IDs are preserved in source. All three existing cases are in
`c/tests/admission_contracts.c`, checked for compilation syntax only. No tests
were executed; behavior parity, Rust adapter/build removal, and full migration
remain open. Logs are in `temp/c-library-admission.log`,
`temp/admission-consumers-build.log`, and `temp/admission-contract-syntax.log`.

Time-sync build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-time-sync -p ghostos-app -p ghostos-fabric
-p ghostos-netd -p ghostos-shell -p ghostos-storaged -p ghostos-top
-p ghostos-rustd -p ghostos-test-support` passed with active freestanding C
time synchronization. Compile-time shared-layout checks cover timestamps,
messages, pending exchanges, clocks, daemons, epoch counters, and atomic manual
clock storage on 64-bit targets. Source preserves packet validation/error
ordering, permissive encoded timestamps, trailing/reserved-byte acceptance,
sequence wrap skipping zero, saturating correction arithmetic, truncation
toward zero, and pending-state consumption before response timestamp failure.
All five retained cases have C source in `c/tests/time_sync_contracts.c`,
checked for compilation syntax only. No tests were executed. Behavior parity,
platform build coverage, Rust adapters/build removal, and full migration remain
open. Logs are in `temp/c-library-time-sync.log`,
`temp/time-sync-consumers-build.log`, and `temp/time-sync-contract-syntax.log`.

Monitor-control build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C monitor protocol/authentication.
C preserves request-overflow-before-append, trailing-command-before-UTF-8 error
ordering, Rust-compatible Unicode trimming and ASCII tail whitespace, exact
aliases, timestamp-before-hex-before-replay-before-HMAC-before-command-before-
permission checks, and nonce insertion only after full authorization. The
HMAC callback retains the existing domain, little-endian timestamp, nonce,
trimmed command, and constant-time tag comparison. JSON replies preserve key
order, escaping, and newline termination. The first five existing cases have
C source in `c/tests/vm_control_contracts.c`, checked for syntax only. No tests
were executed. Two VM diagnostic/redaction fixtures, platform coverage,
behavior parity, and full Rust removal remain open. Logs are in
`temp/c-library-control.log`, `temp/vm-build-control.log`, and
`temp/control-contract-syntax.log`.

Passkey-bridge build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C bridge protocol/state helpers.
The page (980 bytes), stylesheet (1,366 bytes), and script (10,806 bytes) match
the original Rust source byte-for-byte. HTTP parsing preserves first parseable
Content-Length, first token header, native-size decimal parsing, Unicode
whitespace, and the existing checked-overflow/wrapped-slice panic paths.
Percent decoding preserves literal incomplete percent escapes and decoded
UTF-8 validation. Guest-banner ordering, commit-only enrollment completion,
challenge retention/clearing, prompt-driven progression, and serial frame
layout are preserved in source. The single existing localhost-bind fixture
remains Rust; no tests were executed and no bridge/listener was started.
Host I/O, request routing/replies, token entropy, platform coverage, behavior
parity, and full Rust removal remain open. Logs are in
`temp/c-library-passkey.log`, `temp/vm-build-passkey.log`, and
`temp/passkey-assets-source-check.log`.

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
- [ ] `kernel/src/monitor.rs` — C owns process snapshots, switch-history CPU utilization, lock-summary aggregation, DSM page stats, and all four text renderers. The Rust module now only marshals scheduler/DLM data across the C ABI and retains the view state; shell integration remains Rust, and behavior parity still needs verification.
- [ ] `kernel/src/mouse.rs` — the Rust file is an ABI wrapper; C owns PS/2 packet collection, complete-packet publication, decoding, and sequence tracking. The C reader now sees the prior complete packet while a new packet is incomplete. Non-x86 keeps `mouse_stub.rs`; target build and behavior parity remain to verify.
- [x] `kernel/src/mouse_stub.rs` — the Rust API wrapper calls the C non-x86 stub, which always returns an empty mouse state. The `MouseState` size/alignment match is compile-time checked; the AArch64 kernel target builds.
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
- [ ] `kernel/src/scheduler.rs` — C owns ready-thread selection, realtime priority/deadline ordering, and IPC priority inheritance. Rust retains thread/context state, capability checks, CPU topology, NUMA, power policy, and hardware transitions.
- [ ] `kernel/src/shell.rs` — C owns VT input control-sequence/resize/UTF-8 decoding and command-line expansion after Rust registry matching. Rust retains the command registry/interpreter, renderer, editor, operator execution, and subsystem adapters.
- [ ] `kernel/src/syscall.rs` — C owns dispatcher registration, user pointer and request validation, memory map/unmap request-shape checks, and yield/sleep scheduler hints. Rust retains typed memory syscalls, architecture user-memory access, watchdog activity, and response writing.
- [ ] `kernel/src/task.rs` — C owns CPU ID validation and mask operations, address-space ID validation, and generation-tagged thread ID packing and decoding. Rust retains public wrappers, execution and scheduling enums, thread/context records, and persona/page-table fields.
- [ ] `kernel/src/tests.rs` — Added C kernel contract binaries covering boot validation, address-space isolation, capability generations/delegation (including generated attenuation cases), frame allocation, realtime scheduling, IPC bounds, invariants, syscall validation, runtime request validation, and service-image limits. Rust-only filesystem dispatch, quota/page-fault, scheduler lifecycle, architecture-source, shell, and deeper integration cases still need C ports and parity review.
- [ ] `kernel/src/time.rs` — C owns the atomic monotonic and realtime clocks, saturating updates, the 10 ms timer tick, RTC consistency sampling, CMOS BCD/binary and 12/24-hour decoding, date validation, and Unix conversion. Rust keeps the public function wrappers and passes the C tick duration into scheduler accounting; x86 kernel builds enable RTC I/O, other builds retain the unavailable RTC behavior.
- [ ] `kernel/src/tlb.rs` — C owns the bounded shootdown table, range/target validation, ID generation, acknowledgement/pending/completion state, and retirement. Rust preserves the typed API and supplies architecture invalidation/IPI callbacks. C contract coverage is in `c/tests/kernel_contracts.c`; full architecture behavior parity remains.
- [ ] `kernel/src/usb_keyboard.rs` — C owns xHCI controller setup, rings/contexts, device configuration, transfer polling, HID report edge tracking, Caps Lock, key/modifier translation, queued terminal navigation sequences, and descriptor selection. Rust retains PCI discovery, the keyboard API, and boot singleton; full consumer cutover and hardware parity remain.
- [ ] `kernel/src/usb_keyboard_stub.rs` — C stub initializes empty state and always returns no keyboard or byte; Rust retains the API wrapper. Non-x86 build and consumer parity remain.
- [ ] `kernel/src/watchdog.rs` — C owns atomic service and CPU heartbeats, one-shot stale-fault latches, diagnostics state, and polling. Rust adapts CPU masks and keeps watchdog call sites; behavior parity and full consumer cutover remain.
- [ ] `kernel/src/webauthn.rs` — C owns SHA-256; Rust still parses SYWB/client JSON/COSE and performs custom P-256 ES256 verification. Full crypto/parser port and behavior parity remain.
- [ ] `virtual_machine/src/boot/mod.rs` — C owns Multiboot header discovery and Multiboot information decoding; Rust still owns ELF loading, memory layout, boot-info serialization, and CPU/MMU handoff. Full loader port and parity remain.
- [ ] `virtual_machine/src/clock.rs` — C owns host/manual clock state, lifecycle, and time arithmetic; Rust retains the trait/shared-clock API wrapper. VM build compiles the implementation; behavior parity remains to verify.
- [ ] `virtual_machine/src/cluster.rs` — C adds the bounded deterministic network, shared-memory fixture, node membership and heartbeat state, fault recovery, VM run callbacks, and scale evidence in `c/src/vm_cluster.c`. The Rust CXL fixture and serial-output evidence still need C fabric and VM integrations; the C cluster is not yet the active VM consumer.
- [ ] `virtual_machine/src/control.rs` — C owns active bounded request buffering/framing and UTF-8 checks, Unicode-aware command/permission parsing, authentication field/number/hex decoding, timestamp-window validation, callback-ordered signature checks, 1024-entry FIFO nonce tracking, command/sensitive permission enforcement, JSON escaping, response-size policy, and help/action/error/envelope encoding. Rust retains host time/key/string/path/error adapters, const command APIs, VM diagnostic data assembly/redaction, and socket execution. The first five existing cases have C contract source; two VM diagnostic fixtures, behavior parity, and full cutover remain.
- [ ] `virtual_machine/src/cpu/decoder.rs`
- [ ] `virtual_machine/src/cpu/executor.rs`
- [ ] `virtual_machine/src/cpu/mod.rs`
- [ ] `virtual_machine/src/devices/apic.rs` — C owns the active xAPIC register state, MSR handling, interrupt priority/trigger tracking, EOI, IPI routing, and countdown timer in `c/src/vm_apic.c`. Rust retains device-trait/shared-owner adapters and the existing field-by-field snapshot format. The VM builds and the 16 retained APIC tests plus standalone C device contracts pass; removing the remaining Rust adapter awaits VM-wide cutover.
- [ ] `virtual_machine/src/devices/display.rs` — C owns active VGA text/framebuffer memory, register and palette state, cursor control, text/VESA rendering, UTF-8 text snapshots, PPM encoding, and INT 10h dispatch. Rust retains shared-owner/device adapters, GOP type translation, MMU callbacks, filesystem writes, and legacy panic translation. The two existing display cases have C contract source. Existing VBE mode-info width panic, LFB mode-mask, and 15-bit pitch behavior are preserved; full C cutover and behavior parity remain.
- [ ] `virtual_machine/src/devices/guest.rs` — C owns active guest-agent byte/event queues, mailbox registers, memory-hotplug request/notification state and validation, and pvclock MSR state, version sequencing, and little-endian page encoding in `c/src/vm_guest.c`. Rust retains shared-APIC ownership, MMU writes, and host/replay wall-time callbacks. Register error behavior, saturating arithmetic, and system-page-before-wall-time ordering are preserved; full Rust removal and behavior parity remain.
- [ ] `virtual_machine/src/devices/hpet.rs` — C owns the active HPET counter, register state, comparator scheduling, periodic reloads, reset, and interrupt routing in `c/src/vm_hpet.c`. Rust retains device-trait/shared-owner adapters and APIC ownership. The seven retained HPET tests and standalone C device contracts pass; removing the remaining Rust adapter awaits VM-wide cutover. Existing register aliases and timing behavior are preserved rather than claiming hardware-spec conformance.
- [ ] `virtual_machine/src/devices/input.rs` — C owns the active i8042 controller command state, bounded output ring, lossless pending-keyboard queue, keyboard/mouse command replies, mouse packet conversion, and interrupt decisions in `c/src/vm_ps2.c`. Rust retains shared-APIC ownership and deferred IRQ borrowing. The three existing PS/2 cases have C contract source; full Rust removal and behavior parity remain.
- [ ] `virtual_machine/src/devices/interrupt_controller.rs` — C owns IDT gate decoding and interrupt-controller state, routing, snapshots, and reset. Rust keeps the legacy PIC adapter that acknowledges through the `LocalApic` API; full cutover remains.
- [ ] `virtual_machine/src/devices/mod.rs`
- [ ] `virtual_machine/src/devices/net/e1000.rs` — C owns active MMIO register state, RX backlog, backend error tracking, TX/RX descriptor traversal, DMA completion, reset, and interrupt decisions. Rust retains backend, MMU, and APIC callbacks and public device adapters. The two existing e1000 cases have C contract source; existing RX indexing, queue limits, ignored status-write failures, and debug overflow behavior are preserved. Full C cutover and behavior parity remain.
- [ ] `virtual_machine/src/devices/net/mod.rs`
- [ ] `virtual_machine/src/devices/net/virtio.rs` — C owns active register state, RX backlog, backend error tracking, direct-index RX/TX descriptor traversal, DMA completion, and interrupt decisions in `c/src/vm_virtio_net.c`. Rust retains MMU, backend, and APIC callbacks and public device adapters. Shared-PFN and existing interrupt behavior are preserved; full C cutover and behavior parity remain.
- [ ] `virtual_machine/src/devices/pit.rs` — C owns the active three-channel counter state, control/read-back protocols, reload/read sequencing, host-time advancement, and terminal-count pulses in `c/src/vm_pit.c`. Rust retains the port-device/shared-APIC adapter and existing tests. Existing zero-reload, latch-consumption, and mode behavior is preserved; behavior parity remains unverified.
- [x] `virtual_machine/src/devices/power.rs` — C validates power-control port accesses and decodes ACPI sleep-enable writes into shutdown/reboot state. Rust keeps shared state and notification queue integration.
- [ ] `virtual_machine/src/devices/serial.rs` — C owns the active UART registers and divisor aliases, transmit/receive FIFOs, lossless paste queue, raw capture and compaction, newline conversion, panic detection, authentication-marker replacement/holding, prompt suppression, and spinner state/timing in `c/src/vm_serial.c`. Rust retains shared-APIC delivery, host console/clock callbacks, and public slice/vector adapters. All fourteen existing serial cases have standalone C contract source; full Rust removal and behavior parity remain.
- [ ] `virtual_machine/src/devices/storage/ahci.rs` — C owns the active single-port register state, command slots, ATA identification, PRD transfers, FIS completion, and interrupt decisions. Rust retains disk images and MMU/APIC adapters; behavior parity and full cutover remain.
- [ ] `virtual_machine/src/devices/storage/disk_image.rs` — C owns active RAW/fixed-VHD/QCOW2 parsing, sector I/O and QCOW2 allocation/cache handling, flush/sync requests, format names/parsing, and fixed-VHD repair eligibility/checksum repair. Rust retains file handles/callbacks, lock publication/inspection/recovery, host/process identity, report assembly, and cleanup lifetimes. Native filesystem fixtures, behavior parity, and full cutover remain.
- [ ] `virtual_machine/src/devices/storage/management.rs` — C owns active ordered specification checks, duplicate IDs/system roles/controller locations/writable-source rejection, image format/capacity validation, controller/guest identity text, clone selection and 32-attempt reserve/copy/cleanup sequencing, and stable sorted attachment storage/lookup/removal. Rust retains public types, native file/path/clock callbacks, image ownership, and typed errors. Policy portions of both existing cases have C source; native filesystem fixtures, behavior parity, and full cutover remain.
- [ ] `virtual_machine/src/devices/storage/mod.rs`
- [ ] `virtual_machine/src/devices/storage/nvme.rs` — C owns the active register state, admin/I/O queues, identification/features, read/write/flush commands, completion encoding, and DMA processing. Rust retains namespace images and MMU/APIC adapters; behavior parity and full cutover remain.
- [ ] `virtual_machine/src/devices/storage/persistence.rs` — C owns active persistence-port modes, length/cursor handling, command transitions, tail-region loading/writing, SYNOPS01 little-endian headers, FNV-1a checksums, and zero-filled sector serialization in `c/src/vm_persistence.c`. Rust retains the DiskImage owner and sector/sync/error callbacks. The legacy port-error behavior and failed-I/O transitions are preserved; full Rust removal and behavior parity remain.
- [ ] `virtual_machine/src/devices/storage/system_disk.rs`
- [ ] `virtual_machine/src/devices/virtio.rs` — C owns active legacy transport registers, block request parsing and sector transfer sequencing, console capture/newline handling, RNG filling, queue completion, and interrupt decisions in `c/src/vm_virtio.c`. Rust retains disk, MMU, entropy, host console, and APIC callbacks. Full C cutover and behavior parity remain.
- [ ] `virtual_machine/src/devices/virtio_queue.rs` — C owns split-ring address calculation, avail-ring consumption, descriptor-chain validation, and used-ring completion. Production controllers call C directly; the Rust wrapper remains only for existing test source. Full Rust removal and end-to-end behavior parity remain.
- [ ] `virtual_machine/src/driver_capabilities.rs` — C owns the active ten-entry discovery report, availability and fallback selection, and static report text in `c/src/vm_driver_capabilities.c`. Rust retains public enum/array/string adapters and the const kind-name API. The three existing report cases have C contract source; full Rust removal and behavior parity remain.
- [ ] `virtual_machine/src/execution.rs`
- [ ] `virtual_machine/src/firmware/bios.rs` — C owns active BIOS ROM generation, POST IVT/BDA templates, INT 13h disk register services and CHS/EDD transfers, INT 15h memory sizing/E820 encoding, and INT 16h keyboard responses in `c/src/vm_bios.c`. Rust retains context ownership, CPU/MMU adapters, video/UEFI dispatch, and MBR boot handoff. Existing EDD packet offsets/count handling and CHS ignored-write-error behavior are preserved; full C cutover and behavior parity remain.
- [ ] `virtual_machine/src/firmware/mod.rs`
- [ ] `virtual_machine/src/firmware/uefi.rs`
- [ ] `virtual_machine/src/hardware_acceleration.rs` — C owns active backend negotiation, platform/path probing order, KVM version checking/ioctl, rejected-handle cleanup decisions, fallback/capability policy, and status formatting. Rust retains native file ownership/probe adapters, typed handles/errors and attempt snapshots, and public const name APIs. Software guest execution remains active; full cutover and behavior parity remain.
- [ ] `virtual_machine/src/input.rs` — C owns terminal-resize encoding, complete resize-response filtering, and ASCII/control-byte to PS/2 make/break conversion in `c/src/vm_input.c`. Active VM callers use Rust vector/type adapters; behavior parity remains unverified.
- [ ] `virtual_machine/src/integration.rs`
- [ ] `virtual_machine/src/lib.rs`
- [ ] `virtual_machine/src/main.rs`
- [ ] `virtual_machine/src/memory/mod.rs`
- [ ] `virtual_machine/src/migration.rs` — C owns bounded frame decoding, little-endian wire fields, exact/trailing length checks, authentication-domain assembly, constant-time tag comparison, and authentication-before-identity-before-snapshot validation in `c/src/vm_migration.c`. Rust retains snapshot/schema callbacks, owned payloads, and public errors. Full C cutover and behavior parity remain.
- [ ] `virtual_machine/src/net/backend.rs` — C owns active shared-segment and loopback queues, delivery/filtering, segment port identity/admin state, carrier, limits, loss injection, disconnects, and counters. C also owns host-backend admin/promiscuous state, packet validation, UDP/raw framing, receive filtering, and counters. Rust retains shared-owner/backend adapters, independent loopback-handle settings, host socket ownership/send/receive, and Linux carrier queries. The two existing segment cases have C contract source; behavior parity and full cutover remain.
- [ ] `virtual_machine/src/net/dhcp.rs` — C owns active configuration validation, bounded reservation and lease tables, deterministic allocation, renewal/release/expiry policy, DHCP request parsing, OFFER/ACK/NAK reply encoding, IPv4/UDP checksums, and bounded option writing in `c/src/vm_dhcp.c`. Rust retains segment ownership and receive/transmit polling, public config/lease adapters, and existing fixture test source. Legacy explicit-request reservation behavior is preserved; full C cutover and behavior parity remain.
- [x] `virtual_machine/src/net/mac.rs` — C owns byte conversion, broadcast/unicast/multicast classification, destination filtering, and lowercase formatting in `c/src/vm_mac.c`. Rust keeps the public type and const constructors for API compatibility.
- [ ] `virtual_machine/src/net/mod.rs` — C owns the alignment helper; Rust module exports and networking submodules remain.
- [x] `virtual_machine/src/net/packet.rs` — C owns bounded packet queue storage, byte and packet accounting, queue operations, Ethernet minimum-frame padding, and error display strings in `c/src/vm_packet.c`. The VM build links this host-allocator module; Rust keeps the public `Vec` and error-code wrappers.
- [ ] `virtual_machine/src/passkey_bridge.rs` — C owns active HTML/CSS/JavaScript asset storage, bounded HTTP request-shape parsing with borrowed offsets, Unicode-aware header/word trimming, content-length and token-header policy, query lookup, percent/hex decoding, username checks, CR/binary serial framing, banner/challenge detection, observation-driven enrollment/login state transitions, and prompt-driven input readiness. Rust retains sockets/timeouts, browser launch, token entropy, request-route/VM handoff and reply assembly, owned buffers, and the existing localhost-bind fixture. Browser asset bytes were checked against the original source; behavior parity and full cutover remain.
- [ ] `virtual_machine/src/replay.rs` — C owns active SYNVMRP1 file/header codecs, trace and file-size validation, session-owned records and payloads, mode/cursor/capacity transitions, error latching, instruction/clock/timer/interrupt/host-input replay, and DMA payload encoding/decoding. Rust retains typed trace snapshots, filesystem I/O, error formatting, and shared-owner adapters. Full C cutover and behavior parity remain.
- [ ] `virtual_machine/src/snapshot.rs` — Active SHA-256, streaming multipart HMAC-SHA256, and fixed-length authentication comparisons use `c/src/vm_snapshot_auth.c`. Rust retains key wrappers, snapshot serialization/deserialization, restoration, diff/checkpoint storage, and schema policy; the complete snapshot port and behavior parity remain.
- [ ] `virtual_machine/src/terminal.rs` — C owns active CR/LF and DEL input policy, resize polling and change detection, poll buffers, transcript storage/replay, EOF byte generation, and diagnostics counters. Rust retains supplied streams, reader threads, native error-kind adapters, shared clock ownership, and typed transcript snapshots. Policy portions of four existing terminal cases have C contract source. Stream/thread and PTY fixture ports, full C cutover, and behavior parity remain.
- [ ] `virtual_machine/src/terminal_platform.rs` — C owns native Unix terminal handles, saved/raw termios, signal registration/restoration, single-session guarding, and terminal-size queries; Windows console modes/rollback/restoration and portable fallbacks also have C source. Rust retains the RAII and I/O-error adapter. The AArch64 macOS host builds; Linux/Windows builds, PTY behavior parity, and full C cutover remain.

Each entry names a project area containing Rust source files. Port every `.rs` file in each listed tree, including nested source, test, benchmark, example, and build-script files. These are all in-scope port targets, not optional cleanup.

- [ ] `boot/uefi/` — UEFI bootloader.
- [ ] `crates/abi/` — ABI definitions and generated ABI bindings.
- [ ] `crates/actors/` — actor runtime.
- [ ] `crates/admission/` — C owns active fixed-capacity controller state, ordered tenant/class/global budget checks, recovery reserves, bounded ancestor charging, tenant-policy validation/cycle rejection, first-free lease allocation and exact lease validation, queue/drop/retry decisions, saturating counters/sequences, and reports. Backup, inspection, package, and storage consumers use allocation-free Rust type adapters. All three existing cases have C source; const API wrappers, Rust build/adapters, behavior parity, and full removal remain.
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
- [ ] `crates/path-pattern/` — Active parsing, wildcard/class matching, UTF-8-width consumption, and unescaping now run in `c/src/path_pattern.c`. Filesystem, shell, and runtime consumers use a Rust type/FFI adapter. All seven existing contract/property cases have C source. Rust adapter/build-tool removal and behavior parity remain open.
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
- [ ] `crates/time-sync/` — C owns active atomic manual-clock reads/max/saturating advance, timestamp conversion, SPTP packet encoding/decoding, master/slave exchanges and pending state, clock discipline/frequency adjustment and monotonic correction, and epoch issuance/observation. All eight dependent crates call C through allocation-free Rust adapters. Portable signed-magnitude wide arithmetic preserves i128 intermediates without compiler extensions. All five existing cases have C source; const constructors/getters, traits, enum/status adapters, Rust tooling/removal, and behavior parity remain open.
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

Inventory: 546 Rust source files and 75 Cargo manifests were found. The root
workspace lists 71 members; the other manifests include the fuzz package and
nested example/proc-macro packages. Port every file in these trees before
claiming the project has no Rust left.
