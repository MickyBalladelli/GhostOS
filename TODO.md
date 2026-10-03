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

Execution-engine build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C translation/dispatch loops,
instruction boundaries, backward-loop classification, source-range checks,
version observation, hot-block promotion, and cache-retune timing. Translation
still returns the first decode error, accepts a partial block after a later
decode error, and reads/marks the final wrapping source range. Dispatch checks
self-modifying code before translation changes, preserves replay cleanup and
unsupported-instruction shadow restoration through host callbacks, and skips
profile notification after an execution error. Rust retains cache/profile
storage and accounting, admission/eviction, CPU/MMU/device operations, and owned
errors/instructions. No tests were executed; behavior parity, platform coverage,
and full cutover remain open. Logs are in `temp/c-library-execution.log` and
`temp/vm-build-execution.log`.

Execution-cache build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C hash-table cache storage,
FIFO key tracking/eviction, profile and instruction-count maps, and counter
updates. Cache keys preserve RIP, CPU mode, privilege, and CR3. Hash-table
resizing preserves borrowed payload/profile addresses. Successful insertion
transfers boxed-block ownership to C; removal/eviction transfers it back;
replacement, clearing, and drop destroy each retained block once. Reset keeps
stale FIFO keys and cache-policy accounting as before, while clear-cache clears
both cache and FIFO. Capacity zero still means one entry, stale rejection
removes every matching FIFO key, and eviction still precedes admission.
Profile namespaces/default-start behavior and ordered checked/wrapping counter
updates are preserved in source. Rust still owns decoded blocks, CPU/MMU/device
callbacks, cache-policy integration, profile hooks, public types, and error
adapters. No tests were executed; parity, platform coverage, and complete Rust
removal remain open. Logs are in `temp/c-library-execution-cache.log` and
`temp/vm-build-execution-cache.log`.

UEFI-loader build checks on 2026-10-02: `make c-library` and
`cargo build -p ghostos-vm` passed with active C PE32+ parsing, section/range
validation, guest image zero-fill/mapping, DIR64/HIGHLOW relocations, stable
memory-map ordering and 48-byte descriptors, GetMemoryMap/ExitBootServices
handshake policy, boot-service classification, and firmware stub/header/RSDP
encoding. Parsing preserves unsupported-versus-invalid error precedence,
zero-length raw-section pointer acceptance, raw-size-based image bounds, and
relocation-directory bounds. Relocations keep virtual reads with zero on error,
ignored value-write errors, ignored unknown types, modulo-width deltas, and
checked-debug/wrapping-release address arithmetic. Mapping keeps physical
writes, overlap checks, and zero-fill/section writes before missing-relocation
rejection. GetMemoryMap keeps physical-descriptor write failures, ignored
metadata writes, native-width wrapping map keys, and original pointer/status
ordering. Existing RSDP field offsets are preserved, not repaired as part of
the port. Rust retains context/image ownership, guest/MMU callbacks, table
assembly, CPU handoff, console/protocol/image-registration/runtime services,
and existing test fixtures. No tests were executed; platform coverage, parity,
and full Rust removal remain open. Logs are in `temp/c-library-uefi.log` and
`temp/vm-build-uefi.log`.

Durability build checks on 2026-10-03: `make c-library` and
`cargo build -p ghostos-durability -p ghostos-ghostfs -p ghostos-declarative
-p ghostos-update -p ghostos-storaged -p ghostos-init -p ghostos-pkg
-p ghostos-test-support -p ghostos-system-model` passed on AArch64 macOS.
C owns active bounded trace recording and two-pass ordering/recovery validation.
The source preserves latest-matching-step selection, missing-step-before-order
errors, optional rename/storage/cache rules, post-commit data rejection,
sync-validation-before-recovery errors, and recovery against the latest power
loss without clearing earlier acknowledgements. Shared event size and offsets
are compile-time checked. The Rust typed event view uses parallel C records,
increasing trace storage; Rust enums, const tables, traits, and build adapters
remain. No tests were executed; behavior parity and full migration remain open.
Build logs are in `temp/c-library-durability.log` and
`temp/durability-consumers-build.log`.

NUMA/platform-I/O build checks on 2026-10-03: `make c-library` and
host builds of NUMA, platform I/O, kernel, VM, netd, storaged, compute, and
remote display passed. Compute and remote display also built in release mode.
C NUMA preserves sparse-node ordering, explicit-node preference, valid-CPU
retention, UMA-before-remote locality selection, wrapping per-kind fallback
cursors, counter retention on topology changes, and saturating counters.
C queue metadata preserves first matching slot from each cursor, generation
wrap skipping zero, stale-token-before-state errors, completed slots consuming
capacity until polling, and zero-capacity behavior. Generic payloads stay
Rust-owned. I/O/media validation preserves device/format/plane/buffer/access
error ordering, optional control buffers, and flush ignoring supplied buffers.
Both NUMA cases and all four platform-I/O queue model cases have C source and
passed syntax checks only. Logs are in `temp/c-library-numa.log`,
`temp/numa-consumers-build.log`, `temp/c-library-platform-io.log`,
`temp/platform-io-consumers-build.log`,
`temp/platform-io-direct-consumers-build.log`, and
`temp/platform-io-release-build.log`.

Compatibility/protocol consumer build checks on 2026-10-03: `make c-library`
and builds of API compatibility, protocol, client SDK, package, shell,
declarative configuration, HTTP, fabric, mesh, web terminal, and VM passed.
Existing C foundations now serve active Rust callers. Compatibility keeps
invalid-range-before-version rejection, legacy warnings, and migration exact
version matching independently of the supported range. Protocol keeps
negotiation-before-size errors, replay-window updates, locked-auth persistence,
checked inflight accounting, eight reconnect attempts, and saturating retry
arithmetic. Shared C/Rust sizes and field offsets are compile-time checked.
The four changed C modules passed freestanding syntax/layout checks for
x86-64, AArch64, and RISC-V; these are not full target OS builds. Existing
foundation and new NUMA/platform-I/O contract source passed syntax checks.
No tests were executed; behavior parity, Rust adapter/build removal, and full
migration remain open. Logs are in
`temp/c-library-api-compat-consumers.log`,
`temp/api-compat-consumers-build.log`,
`temp/c-library-protocol-consumers.log`,
`temp/protocol-consumers-build.log`,
`temp/ported-foundation-contract-syntax.log`, and
`temp/c-ports-{x86_64,aarch64,riscv64}-syntax.log`.

Policy/kernel consumer build checks on 2026-10-03: `make c-library` and
host builds of policy, authentication, package management, netd, storaged,
update, and VM passed. The policy C implementation owns every snapshot mutation
and all five simulations. Source retains principal-before-object lookup,
duplicate/replace-before-capacity decisions, inactive binding rights checks,
kind/staleness/zero-revision error ordering, direct-child inclusion even when
inactive, active-only bound-principal propagation, insertion-order de-duplication,
and the original whole-word FNV fingerprint mixing and optional-field omission.
C simulations retain the epoch/fingerprint and never mutate the snapshot.
Rust keeps the existing public report and lazy Option-record debug views;
report conversion uses an additional 8,632-byte temporary C report. All three
retained fixtures have C source in `c/tests/policy_contracts.c`, checked for
syntax only. The x86 keyboard now uses the previously built C decoder/controller
and C-owned boot singleton. Invariant checks/formatting call C, while Rust
const interfaces and panic behavior remain.

The x86-64 kernel image, including its Ring 3 boot shell, and the AArch64 kernel
library built successfully. Those target builds exposed and resolved two
migration blockers: hosted `string.h` in admission/time-sync, and BSD archive
indexes from macOS `ar` that lost native symbols during ELF Rust linking.
The two modules now use freestanding byte helpers. The temporary Rust adapter
build scripts select GNU LLVM archives for ELF targets, COFF for MSVC targets,
and retain native Apple archives and explicit `AR` overrides. Shared build
support tracks `AR`/`CLANG` changes. Rust's bare-metal RISC-V target name is
mapped to Clang's base triple with RV64GC/lp64d settings; kernel C objects use
the same ISA/ABI. The RISC-V kernel library also built with
`CLANG="$PWD/temp/clang-riscv64-verify.py"`: Apple Clang has no RISC-V codegen
backend, so that verification wrapper emits LLVM IR and uses the installed
Rust LLVM `llc`. The inspected policy object has the double-float/RVC ELF flags.
This is a configured compiler verification, not proof that Apple Clang alone
can produce RISC-V objects. Default Windows/MSVC builds remain unverified.

Changed C modules passed freestanding syntax/layout checks for x86-64,
AArch64, and RISC-V. Policy, admission, and time-sync contract sources passed
syntax checks only. No tests were executed; hardware/behavior parity and full
Rust removal remain open. Logs are in
`temp/c-library-policy-kernel-final.log`,
`temp/policy-kernel-consumers-final-build.log`,
`temp/kernel-keyboard-invariants-x86_64-build.log`,
`temp/kernel-invariants-aarch64-build.log`,
`temp/kernel-invariants-riscv64-build.log`,
`temp/policy-foundation-contract-syntax.log`, and
`temp/policy-kernel-{x86_64,aarch64,riscv64}-syntax.log`.
The RISC-V verification wrapper and its LLVM IR/object artifacts are in `temp/`.

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
- [x] `kernel/src/boot_diagnostics.rs` — Active runtime transitions, attempt rollover/interruption handling, failure counts, encoding, checksums, and decoding now use C. Rust retains const public values, typed adapters, and storage orchestration. Raw-status encoding preserves directly constructed public records. Full Rust removal and behavior parity remain part of the overall migration.
- [x] `kernel/src/boot_services.rs` — C port has the 13-service registry and dependency ordering, kernel launch callback integration, owned filesystem lifecycle, transactional first-admin provisioning and recovery, durable passkey key/counter records, filesystem-rights enforcement, and session-scoped shell authority. The C runtime consumes kernel and filesystem callbacks so platform implementations provide process launch and filesystem operations.
- [x] `kernel/src/capability.rs`
- [x] `kernel/src/console.rs`
- [x] `kernel/src/contention.rs`
- [x] `kernel/src/cow.rs` — C reference tracker, sharing and release, allocator-backed write faults, and page-copy callback.
- [x] `kernel/src/crash.rs` — kernel builds and links the C capsule encoder and one-time crash guard; the kernel supplies register, capability, scheduler, audit, and persistence data.
- [ ] `kernel/src/dlm.rs` — C fence tables, capability checked range locks, lease epochs, FIFO promotion, and contention reports are implemented and built. Shell diagnostics read the C singleton, but kernel lock operations still use the Rust manager; complete consumer cutover and behavior parity remain.
- [ ] `kernel/src/dma.rs` — Active callers use C-owned mapping records, request/buffer validation, IOVA allocation, ID generation, exact owner/authority checks, lookup, and staged map/unmap commits. Caller-owned arrays preserve zero and arbitrary generic capacities. Rust retains capability-space checks, typed public/const APIs, and IOMMU trait calls outside C, preserving host unwinding and commit-after-approval ordering. Behavior parity and full Rust removal remain.
- [ ] `kernel/src/driver_capabilities.rs` — Active callers use C PCI role/BAR selection, range validation, MMIO placement, image-overlap checks, and manifest creation. Standalone and active consumers share the same selection implementation. Rust retains PCI type translation, capability minting/error propagation, and the typed mapping list; capacity-before-range checks and partial-grant side effects are preserved. Shared layouts and MMIO/image constants have compile-time checks. Behavior parity and full cutover remain.
- [ ] `kernel/src/hot_allocator.rs` — Active callers use C pool bitmaps, allocation/fallback order, reclaim checks, counters, and fragmentation reports. A shared borrowed view supports generic CPU/node capacities without the standalone C limits. Rust retains typed ownership tokens, public const accessors/stats constructors, remote-memory tracing, and checked-overflow/zero-divisor panic conversion. Host/VM and all three target library builds pass; behavior parity and full Rust removal remain.
- [ ] `kernel/src/invariants.rs` — Active address-space checks, page-table-transition checks, and redacted failure formatting now call C in host and target kernels. Host native-library metadata includes the C module for VM consumers. Rust keeps the public const catalogue/IDs/failure constructors, const interrupt check, and debug panic policy to preserve const APIs and host unwinding. x86 image, AArch64 library, and RISC-V library builds pass; behavior parity and full cutover remain.
- [ ] `kernel/src/ipc.rs` — C port implements bounded lock-free MPMC channels, capability-checked send/receive and transfers, quotas, mapped endpoints, close/owner cleanup, partition and scheduler callbacks, diagnostics, and stuck reports. Rust kernel consumers remain active; behavior parity and consumer cutover remain.
- [ ] `kernel/src/keyboard.rs` — Active x86 kernel and shell keyboard callers now use C controller setup, native port I/O, Set 1 decoding, modifier state, four-byte navigation queues, ACK filtering, mouse-byte delivery, and the serialized C boot singleton. Rust retains checked-layout instance/callback-table wrappers and the mouse API callback. The x86 kernel and boot shell build; hardware behavior parity, Rust wrapper removal, and full cutover remain.
- [ ] `kernel/src/keyboard_stub.rs` — C stub initializes empty state and always reports no key, matching non-x86 kernel behavior. Rust callers remain active; behavior parity and consumer cutover remain.
- [ ] `kernel/src/lib.rs` — C port adds the aggregate C kernel API, callback-driven boot coordinator, boot validation and stage reporting, dispatch boundary, service readiness, fatal/crash path, and login throttling/session/quote state. The Rust syscall dispatcher, hardware consumers, process/scheduler integration, and shell remain active; parity and consumer cutover remain.
- [ ] `kernel/src/litmus.rs` — Active callers use C schedule append, deterministic seeded generation, all six ordering models, fault replay, and failure minimization. Rust retains typed/const public values and translates C reports and bounds failures. The checked replay bridge preserves oversized public schedules that stop early without failure, and reports bounds panics when execution or minimization would index beyond storage. Behavior parity and full Rust removal remain.
- [ ] `kernel/src/main.rs` — C port owns the target `_start` entry symbol and forwards boot info to `kernel_entry`. Rust keeps only the compiler-required panic ABI hook, which forwards to kernel panic reporting; full panic-handler cutover remains.
- [ ] `kernel/src/micro_silo.rs` — Active callers use C hardware protection, non-overlapping memory maps, borrowed-range cleanup, and address lookup. Caller-owned tables preserve zero and arbitrary capacities, kernel address-space acceptance, directly constructed ranges, and debug overflow ordering. Public const APIs remain Rust; behavior parity and full cutover remain.
- [ ] `kernel/src/monitor.rs` — C owns process snapshots, switch-history CPU utilization, lock-summary aggregation, DSM page stats, and all four text renderers. The Rust module now only marshals scheduler/DLM data across the C ABI and retains the view state; shell integration remains Rust, and behavior parity still needs verification.
- [ ] `kernel/src/mouse.rs` — the Rust file is an ABI wrapper; C owns PS/2 packet collection, complete-packet publication, decoding, and sequence tracking. The C reader now sees the prior complete packet while a new packet is incomplete. Non-x86 keeps `mouse_stub.rs`; target build and behavior parity remain to verify.
- [x] `kernel/src/mouse_stub.rs` — the Rust API wrapper calls the C non-x86 stub, which always returns an empty mouse state. The `MouseState` size/alignment match is compile-time checked; the AArch64 kernel target builds.
- [ ] `kernel/src/page_fault.rs` — C now owns x86 fault decoding, one-time handler install and dispatch, quota consume/refund ordering, stack-map-before-commit ordering, and COW write sequencing. Rust adapters retain the native capability, allocator, and address-space objects; moving those objects and full cutover remain.
- [ ] `kernel/src/partition.rs` — Host and target kernel methods call C for CPU mask queries, housekeeping policy, and online/isolate/release transitions. Rust-only host fallback is removed; shared mask layout has compile-time checks. Rust retains the const constructor and typed scheduler-owned wrapper; full cutover and behavior parity remain.
- [ ] `kernel/src/pci.rs` — C owns the x86 CF8/CFC scan and bounded 64-device inventory. Rust adapts entries to the existing boot-facing iterator and logging; driver and storage consumers still use that adapter.
- [ ] `kernel/src/persistence.rs` — C performs bounded x86 persistence-port load/save/flush plus active SYNREC01 record encoding/decoding, FNV-1a checksums, legacy SYNCRSH1/SYNBTD01 acceptance, zero-filled slot updates, and boot-record extraction. Rust retains the slice/storage adapter and temporary record/buffer ownership; full Rust removal and behavior parity remain.
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
- [ ] `virtual_machine/src/cluster.rs` — Active `VmCluster` consumers now route bounded network delivery, shared-memory fixture operations, node membership and heartbeat state, fault recovery, VM run callbacks, and scale evidence through `c/src/vm_cluster.c`, and `virtual_machine/build.rs` links the C module into the VM crate. The Rust CXL fixture and serial-output evidence remain, so full fabric cutover and broader behavior-parity verification are still open.
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
- [ ] `virtual_machine/src/execution.rs` — C owns active translation/dispatch loops, instruction boundaries and backward-loop classification, source-range validation, observed-version updates, saturating hot-block promotion, cache-retune timing, context-keyed cache hash tables and FIFO eviction, block-payload ownership, profile/instruction-count maps, and ordered checked/wrapping counter updates. Rust retains cache-policy admission/events, CPU/MMU/device callbacks, decoded-instruction allocation, public types and profile hooks, and error adapters. Build checks passed; behavior parity and full cutover remain.
- [ ] `virtual_machine/src/firmware/bios.rs` — C owns active BIOS ROM generation, POST IVT/BDA templates, INT 13h disk register services and CHS/EDD transfers, INT 15h memory sizing/E820 encoding, and INT 16h keyboard responses in `c/src/vm_bios.c`. Rust retains context ownership, CPU/MMU adapters, video/UEFI dispatch, and MBR boot handoff. Existing EDD packet offsets/count handling and CHS ignored-write-error behavior are preserved; full C cutover and behavior parity remain.
- [ ] `virtual_machine/src/firmware/mod.rs`
- [ ] `virtual_machine/src/firmware/uefi.rs` — C owns active PE32+ header/section validation, guest image mapping/zero-fill, DIR64/HIGHLOW relocation traversal, stable image-based memory-map encoding, GetMemoryMap/ExitBootServices policy, boot-service classification, and firmware stub/header/RSDP bytes. Rust retains context/image ownership, guest/MMU callbacks, firmware table assembly, CPU handoff, console/protocol/image-registration/runtime services, and existing test fixtures. Host builds pass; parity and full cutover remain.
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
- [ ] `crates/actors/` — C owns mailbox validation, envelope identity checks, endpoint matching, directory and node-route slot selection, and spawn prechecks. Rust retains actor traits, transport and runtime calls, typed records, and commit points after those calls. The existing mailbox test has C source; behavior parity beyond that test and full removal remain.
- [ ] `crates/admission/` — C owns active fixed-capacity controller state, ordered tenant/class/global budget checks, recovery reserves, bounded ancestor charging, tenant-policy validation/cycle rejection, first-free lease allocation and exact lease validation, queue/drop/retry decisions, saturating counters/sequences, and reports. Backup, inspection, package, and storage consumers use allocation-free Rust type adapters. All three existing cases have C source; const API wrappers, Rust build/adapters, behavior parity, and full removal remain.
- [ ] `crates/api-compat/` — Active contract checks and exact-version migration decisions call the existing C compatibility implementation through a value-only bridge. Public Rust const contracts, version helpers, migration strings, formatting, and result/error wrappers remain. Consumer builds pass; behavior parity and complete Rust removal remain.
- [ ] `crates/app/` — application loading, manifests, and supervision.
- [ ] `crates/auth/` — authentication.
- [ ] `crates/balancerd/` — balancing daemon.
- [ ] `crates/boot-protocol/` — boot protocol and handoff.
- [ ] `crates/client-sdk/` — client SDK and wire protocol.
- [ ] `crates/compute/` — tensor and accelerator compute.
- [ ] `crates/durability/` — C owns active bounded trace recording and durable-write ordering/recovery verification, with a matching six-layer contract table and interruption callback API. Rust retains public enums, const contract tables, injector traits, and the typed event view (with parallel C records). Rust build/adapters, behavior parity, and full removal remain.
- [ ] `crates/fabric/` — cluster fabric, memory, CXL, and DSM.
- [ ] `crates/fsd/` — filesystem daemon.
- [ ] `crates/ghostfs/` — GhostFS storage and volume management.
- [ ] `crates/ghostos-agent-bridge/` — C owns task-scope validation, parent and lifetime limits, expiry arithmetic, reusable grant slots, nonce and revocation wrap, consume checks, run-right unions, active-grant counts, and commit/discard decisions. Rust retains signature and lease verification, script execution, and GhostFS sandbox calls. Existing attenuation tests were executed.
- [ ] `crates/ghostos-agentd/` — agent daemon.
- [ ] `crates/ghostos-audit/` — C owns advisory identifier checks, advisory and obsolescence slot selection, out-of-date version ordering, finding de-duplication, and scan-budget validation. Rust retains feed insertion records, withdrawn-match filtering, package iteration, and batch scheduling. No existing audit tests were present; C contract checks cover the slot rules.
- [ ] `crates/ghostos-backup/` — C archive streaming in `c/src/backup.c` writes `SYNBACK1` volume headers, file records, and `SYNBEND1` trailers, and rejects a zero byte budget before changing the job. C chunk crypto in `c/src/backup_crypto.c` seals and opens 4096-byte chunks with an HMAC tag. Filesystem checkpoints remain. Rust sources were not changed.
- [ ] `crates/ghostos-confidential/` — C enclave binding in `c/src/enclave.c` rejects a zero measurement, checks page-aligned ranges, selects the first free node slot, and reports whether a range is protected. C capabilities in `c/src/confidential_capability.c` issue, validate, and revoke caller-owned records, including expiry-before-authorization and epoch wrap. C fabric frames in `c/src/confidential_fabric.c` seal and open DSM packets, encode `SCF1` frames, and reject replayed nonces. C attestation in `c/src/attestation.c` registers nodes, issues challenges, and admits HMAC quotes. Rust sources were not changed.
- [ ] `crates/ghostos-debug/` — C probe programs in `c/src/probes.c` verify forward-only jumps, execute bounded arithmetic, and record emitted values in a ring. C GDB framing in `c/src/gdb.c` buffers partial packets, checks checksums, reads memory, and rejects writes without permission. C coredumps in `c/src/coredump.c` validate `/cores` paths and encode `SYNCORE1` metadata and page records. C remote sessions in `c/src/remote_debug.c` check the capability wire header, target, nonce, and operation rights. Signature authorization remains outside C. Rust sources were not changed.
- [ ] `crates/ghostos-declarative/` — C configuration signatures in `c/src/config_signature.c` derive key ids, trust keys, and verify a signed revision against one node or a cluster. C activation in `c/src/reconfigure.c` checks history, stale revisions, and the already-active revision before commit, and rolls back to the previous revision. C system parsing in `c/src/config_parser.c` reads schema, revision, and service records. C network parsing in `c/src/config_network.c` reads hostname, interfaces, and routes, and rejects a route whose interface was not declared. C cluster parsing in `c/src/config_cluster.c` reads identity, quorum, security, resources, federation, transports, and node overrides, and rejects a quorum that asks for more votes than members. C capability parsing in `c/src/config_capability.c` reads service, resource, kind, and rights, and rejects a policy whose service was not declared. C diff in `c/src/config_diff.c` records changed areas and the nodes named by overrides. Rust sources were not changed.
- [ ] `crates/ghostos-embedded-script/` — C owns script limits, capability names, operation masks, duplicate resources, source size, and request admission. Rust retains the Rhai engine and request storage. Existing capability tests were executed.
- [ ] `crates/ghostos-heal/` — C owns health-service validation, duplicate detection, registration-id arithmetic, progress timestamp decisions, fault classification, and recovery slot selection, generation wrap, and replacement-process acceptance. Rust retains atomic loads and stores in their original order, trace emission, GhostFS checkpoints, and recovery runtime calls. Existing health and recovery behavior was executed; hot-swap and kernel-patch orchestration remain Rust.
- [ ] `crates/ghostos-inference/` — C protocol checks in `c/src/inference.c` validate model names, OpenAI completion JSON, and gRPC frame flags and lengths. C scheduling in `c/src/inference_schedule.c` registers models, reserves execution slots, and checks token checkpoints. Cache and journal commits remain. Rust sources were not changed.
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
- [ ] `crates/ghostos-wasm-script/` — C owns limit validation, entry and module-size checks, grant capacity, invalid and duplicate handles, operation masks, host invoke decisions, and fuel consumption. Rust retains the WebAssembly engine, store, and host callback. Existing runtime tests were executed.
- [ ] `crates/ghostos-webterm/` — web terminal and SSH.
- [ ] `crates/host-filesystems/` — FAT32, NTFS, and ext4 host filesystem support.
- [ ] `crates/http/` — HTTP implementation.
- [ ] `crates/init/` — initialization and fault domains.
- [ ] `crates/ipc/` — inter-process communication.
- [ ] `crates/legacy-pc-drivers/` — legacy PC block, PCI, storage, USB, and Ethernet drivers.
- [ ] `crates/llm-runtime/` — LLM inference runtime and KV cache.
- [ ] `crates/logd/` — C owns operator subscribe, unsubscribe, delivery filtering, operator-event selection, dropped-counter deltas, and remaining record budget. Rust retains journal writes, trace pops, and delivery callbacks. Existing daemon tests were executed.
- [ ] `crates/netd/` — network daemon.
- [ ] `crates/numa/` — C owns active bounded topology construction, sparse node lookup, per-kind fallback cursors, CPU/node selection, topology replacement, locality decisions, and saturating remote-memory counters in `c/src/numa.c`. Kernel scheduling, networking, storage, and platform I/O consumers use checked-layout Rust adapters. Both existing cases have C source; Rust const constructors/getters, build/adapters, behavior parity, and full removal remain.
- [ ] `crates/observability/` — profiling, telemetry, SLOs, and scaling.
- [ ] `crates/path-pattern/` — Active parsing, wildcard/class matching, UTF-8-width consumption, and unescaping now run in `c/src/path_pattern.c`. Filesystem, shell, and runtime consumers use a Rust type/FFI adapter. All seven existing contract/property cases have C source. Rust adapter/build-tool removal and behavior parity remain open.
- [ ] `crates/pkg/` — package management and signatures.
- [ ] `crates/platform-io/` — C owns active asynchronous queue metadata, round-robin cursors, generation-tagged token validation, submit/dispatch/complete/poll/cancel state transitions, pending counts, and I/O/media buffer/format/plane/access validation. Rust retains generic request/response payloads, public wrappers, NUMA logging, const helpers, and status conversion. All four existing queue model cases have C source; behavior parity, Rust build/adapters, and full removal remain.
- [ ] `crates/policy/` — C owns active caller-owned principal/object/binding storage, insertion and replacement, snapshot fingerprints, and all five read-only change simulations. Authentication, package, network, storage, update, and VM consumers build with checked-layout Rust adapters. Stable affected-entry ordering/de-duplication and source error precedence are retained; all three existing fixtures have C source. Rust public types/const constructors/report conversion/debug views, build adapters, behavior parity, and full removal remain.
- [ ] `crates/posix-compat/` — Active descriptor allocation, access checks, offsets, removal, and `/proc`, `/sys`, `/dev` path parsing now use C. Rust retains runtime file ownership, service calls, ABI dispatch, resolvers, and public types. Full cutover and behavior parity remain.
- [ ] `crates/power/` — Active thermal action/event decisions, generic event queue bookkeeping, cluster selection, frequency arithmetic, CPU idle selection, and device sleep decisions now use C. Rust retains typed storage, configuration validation, CPU/result conversion, metrics, const APIs, sensors/actuators, ACPI, and hot-plug orchestration. Full cutover and behavior parity remain.
- [ ] `crates/protocol/` — Active transport guards now store checked-layout C state and call C for construction, class validation, version negotiation, message limits, replay protection, authentication lockout, backpressure reserve/release, and disconnect/retry scheduling. HTTP, SDK, fabric, mesh, web terminal, and VM consumers build. Rust keeps public const limits/version helpers/getters/reconnect reset and typed error adapters; behavior parity, Rust build removal, and full cutover remain.
- [ ] `crates/ras/` — Active event history, sequence/cursor/drop state, saturating ECC/CXL/AER counters, and caller-owned poison quarantine/admission tables now use C. Generic capacities, newest-first iteration, node-scoped overlap checks, error ordering, zero-length directly constructed ranges, and debug overflow handling are preserved. C also owns budget prediction/threshold decisions, workload insertion/removal, eviction selection, and approved-eviction commits. C now also owns dirty-page tables, flush-state transitions, clean-slot cleanup, recovery reset/generation updates, and AER isolation decisions. Rust retains typed/const APIs, trace emission, controller/journal calls, last-reading/generation storage, marker conversion, and storage actions. The existing three RAS fixtures and a flush-state regression have C source. Behavior parity and full removal remain.
- [ ] `crates/rms/` — C owns lock-path validation, FNV resource ids, namespace checks, hexadecimal key paths, transaction slot reservation, and record-count overflow. Rust retains DLM acquire/release, GhostFS record and database I/O, and UTF-8 acceptance of the encoded path. Existing record and database tests were executed.
- [ ] `crates/runtime/` — core runtime.
- [ ] `crates/service-scale/` — Active membership validation/slot selection, generation/readiness fences, least-load target selection, session-close/handoff-preparation checks, handoff token matching, snapshot hashing, request deduplication/routing classification, effect reservation checks, completion/failure fences, checked/saturating counter arithmetic, snapshot aggregation, table lookup, and free-slot selection now use C. Rust retains typed tables/commits, session/handoff/request/effect commits, counter storage, and public APIs. Full cutover and behavior parity remain.
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

Micro-silo consumer progress on 2026-10-03: hardware protection and runtime
memory-table operations now call C. The caller-owned C API preserves generic
capacities without the standalone API's 16-slot cap, and reports arithmetic
overflow so the Rust adapter retains debug panic behavior. Const constructors,
accessors, and authorization remain Rust. `make c-library` and host kernel
library build passed. Build logs are in `temp/c-library-micro-silo.log` and
`temp/kernel-micro-silo-build.log`. The x86 freestanding kernel library also
built (`temp/kernel-micro-silo-x86-build.log`). Behavior parity and complete migration
remain open.

DMA/partition consumer progress on 2026-10-03: the DMA adapter now uses
`c/src/dma_state.c` for caller-owned mapping tables and staged updates.
Capability checks and IOMMU trait calls stay outside C, so rejected or panicking
backends leave allocation state unchanged. Error ordering, physical-range
constructor failures, first-free placement, wrapping IDs, and unmap authority
matching follow the prior Rust implementation. Host CPU partition operations
now use the same C implementation as target kernels. Host kernel, VM, and x86
kernel library builds passed; C DMA syntax passed for x86, AArch64, and RISC-V.
AArch64 and RISC-V kernel libraries also built; RISC-V used the existing
temporary Clang/LLVM verification wrapper. Their logs are
`temp/kernel-dma-partition-aarch64-build.log` and
`temp/kernel-dma-partition-riscv64-build.log`.
No behavior-parity execution is claimed; full migration remains open.
Logs are in `temp/c-library-dma-partition-build.log`,
`temp/kernel-dma-state-build.log`, `temp/vm-dma-partition-build.log`, and
`temp/kernel-dma-partition-x86-build.log`.

Allocator/driver/litmus consumer progress on 2026-10-03: active hot-object
allocator and driver resource policy now use C. Generic allocator views retain
CPU/node bounds, u8 node-count/fallback conversion, saturating probe counters,
reclaim accounting, and tracing semantics. Driver selection preserves role and
BAR matching, capacity-before-range ordering, missing-MMIO resources, and
partial capability grants. C driver selection is shared by standalone and
kernel consumers. Active litmus callers also use C seeded scheduling and all
six replay/minimization models; Rust retains typed reports and public-field
bounds panic handling. These are consumer cutovers, not full Rust removal.

`make c-library`, VM builds, and x86/AArch64/RISC-V kernel library builds passed
for the allocator and driver ports. RISC-V used the existing temporary LLVM
verification wrapper. Logs are in `temp/c-library-hot-allocator-build.log`,
`temp/vm-hot-allocator-build.log`, `temp/kernel-hot-allocator-*-build.log`,
`temp/c-library-driver-resources-build.log`, `temp/vm-driver-resources-build.log`,
and `temp/kernel-driver-resources-*-build.log`. C syntax checks also passed
for all three targets. Litmus C/VM and x86/AArch64/RISC-V library builds also
passed; logs are `temp/c-library-litmus-build.log`, `temp/vm-litmus-build.log`,
and `temp/kernel-litmus-*-build.log`. RISC-V again used the verification
wrapper. No new behavior-parity execution is claimed.

Persistence/boot-diagnostic consumer progress on 2026-10-03: active SYNREC01
container operations now use `c/src/persistence_records.c`. The C codec keeps
the existing 24-byte header, 56-byte boot slot, 1024-byte crash slot, payload
checksum, and legacy raw-record handling. Oversized writes stop before loading
storage; empty updates clear only their own slot. Active boot diagnostics now
use C transitions and the existing 56-byte diagnostic format. A separate raw
encoder preserves the Rust public structs' ability to encode unvalidated
status words; decoding retains status, reserved-byte, and checksum validation.
Rust retains public const APIs and storage orchestration.

`make c-library`, `cargo build -p ghostos-vm`, the full x86 kernel package,
and the AArch64 kernel library built. C syntax checks passed on all three
targets. Logs are `temp/c-library-boot-records-build.log`,
`temp/vm-boot-records-build.log`, `temp/kernel-boot-records-x86-build.log`,
`temp/kernel-boot-records-aarch64-build.log`, and
`temp/boot-records-target-syntax.log`. The RISC-V kernel library also built
(`temp/kernel-boot-records-riscv64-build.log`) using a recreated
`temp/clang-riscv64-verify.py` wrapper because the previous temporary wrapper
was absent. No behavior-parity execution is claimed; full migration remains
open.

RAS consumer progress on 2026-10-03: `c/src/ras.c` now owns active hardware
telemetry and poison tables through caller-owned storage. Rust keeps public
const constructors/getters, typed event/range conversion, and trace emission.
Sequence IDs wrap past zero, counter/drop updates saturate, and quarantine
checks alignment before overlap before capacity. Poison overlap arithmetic
follows Rust evaluation order and reports checked overflow for host panic
conversion. Budget/controller and persistent-memory operations remain Rust.

`make c-library` and `cargo build -p ghostos-ras` passed. The RAS library also
built for x86, AArch64, and RISC-V. RISC-V used the recreated temporary LLVM
verification wrapper. C syntax checks passed on all three targets. Logs are
`temp/c-library-ras-build.log`, `temp/ras-native-build.log`,
`temp/ras-x86-build.log`, `temp/ras-aarch64-build.log`,
`temp/ras-riscv64-build.log`, and `temp/ras-target-syntax.log`.
Behavior parity and the complete migration remain open.

RAS budget consumer progress on 2026-10-03: C now owns saturated thermal/power
prediction, threshold and throttle decisions, workload tables, eviction selection,
and post-approval state updates. The prediction horizon remains capped at
u32::MAX, signed division truncates toward zero, and positive/negative predictions
clamp exactly as before. Slot ordering, duplicate detection for inactive records,
zero-capacity tables, and critical-workload exclusion remain intact. Rust stores
the last reading before controller calls; throttle failures and partial eviction
failures retain the original state ordering. Controller trait calls stay outside
C, preserving host unwinding.

`make c-library` and host/x86/AArch64/RISC-V RAS library builds passed. RISC-V
used the temporary Clang/LLVM verification wrapper. C syntax checks passed for
all three targets. Logs are `temp/c-library-ras-budget-build.log`,
`temp/ras-budget-native-build.log`, `temp/ras-budget-x86-build.log`,
`temp/ras-budget-aarch64-build.log`, `temp/ras-budget-riscv64-build.log`, and
`temp/ras-budget-target-syntax.log`. Persistent-memory recovery and full
migration remain open; no behavior-parity execution is claimed.

RAS persistent-pool progress on 2026-10-03: dirty-page insertion/replacement,
full-table clean-slot reuse, flush selection/state updates, post-barrier cleanup,
recovery reset, generation rollover, and AER isolation decisions now use C.
Rust retains backend/journal trait calls, marker conversion, and public types.
Journal write precedes flushing; barrier success precedes clean-slot removal;
journal clear success precedes generation advancement. Zero-capacity tables
and directly supplied recovery-marker pages remain supported.

This port deliberately fixes a source-proven flush bug: the prior
`self.pages[index].ok_or(...)?.state = ...` statements changed copied
`DirtyPage` values. Stored entries stayed Dirty, so successful `flush_all`
could repeat the same page forever. C now changes the stored slot to Flushing
before calling the backend, then Clean or Failed afterward. A backend panic
leaves it Flushing. This correction is documented separately from migration
parity; no runtime verification was performed.

`make c-library` and host/x86/AArch64/RISC-V RAS library builds passed. RISC-V
used the temporary Clang/LLVM wrapper. All three existing RAS fixtures were
ported to `c/tests/ras_contracts.c`, alongside a focused flush-state regression,
and checked for C syntax only. The optional `ras-contracts` build target is
registered. Logs are `temp/c-library-ras-persistent-build.log`,
`temp/ras-persistent-native-build.log`, `temp/ras-persistent-x86-build.log`,
`temp/ras-persistent-aarch64-build.log`, `temp/ras-persistent-riscv64-build.log`,
`temp/ras-persistent-target-syntax.log`, and
`temp/ras-contract-source-syntax.log`. Full Rust removal and behavior parity
remain open.

POSIX compatibility progress on 2026-10-03: caller-owned C tables now handle
free-slot lookup, descriptor allocation, access checks, offsets, and removal.
Rust retains opaque runtime File values and service calls; failed closes keep
the descriptor. Generic and zero capacities remain supported. C also maps
/proc, /sys, and /dev paths to logical names, preserving mount matching,
component rejection, ASCII mapping, and validation/length error ordering.

`make c-library` and host/x86/AArch64/RISC-V POSIX library builds passed.
RISC-V used the temporary Clang/LLVM wrapper. Strict freestanding C syntax
checks passed for all three targets. Logs are
`temp/c-library-posix-build.log`, `temp/posix-native-build.log`,
`temp/posix-x86-build.log`, `temp/posix-aarch64-build.log`, and
`temp/posix-riscv-build.log`. No behavior-parity execution is claimed;
remaining ABI dispatch, resolver, and runtime integration still use Rust.

Power thermal progress on 2026-10-03: `c/src/thermal.c` now owns critical/hot/
passive priority, throttling percentages, saturated hysteresis recovery, and
change-event classification. C also owns generic event-ring cursors, counts,
saturating dropped-event accounting, and partial-drain selection. Rust keeps
typed event values, last readings, const validation/getters, and sensor and
actuator calls. Zero capacity and FIFO overwrite/drain order are preserved.
Manager updates still precede actuator calls, including actuator failures.

The C archive and host/x86/AArch64/RISC-V power library builds passed, along
with strict freestanding target syntax checks. RISC-V used the temporary
Clang/LLVM wrapper. Logs: `temp/c-library-thermal-build.log`,
`temp/power-native-build.log`, `temp/power-x86-build.log`,
`temp/power-aarch64-build.log`, `temp/power-riscv-build.log`.
ACPI, hot-plug orchestration, placement/frequency policy, full Rust removal,
and executed behavior parity remain open.

Power policy progress on 2026-10-03: `c/src/power_policy.c` now selects eligible
clusters by affinity, throttle, preferred-cluster fallback, and stable score
ties. Debug score overflow still panics before results/metrics are committed;
release arithmetic wraps as before. C also owns saturated frequency range,
load and throttle arithmetic, idle-state selection under wake/latency limits,
and device sleep selection under saturated elapsed time. Rust retains typed
state/configuration, first-CPU/result conversion, metrics, and hardware calls.
Frequency/device application order and failure commit points are unchanged.

`make c-library` and host/x86/AArch64/RISC-V power library builds passed.
Strict freestanding C syntax checks passed for all three targets. RISC-V used
the temporary Clang/LLVM wrapper. Logs are
`temp/c-library-power-policy-build.log`, `temp/power-policy-native-build.log`,
`temp/power-policy-x86-build.log`, `temp/power-policy-aarch64-build.log`, and
`temp/power-policy-riscv-build.log`. Full Rust removal and executed behavior
parity remain open.

Service-scale progress on 2026-10-03: `c/src/service_scale.c` now owns join/rejoin,
ready, drain, restart, checked-generation and readiness decisions over checked-
layout instance views. First-free allocation, duplicate/error precedence,
restart counter checks, and generation fences are preserved. C selects the
first least-loaded ready target excluding the current owner and computes the
wrapping FNV snapshot digest. Rust commits typed records only after decisions
succeed and retains session, handoff, request/effect state and counters.
The adapter creates a temporary stack view proportional to instance capacity.

The C archive and host/x86/AArch64/RISC-V service-scale library builds passed.
Strict freestanding C syntax checks passed for all three targets. RISC-V used
the temporary Clang/LLVM wrapper. Logs: `temp/c-library-service-scale-build.log`,
`temp/service-scale-native-build.log`, `temp/service-scale-x86-build.log`,
`temp/service-scale-aarch64-build.log`, `temp/service-scale-riscv-build.log`.
Full Rust removal and executed behavior parity remain open.

All six direct service-scale consumers (HTTP, web terminal, storage, package,
observability, and compiler daemon) also built successfully; existing kernel
warnings remain. Log: `temp/service-scale-consumer-build.log`.

Service-scale handoff progress on 2026-10-03: C now validates session close
state/in-flight counts and handoff preparation generation, state, sequence,
and in-flight checks. In-flight errors still take precedence over other
preparation failures. C also compares all stored/token handoff fields; Rust
checks the snapshot digest only after those fields match, preserving lazy
slice validation. Typed handoff commits, snapshot copying, counters, request/
effect transitions, and public APIs remain Rust.

The C archive, host/x86/AArch64/RISC-V scaling library, and all six direct
consumers built successfully. Strict freestanding C syntax checks passed for
all three targets; RISC-V used the temporary Clang/LLVM wrapper. Logs:
`temp/c-library-scale-handoff-build.log`,
`temp/service-scale-handoff-native-build.log`, `temp/scale-handoff-x86-build.log`,
`temp/scale-handoff-aarch64-build.log`, `temp/scale-handoff-riscv-build.log`,
`temp/scale-handoff-consumer-build.log`. Full removal and executed behavior
parity remain open.

Service-scale routing progress on 2026-10-03: C now checks invalid effect IDs,
existing-request conflicts, completed receipts, in-flight requests, retry
selection, completed-effect receipts, cross-request duplicate effects, and
reservation capacity. Existing-request precedence over effect lookup and
reservation checks before session validation remain intact. Rust converts
native decisions to public receipts, commits new/retried records, and retains
session/readiness checks and in-flight counters. Native request/effect views
use temporary stack space proportional to the generic ledger capacities.

The C archive and host/x86/AArch64/RISC-V service-scale library builds passed.
Strict freestanding C syntax checks passed for all three targets. RISC-V used
the temporary Clang/LLVM wrapper. Logs: `temp/c-library-scale-route-build.log`,
`temp/scale-route-native-build.log`, `temp/scale-route-x86-build.log`,
`temp/scale-route-aarch64-build.log`, `temp/scale-route-riscv-build.log`.
Full Rust removal and executed behavior parity remain open.

All six direct service-scale consumers also built successfully. Existing
kernel warnings remain. Log: `temp/scale-route-consumer-build.log`.

Service-scale counter progress on 2026-10-03: C now validates completion and
failure ownership/generation before request-state checks. It also owns checked
session/in-flight increments, checked retry-attempt increments, and saturated
session/in-flight decrements. Rust still commits typed records at each original
point. In particular, an open session is stored before its instance counter
increment; the instance in-flight increment precedes the session increment;
and handoff source decrement precedes target increment. Capacity failures keep
the original partial-update behavior rather than introducing transactional
changes during migration. Typed state storage, ledgers, snapshot aggregation,
and public APIs remain Rust.

The C archive and host/x86/AArch64/RISC-V scaling library builds passed.
Strict freestanding syntax checks passed for all three targets. RISC-V used
the temporary Clang/LLVM wrapper. Logs:
`temp/c-library-scale-counters-build.log`, `temp/scale-counters-native-build.log`,
`temp/scale-counters-x86-build.log`, `temp/scale-counters-aarch64-build.log`,
`temp/scale-counters-riscv-build.log`. Full removal and executed behavior parity
remain open.

All six direct service-scale consumers built successfully; existing kernel
warnings remain. Log: `temp/scale-counters-consumer-build.log`.

Service-scale snapshot progress on 2026-10-03: C now aggregates occupied
instances, ready/draining/restarting states, session and in-flight totals, and
completed effects. Checked-layout views preserve generic capacities and slot
order. Debug addition overflow returns to a Rust panic adapter; release totals
wrap. The snapshot still reports instance counters rather than recounting
session records, preserving reporting after partial capacity-failure commits.
Rust retains service-kind/result conversion and typed tables. Temporary views
use stack space proportional to instance and effect capacities.

The C archive and host/x86/AArch64/RISC-V scaling library builds passed.
Strict freestanding C syntax checks passed for all three targets. RISC-V used
the temporary Clang/LLVM wrapper. Logs: `temp/c-library-scale-snapshot-build.log`,
`temp/scale-snapshot-native-build.log`, `temp/scale-snapshot-x86-build.log`,
`temp/scale-snapshot-aarch64-build.log`, `temp/scale-snapshot-riscv-build.log`.
Full Rust removal and executed behavior parity remain open.

All six direct service-scale consumers also built successfully; existing kernel
warnings remain. Log: `temp/scale-snapshot-consumer-build.log`.

Service-scale lookup progress on 2026-10-03: C now performs first-match lookup
for instance/session/request IDs and effect receipts, plus first-free selection
for session/request/effect tables. Checked-layout slot views retain occupancy
separately from identifiers; holes and generic capacities are supported without
sentinel IDs. Rust retains typed payloads, receipt conversion, and original
commit/error ordering. Closed sessions and completed requests remain occupied,
as before; this port does not reclaim records. Views use temporary stack space
proportional to table capacity.

The C archive and host/x86/AArch64/RISC-V scaling library builds passed.
Strict freestanding C syntax checks passed for all three targets; RISC-V used
the temporary Clang/LLVM wrapper. Logs: `temp/c-library-scale-lookup-build.log`,
`temp/scale-lookup-native-build.log`, `temp/scale-lookup-x86-build.log`,
`temp/scale-lookup-aarch64-build.log`, `temp/scale-lookup-riscv-build.log`.
Full removal and executed behavior parity remain open.

All six direct service-scale consumers built successfully; existing kernel
warnings remain. Log: `temp/scale-lookup-consumer-build.log`.

Actor-runtime progress on 2026-10-03: `c/src/actors.c` now owns mailbox
validation, envelope identity checks, endpoint matching, first-free directory
and node-route selection, and spawn prechecks. Duplicate detection still
precedes capacity failure. Mailbox checks still reject a zero-length wrapped
range before alignment, authority, epoch, and expiry checks, and debug address
overflow still panics before those later checks. Remote spawn still reads the
clock only after a route is found. Rust retains actor traits, IPC and DSM
transport calls, typed endpoints, and directory commits after spawn or stop
returns. `cargo test -p ghostos-actors` passed both existing tests, and
`build/c/actor-contracts` passed. `make c-library`, the balancer consumer, and
host/x86/AArch64/RISC-V actor library builds passed. RISC-V used the temporary
Clang/LLVM wrapper because Apple Clang has no RISC-V codegen backend. Strict
freestanding syntax checks passed for `c/src/actors.c` on all three targets.
Logs: `temp/c-library-actors-build.log`, `temp/actors-test.log`,
`temp/actors-consumer-build.log`, `temp/actor-contract-build.log`,
`temp/actors-x86-build.log`, `temp/actors-aarch64-build.log`, and
`temp/actors-riscv64-build.log`. Full Rust removal and broader behavior parity
remain open.

Health and recovery progress on 2026-10-03: `c/src/heal.c` now owns service
validation, first-match duplicate detection, registration-id wrap, progress
timestamp decisions, fault classification, recovery slot selection, generation
wrap, and replacement-process acceptance. Duplicate detection still stops at
the first matching service. Fault loads still follow memory, heartbeat,
driver, then progress, and a memory fault still suppresses the later timeout
loads. Recovery still fences before clearing the process, and a rejected
replacement leaves the generation unchanged. Rust retains the atomic registry,
trace emission, GhostFS checkpoint calls, and recovery runtime calls. The four
existing heal tests passed, as did `build/c/heal-contracts`. `make c-library`,
the inspection consumer, and host/x86/AArch64/RISC-V heal library builds
passed. RISC-V used the temporary Clang/LLVM wrapper. Strict freestanding
syntax checks passed for `c/src/heal.c` on all three targets. Logs:
`temp/c-library-heal-build.log`, `temp/heal-test.log`,
`temp/heal-consumer-build.log`, `temp/heal-contract-build.log`,
`temp/heal-x86-build.log`, `temp/heal-aarch64-build.log`, and
`temp/heal-riscv64-build.log`. Hot-swap, kernel patch orchestration, full Rust
removal, and broader behavior parity remain open.

Record-management progress on 2026-10-03: `c/src/rms.c` now owns lock-path
validation, FNV-1a resource ids, namespace character checks, hexadecimal key
paths, transaction slot reservation, and checked record-count changes. Empty
keys still fail before length arithmetic, and a zero hash still becomes
resource id 1. Rust retains DLM acquire/release, GhostFS reads and writes, and
UTF-8 acceptance of the encoded path. The three existing RMS tests passed, as
did `build/c/rms-contracts`. `make c-library` and host/x86/AArch64/RISC-V RMS
library builds passed. RISC-V used the temporary Clang/LLVM wrapper. Strict
freestanding syntax checks passed for `c/src/rms.c` on all three targets.
Logs: `temp/c-library-rms-build.log`, `temp/rms-test.log`,
`temp/rms-contract-build.log`, `temp/rms-x86-build.log`,
`temp/rms-aarch64-build.log`, and `temp/rms-riscv64-build.log`. Full Rust
removal and broader behavior parity remain open.

Agent-bridge progress on 2026-10-03: `c/src/agent.c` now owns task-scope
validation, parent identity and epoch checks, right and transport coverage,
lifetime limits, checked expiry, reusable grant slots, nonce and revocation
wrap, consume authorization, run-right unions, active-grant counts, and
commit/discard decisions. Parent right and transport reads still happen only
after the earlier comparisons succeed, and parent expiry is still read only
after the lifetime addition succeeds. A failed consume still leaves the grant
in place. Rust retains signature and lease verification, script execution, and
GhostFS sandbox calls. Both existing attenuation tests passed, as did
`build/c/agent-contracts`. `make c-library` and host/x86/AArch64/RISC-V agent
library builds passed. RISC-V used the temporary Clang/LLVM wrapper. Strict
freestanding syntax checks passed for `c/src/agent.c` on all three targets.
Logs: `temp/c-library-agent-build.log`, `temp/agent-test.log`,
`temp/agent-contract-build.log`, `temp/agent-x86-build.log`,
`temp/agent-aarch64-build.log`, and `temp/agent-riscv64-build.log`. Full Rust
removal and broader behavior parity remain open.

Audit progress on 2026-10-03: `c/src/audit.c` now owns advisory identifier
checks, advisory and obsolescence slot selection, out-of-date version
ordering, finding de-duplication, and scan-budget validation. Duplicate
advisories and obsolete packages are still rejected before a full table is
reported. A finding that does not fit still leaves earlier findings from the
same package visit in place. Withdrawn advisories are still filtered before
finding insertion. Rust retains feed records, package iteration, and batch
scheduling. The audit package has no existing Rust tests; `build/c/audit-contracts`
passed. `make c-library`, the inspection consumer, and host/x86/AArch64/RISC-V
audit library builds passed. RISC-V used the temporary Clang/LLVM wrapper.
Strict freestanding syntax checks passed for `c/src/audit.c` on all three
targets. Logs: `temp/c-library-audit-build.log`, `temp/audit-test.log`,
`temp/audit-consumer-build.log`, `temp/audit-contract-build.log`,
`temp/audit-x86-build.log`, `temp/audit-aarch64-build.log`, and
`temp/audit-riscv64-build.log`. Patch workflow, full Rust removal, and broader
behavior parity remain open.

Log-daemon progress on 2026-10-03: `c/src/logd.c` now owns operator subscribe
and unsubscribe, level filtering, operator-event selection, saturating
dropped-counter deltas, and remaining record budget. An existing terminal is
still updated before a free slot is considered. Info events still stay off
the operator path unless their kind is operator, and a failed journal append
still happens before rotation. Rust retains journal writes, trace pops, and
delivery callbacks. All three existing log tests passed, as did
`build/c/logd-contracts`. `make c-library` and host/x86/AArch64/RISC-V log
library builds passed. RISC-V used the temporary Clang/LLVM wrapper. Strict
freestanding syntax checks passed for `c/src/logd.c` on all three targets.
Logs: `temp/c-library-logd-build.log`, `temp/logd-test.log`,
`temp/logd-contract-build.log`, `temp/logd-x86-build.log`,
`temp/logd-aarch64-build.log`, and `temp/logd-riscv64-build.log`. Journal
storage, full Rust removal, and broader behavior parity remain open.

Wasm-script progress on 2026-10-03: `c/src/wasm_script.c` now owns limit
validation, entry and module-size checks, grant capacity, invalid and
duplicate handles, operation masks, host invoke decisions, and saturating
fuel consumption. An empty entry is still rejected before the module-size
check, and an invalid operation is still rejected before capability lookup.
Rust retains the WebAssembly engine, store, linker, and host callback. Both
existing runtime tests passed, as did `build/c/wasm-script-contracts`.
`make c-library` and host/x86/AArch64/RISC-V Wasm-script library builds
passed. RISC-V used the temporary Clang/LLVM wrapper. Strict freestanding
syntax checks passed for `c/src/wasm_script.c` on all three targets. Logs:
`temp/c-library-wasm-script-build.log`, `temp/wasm-script-test.log`,
`temp/wasm-script-contract-build.log`, `temp/wasm-script-x86-build.log`,
`temp/wasm-script-aarch64-build.log`, and `temp/wasm-script-riscv64-build.log`.
Full Rust removal and broader behavior parity remain open.

Embedded-script progress on 2026-10-03: `c/src/embedded_script.c` now owns
script limits, capability-name checks, operation masks, duplicate resource
names, source-size checks, and request admission. Invalid operations are still
rejected before payload and capability scans, and a too-large payload is still
rejected before capability lookup. Rust retains the Rhai engine and request
storage. `build/c/embedded-script-contracts` passed, as did `make c-library`
and strict freestanding syntax checks for `c/src/embedded_script.c` on x86-64,
AArch64, and RISC-V. The Rust package tests and target library builds were
not run: Cargo could not download the Rhai dependency `ahash`. Logs:
`temp/c-library-embedded-script-build.log` and
`temp/embedded-script-contract-build.log`. Full Rust removal and broader
behavior parity remain open.

Debug-probe progress on 2026-10-03: `c/src/probes.c` is a C probe machine.
It checks that a program ends in halt, rejects backward and out-of-range
jumps, executes wrapping arithmetic, and stores emitted values in a ring that
counts overwritten records. The existing arithmetic case produces 42 and the
self-jump program is rejected. `build/c/probe-contracts` passed. `make
c-library` passed, and strict freestanding syntax checks passed for
`c/src/probes.c` on x86-64, AArch64, and RISC-V. No Rust sources were edited
and Cargo was not used. GDB framing, coredumps, and remote sessions remain.
Logs: `temp/c-library-probes-build.log` and `temp/probe-contract-build.log`.

GDB framing progress on 2026-10-03: `c/src/gdb.c` buffers partial remote
packets, checks the checksum, reads memory as hex, and returns `E03` when a
write is not permitted. A split `m0,4` packet produces `+$01020304#0a`, and
`M0,1:ff` under a read-only permit produces `+$E03#a8`.
`build/c/gdb-contracts` passed. `make c-library` passed, and strict
freestanding syntax checks passed for `c/src/gdb.c` on x86-64, AArch64, and
RISC-V. No Rust sources were edited and Cargo was not used. Register
commands, coredumps, and remote sessions remain. Logs:
`temp/c-library-gdb-build.log` and `temp/gdb-contract-build.log`.

Coredump progress on 2026-10-03: `c/src/coredump.c` validates core paths,
accepts only directories under `/cores`, and encodes `SYNCORE1` metadata and
16-byte page headers. The existing capture shape is process 3, panic, two
registers, and page bytes `9 8 7` at offset 16 of `PAGE-00000000`.
`build/c/coredump-contracts` passed. `make c-library` passed, and strict
freestanding syntax checks passed for `c/src/coredump.c` on x86-64, AArch64,
and RISC-V. No Rust sources were edited and Cargo was not used. Filesystem
transactions and remote debug sessions remain. Logs:
`temp/c-library-coredump-build.log` and `temp/coredump-contract-build.log`.

Remote-debug progress on 2026-10-03: `c/src/remote_debug.c` checks the
192-byte `SYCA` capability header, rejects a mismatched process or a zero
nonce before authorization, and selects debug, read, and write rights for
each operation. A permit still requires the session token to equal the
capability nonce and an external authorization result. `build/c/remote-debug-contracts`
passed. `make c-library` passed, and strict freestanding syntax checks passed
for `c/src/remote_debug.c` on x86-64, AArch64, and RISC-V. No Rust sources
were edited and Cargo was not used. Signature verification remains outside
this module. Logs: `temp/c-library-remote-debug-build.log` and
`temp/remote-debug-contract-build.log`.

Enclave progress on 2026-10-03: `c/src/enclave.c` rejects a zero measurement,
requires page-aligned ranges, returns a duplicate node before allocating a
slot, and treats a protected range as covering its exclusive end. A query
whose end would overflow is not protected. The four-page range from the
existing admission case is protected, and the range eight pages in is not.
`build/c/enclave-contracts` passed. `make c-library` passed, and strict
freestanding syntax checks passed for `c/src/enclave.c` on x86-64, AArch64,
and RISC-V. No Rust sources were edited and Cargo was not used. Attestation
quotes, capability provisioning, and encrypted frames remain. Logs:
`temp/c-library-enclave-build.log` and `temp/enclave-contract-build.log`.

Confidential-capability progress on 2026-10-03: `c/src/confidential_capability.c`
issues caller-owned records, rejects an empty subject before allocation, and
checks expiry before other authorization failures. Revoking every record
advances the epoch from 1 to 2 and makes the old record unauthorized.
`build/c/confidential-capability-contracts` passed. `make c-library` passed,
and strict freestanding syntax checks passed for
`c/src/confidential_capability.c` on x86-64, AArch64, and RISC-V. No Rust
sources were edited and Cargo was not used. Attestation quotes and encrypted
frames remain. Logs: `temp/c-library-confidential-capability-build.log` and
`temp/confidential-capability-contract-build.log`.

Confidential-fabric progress on 2026-10-03: `c/src/confidential_fabric.c`
seals a DSM page with the legacy key-derivation labels, opens it back to the
payload `secret page`, and encodes a fixed `SCF1` frame. Changing the last
tag byte fails authentication. A repeated 16-byte nonce is rejected as replay.
`build/c/confidential-fabric-contracts` passed. `make c-library` passed, and
strict freestanding syntax checks passed for `c/src/confidential_fabric.c` on
x86-64, AArch64, and RISC-V. No Rust sources were edited and Cargo was not
used. Attestation quotes remain. Logs:
`temp/c-library-confidential-fabric-build.log` and
`temp/confidential-fabric-contract-build.log`.

Attestation progress on 2026-10-03: `c/src/attestation.c` registers a node
before accepting a duplicate, requires a challenge before admission, and
reports expiry before a signature check. A quote signed with the stored key
is admitted at time 3 and rejected once the challenge expiry of 50 has passed.
A changed signature is rejected. `build/c/attestation-contracts` passed.
`make c-library` passed, and strict freestanding syntax checks passed for
`c/src/attestation.c` on x86-64, AArch64, and RISC-V. No Rust sources were
edited and Cargo was not used. Logs: `temp/c-library-attestation-build.log`
and `temp/attestation-contract-build.log`.

Backup progress on 2026-10-03: `c/src/backup.c` streams a `SYNBACK1`
volume header, one file record, and a `SYNBEND1` trailer. A zero byte budget
is rejected before the job changes. The streamed bytes include `old` and do
not include `new`, and the finished job reports one file.
`build/c/backup-contracts` passed. `make c-library` passed, and strict
freestanding syntax checks passed for `c/src/backup.c` on x86-64, AArch64,
and RISC-V. No Rust sources were edited and Cargo was not used. Filesystem
checkpoints and encrypted chunk recovery remain. Logs:
`temp/c-library-backup-build.log` and `temp/backup-contract-build.log`.

Backup-crypto progress on 2026-10-03: `c/src/backup_crypto.c` seals a
chunk with a 12-byte nonce and a 32-byte HMAC tag, then opens it back to
`old`. A chunk longer than 4096 bytes is rejected before encryption. Changing
the last tag byte fails authentication, and a short encoding is rejected
before the tag check. `build/c/backup-crypto-contracts` passed. `make
c-library` passed, and strict freestanding syntax checks passed for
`c/src/backup_crypto.c` on x86-64, AArch64, and RISC-V. No Rust sources were
edited and Cargo was not used. Filesystem checkpoints remain. Logs:
`temp/c-library-backup-crypto-build.log` and
`temp/backup-crypto-contract-build.log`.

Configuration-signature progress on 2026-10-03: `c/src/config_signature.c`
derives a 16-byte key id, rejects a second copy of the same key, and checks
the target node before looking up the signer. An empty cluster is a target
mismatch. A changed signature is rejected after the key is found.
`build/c/config-signature-contracts` passed. `make c-library` passed, and
strict freestanding syntax checks passed for `c/src/config_signature.c` on
x86-64, AArch64, and RISC-V. No Rust sources were edited and Cargo was not
used. Configuration parsing and activation remain. Logs:
`temp/c-library-config-signature-build.log` and
`temp/config-signature-contract-build.log`.

Reconfigure progress on 2026-10-03: `c/src/reconfigure.c` rejects a full
history before checking the revision, rejects a stale expected revision, and
rejects an update that repeats the active revision. Commit records the
previous revision, and rollback returns that revision. A zero-capacity
history is full. `build/c/reconfigure-contracts` passed. `make c-library`
passed, and strict freestanding syntax checks passed for `c/src/reconfigure.c`
on x86-64, AArch64, and RISC-V. No Rust sources were edited and Cargo was not
used. Configuration parsing remains. Logs:
`temp/c-library-reconfigure-build.log` and
`temp/reconfigure-contract-build.log`.

Configuration-parser progress on 2026-10-03: `c/src/config_parser.c` reads
the system schema and revision. Hexadecimal `0x1` is schema 1. Schema 99 is
unsupported after the file is read, an unknown key fails while reading, a
repeated schema is a duplicate, a missing schema is reported after the file,
and revision 0 is invalid. Comments are ignored. `build/c/config-parser-contracts`
passed. `make c-library` passed, and strict freestanding syntax checks passed
for `c/src/config_parser.c` on x86-64, AArch64, and RISC-V. No Rust sources
were edited and Cargo was not used. Service, network, and cluster sections
remain. Logs: `temp/c-library-config-parser-build.log` and
`temp/config-parser-contract-build.log`.

Service-parser progress on 2026-10-03: `c/src/config_parser.c` now reads
`[[service]]` records. The init service keeps image `0x44`, kind system,
restart on-failure, and the default enabled state. A zero image is invalid
before a duplicate name would be reported. `build/c/config-service-contracts`
passed. `make c-library` passed, and strict freestanding syntax checks passed
for `c/src/config_parser.c` on x86-64, AArch64, and RISC-V. No Rust sources
were edited and Cargo was not used. Network and cluster sections remain.

Network-parser progress on 2026-10-03: `c/src/config_network.c` reads a quoted
hostname, interface address and MTU, and a route. The eth0 sample keeps MTU
1500 and the default metric 100. A route that names an undeclared interface is
invalid. `build/c/config-network-contracts` passed. `make c-library` passed,
and strict freestanding syntax checks passed for `c/src/config_network.c` on
x86-64, AArch64, and RISC-V. No Rust sources were edited and Cargo was not
used. Cluster sections remain. Logs: `temp/c-library-config-network-build.log`,
`temp/config-network-contract-build.log`.
Logs: `temp/c-library-config-service-build.log` and
`temp/config-service-contract-build.log`.

Cluster-parser progress on 2026-10-03: `c/src/config_cluster.c` reads cluster
identity, quorum, security, resources, federation, transports, and node
overrides. The sample keeps id `0x1234`, hybrid discovery, attested admission,
two of three votes, one ethernet transport, and a node 2 heartbeat of 100000.
A quorum that requires more votes than members is invalid.
`build/c/config-cluster-contracts` passed. `make c-library` passed, and strict
freestanding syntax checks passed for `c/src/config_cluster.c` on x86-64,
AArch64, and RISC-V. No Rust sources were edited and Cargo was not used.
Capability sections remain. Logs: `temp/c-library-config-cluster-build.log`,
`temp/config-cluster-contract-build.log`.

Capability-parser progress on 2026-10-03: `c/src/config_capability.c` reads a
capability policy. The init policy on `SYS$BOOT` keeps file kind and rights
value 5 for read and execute. A policy whose service was not declared is
invalid. `build/c/config-capability-contracts` passed. `make c-library` passed,
and strict freestanding syntax checks passed for `c/src/config_capability.c` on
x86-64, AArch64, and RISC-V. No Rust sources were edited and Cargo was not
used. Configuration diff remains. Logs: `temp/c-library-config-capability-build.log`,
`temp/config-capability-contract-build.log`.

Config-diff progress on 2026-10-03: `c/src/config_diff.c` compares two parsed
cluster views. Raising required votes from 2 to 3 records the quorum area and
affected node 2. An identical pair records nothing, and a zero change slot
returns capacity. `build/c/config-diff-contracts` passed. `make c-library`
passed, and strict freestanding syntax checks passed for `c/src/config_diff.c`
on x86-64, AArch64, and RISC-V. No Rust sources were edited and Cargo was not
used. Logs: `temp/c-library-config-diff-build.log`,
`temp/config-diff-contract-build.log`.

Inference-protocol progress on 2026-10-03: `c/src/inference.c` checks model
names, decodes an OpenAI completion request, and encodes a finished chat
completion. The tiny request keeps max tokens 4 and streaming. A zero token
limit is invalid, an unknown path is unsupported, and a gRPC frame with a
nonzero flag or a mismatched length is rejected.
`build/c/inference-contracts` passed. `make c-library` passed, and strict
freestanding syntax checks passed for `c/src/inference.c` on x86-64, AArch64,
and RISC-V. No Rust sources were edited and Cargo was not used. Service
scheduling remains. Logs: `temp/c-library-inference-build.log`,
`temp/inference-contract-build.log`.
