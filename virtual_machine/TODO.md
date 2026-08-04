# TODO.md: Virtual Machine Project

A lightweight virtual machine implementation in Rust designed to serve as a test environment for booting SynOS.

---

## 1. Core VM Architecture
- [x] **CPU Emulation Core**
  - [x] Implement x86_64 instruction decoder and executor
  - [x] Add support for protected mode and long mode transitions
  - [x] Implement exception/interrupt handling (IDT, IRQ vectors)
  - [x] Add CPU state management (registers, flags, segment registers)

- [x] **Memory Management Unit (MMU)**
  - [x] Implement physical memory allocator (frame-based)
  - [x] Build page table manager (PML4, PDPT, PD, PT levels)
  - [x] Add memory-mapped I/O support for devices
  - [x] Implement memory protection and access validation

### 2. Hardware Emulation

- [x] **Chipset & Bus**
  - [x] Implement PCI Express host bridge
  - [x] Add PCI bus enumeration and configuration space
  - [x] Build interrupt controller (APIC) emulation
    - [x] Memory-mapped xAPIC register file (0xFEE0_0000, 4 KiB)
    - [x] IA32_APIC_BASE MSR (0x1B) routing (wrmsr/rdmsr)
    - [x] IRR/ISR/TMR priority queues with TPR/PPR filtering
    - [x] EOI, ICR self-IPI delivery, six-entry LVT (timer, thermal,
          perfmon, LINT0, LINT1, error)
    - [x] Count-down timer with programmable divide configuration
    - [x] IF-gated and STI-shadow interrupt delivery in the VM loop
  - [x] Implement HPET timer and PIT
    - [x] 8254 PIT with three counters on I/O ports 0x40-0x43
    - [x] PIT modes 0-5, count latching, read-back, BCD
    - [x] PIT channel 0 wired to APIC vector 0x20 (IRQ0)
    - [x] HPET with 64-bit counter, 32 timers, periodic/one-shot mode
    - [x] HPET at ACPI base 0xFED0_0000 with legacy route to APIC
    - [x] Both driven from the VM host-time loop

- [x] **Storage Controllers**
  - [x] AHCI controller emulation for SATA drives
  - [x] NVMe controller emulation for PCIe devices
  - [x] Block device image file parser (RAW, VHD, QCOW2)
  - [x] DMA transfer implementation

- [x] **VM Disk Attachment and Management**
  - [x] Define a disk specification with a stable disk ID, role (`system` or `data`), controller, bus/slot, image path, format, capacity, and read-only mode.
  - [x] Add disk specifications to `VmConfig` so disks are attached while the VM is created, before firmware and SynOS boot.
  - [x] Provide one shared attachment path for AHCI, NVMe, and virtio-blk instead of requiring callers to open an image and reach into a controller manually.
  - [x] Reject missing, malformed, unsupported, duplicate, already-locked, and incorrectly sized images with actionable VM startup errors.
  - [x] Prevent the same backing image from being attached to multiple writable disks or multiple VMs at the same time.
  - [x] Define deterministic disk enumeration and guest-visible identity across VM restarts and controller types.
  - [x] Support explicit disk flush, sync, detach, and close behavior during normal shutdown, reboot, reset, and VM errors.
  - [x] Define read-only, copy-on-write, and disposable disk modes for safe inspection and test runs.

- [x] **SynOS System-Disk Provisioning**
  - [x] Define the system-disk layout: boot metadata, SynOS kernel and initrd, SynFS system volume, persistent settings, and reserved space for future updates.
  - [x] Add a disk-creation path that creates a new raw or supported formatted image with validated size, alignment, and format options.
  - [x] Build an idempotent installer/provisioner that writes SynOS, required system files, and default settings onto the system disk.
  - [x] Store the selected boot arguments, machine identity, network identity, capabilities, and system configuration on the disk rather than only in host-side VM arguments.
  - [x] Make provisioning atomic and restartable, with generation markers, checksums, and recovery for interrupted installation.
  - [x] Refuse accidental overwrite of an existing disk unless the caller explicitly requests replacement or reformatting.
  - [x] Validate the system-disk manifest and SynFS volume before boot, and report missing, stale, incompatible, or corrupt installation state.

- [x] **Boot SynOS From the Attached Disk**
  - [x] Teach BIOS and UEFI boot flows to discover the configured system disk and load SynOS from its on-disk boot artifacts.
  - [x] Define boot-source precedence when host `--kernel`/`--initrd` arguments and an installed system disk are both present.
  - [x] Load persistent settings from the disk before starting user-space services, with safe defaults when optional settings are absent.
  - [x] Mount the installed SynFS system volume as the SynOS root and keep system, package, log, user-data, and temporary storage roles separate.
  - [x] Preserve settings and filesystem changes across VM shutdown, reboot, and a later VM created from the same disk.
  - [x] Add system-disk version checks and an upgrade/migration path for changes to the on-disk layout or SynOS installation.

- [x] **Disk CLI and Operational Controls**
  - [x] Add VM CLI options for `--disk`, `--system-disk`, `--disk-controller`, `--disk-format`, `--disk-size`, `--read-only`, and create-if-missing behavior.
  - [x] Add commands or inspection output to list attached disks, roles, formats, capacities, persistence mode, health, and guest-visible identifiers.
  - [x] Add a validate/provision workflow that can prepare a disk without booting a VM and can inspect an installed disk without modifying it.
  - [x] Make disk paths explicit, canonical, and scoped to the VM configuration; avoid silently creating or selecting a host disk.
  - [x] Add locking and ownership metadata so concurrent VM launches fail safely and stale locks can be diagnosed and recovered.
  - [x] Document disk lifecycle examples for creating a new SynOS VM, reopening an existing VM, attaching a data disk, and using a read-only clone.

- [x] **Disk Reliability and Validation**
  - [x] Persist SynOS shell filesystem state on writable persistent disks across shutdown, reboot, and VM recreation.
  - [x] Test attachment and boot with RAW, fixed VHD, and QCOW2 images through AHCI, NVMe, and virtio-blk where supported.
  - [x] Test persistence by installing SynOS and settings, rebooting, creating files, recreating the VM, and verifying the same state is loaded.
  - [x] Test power-loss and interrupted-flush recovery for the boot metadata, settings store, and SynFS volume.
  - [x] Test read-only and copy-on-write behavior so base system disks cannot be mutated accidentally.
  - [x] Test invalid images, truncated images, corrupt metadata, out-of-range I/O, full disks, unsupported formats, and controller reset during I/O.
  - [x] Add end-to-end BIOS and UEFI coverage proving a newly provisioned system disk boots SynOS without host-provided kernel or initrd paths.
  - [x] Document the system-disk format, provisioning contract, backup/restore expectations, and compatibility policy.

- [x] **Networking**
  - [x] Intel e1000 NIC emulation
  - [x] virtio-net device emulation
  - [x] Packet buffer management
  - [x] MAC address handling

- [x] **Display**
  - [x] VGA text mode emulator
  - [x] VESA framebuffer emulator
  - [x] UEFI Graphics Output Protocol (GOP)
  - [x] Simple cursor and palette management

### 3. Firmware & Boot Support

- [x] **BIOS Implementation**
    - [x] 16-bit real mode entry point
  - [x] POST (Power-On Self Test) sequence
  - [x] MBR boot sector loading
  - [x] INT 10h, INT 13h, INT 15h BIOS services

- [x] **UEFI Implementation**
  - [x] UEFI firmware initialization
  - [x] EFI application loading
  - [x] UEFI boot services (LoadImage, StartImage, etc.)
  - [x] Runtime services (GetVariable, SetVariable, etc.)

- [x] **Boot Integration**
  - [x] SynOS kernel image loading
  - [x] Multiboot specification support
  - [x] Kernel entry point handoff
  - [x] Boot parameter passing (initrd, cmdline, etc.)

### 4. Device Drivers (Guest-side)

- [x] **Virtio Devices**
  - [x] Virtio-blk (block device)
  - [x] Virtio-net (network)
  - [x] Virtio-console (serial output)
  - [x] Virtio-rng (random number generator)

- [x] **Guest Utilities**
  - [x] Serial port output for debugging
  - [x] Keyboard input handling
  - [x] Mouse input (PS/2 or USB)
  - [x] Debug port (COM1/16550) emulation

### 5. Testing & Integration

- [x] **SynOS Integration**
  - [x] Verify SynOS kernel boots successfully
  - [x] Test basic kernel functionality (scheduler, IPC)
  - [x] Validate memory management (paging, capabilities)
  - [x] Test inter-process communication

- [x] **Test Environments**
  - [x] QEMU integration tests
  - [x] Network connectivity tests
  - [x] Storage I/O performance tests
  - [x] Multi-CPU SMP boot tests

### 6. Performance & Optimization

- [x] **Execution Engine**
  - [x] Dynamic binary translation (optional)
  - [x] JIT compilation for hot loops
  - [x] Translation cache management
  - [x] Execution profiling hooks

- [x] **Memory Efficiency**
  - [x] Copy-on-write for memory pages
  - [x] Memory ballooning support
  - [x] Large page support (2MB, 1GB)
  - [x] Memory overcommit handling

---

## 7. CLI & Management

- [x] **Command-Line Interface**
  - [x] VM configuration via CLI arguments
  - [x] Snapshot save/restore functionality
  - [x] Live migration support (checkpoint-based transfer)
  - [x] Monitor console access

- [x] **Interactive SynOS Terminal**
  - [x] Define the terminal contract: use the guest COM1 serial console as the
        first interactive path, with VGA/framebuffer output remaining available
        for future graphical terminals.
  - [x] Add terminal CLI options for enabling/disabling the terminal, choosing
        the serial port, and running non-interactively for scripts and CI.
  - [x] Detect whether stdin/stdout are attached to a TTY. Keep pipe and file
        execution usable without raw-terminal setup.
  - [x] Put the host terminal in raw, non-canonical mode so every key reaches
        the guest immediately. Save and restore the original settings.
  - [x] Restore terminal settings on normal exit, Ctrl-C, EOF, VM error, and
        panic. Never leave the host shell in raw mode.
  - [x] Read host stdin without blocking the VM execution loop. Forward bytes
        to the emulated COM1 receive FIFO through `Vm::queue_serial_input`.
  - [x] Handle Enter, Backspace, Tab, Ctrl-C, Ctrl-D, Escape sequences, and
        EOF. Define which controls stay host-side and which reach SynOS.
  - [x] Keep guest serial output connected to host stdout. Flush promptly and
        prevent input handling from corrupting displayed output.
  - [x] Add ANSI/VT pass-through or a clear policy so prompts, colors, cursor
        movement, and backspace behavior display correctly.
  - [x] Make the VM event loop poll stdin, devices, timers, and APIC interrupts
        while the guest executes or waits in `HLT`.
  - [x] Wake a halted guest when an accepted serial or keyboard interrupt
        arrives. Do not treat guest `HLT` as process shutdown.
  - [x] Add a clean guest shutdown path for poweroff and reboot, while Ctrl-C
        is delivered to the guest shell without stopping the VM.
  - [x] Add PS/2 keyboard injection for guests that do not use
        `console=serial0`.
  - [x] Add a terminal-session abstraction owning stdin polling, input
        translation, output flushing, terminal state, and cleanup.
  - [x] Keep terminal code outside device emulation. It may call public VM
        input/output APIs but must not reach into CPU internals.
  - [x] Add manual smoke checks: boot to the prompt, type `help`, run a
        command, verify output, test Backspace and Ctrl-C, leave the prompt
        idle in `HLT`, then wake it with input.
  - [x] Add automated CLI checks for piped input/output, TTY cleanup after an
        error, EOF handling, and guest shutdown.
  - [x] Document the interactive command and explain interactive versus
        bounded `--steps` runs.

- [x] **Snapshot & State**
  - [x] VM state serialization
  - [x] Checkpoint/restore implementation
  - [x] Diff-based snapshotting
  - [x] Snapshot chain management

---

## 8. Documentation & Examples

- [x] **Developer Documentation**
  - [x] Architecture overview
  - [x] Device emulation guide
  - [x] Adding new device support
  - [x] Performance tuning guide

- [x] **SynOS Boot Examples**
  - [x] Minimal boot configuration
  - [x] Multi-core boot setup
  - [x] Network boot via virtio-net
  - [x] Storage boot via AHCI/NVMe

---

## 9. Future Enhancements

- [x] **Hardware Acceleration**
  - [x] KVM host interface and API validation on Linux
  - [x] HAXM device integration (Intel)
  - [x] HVF backend selection (Apple Silicon/macOS)
  - [x] WHPX backend selection (Windows)

- [x] **Guest Features**
  - [x] Guest agent communication
  - [x] Time synchronization (PV clock)
  - [x] Shutdown/reboot notifications
  - [x] Memory hot-plug support

- [ ] **Cluster Testing**
  - [ ] Multi-node VM orchestration
  - [ ] Network simulation for clustering
  - [ ] CXL fabric emulation
  - [ ] Fault injection for testing

---

## 10. Test Completion Plan

The VM is the SynOS test machine. Every device and every public VM API needs direct tests before it can be used as proof for a SynOS feature.

### 10.1 Test Foundation

- [x] Split tests into fast unit tests, VM integration tests, CLI tests, QEMU tests, cluster tests, performance tests, and hardware-accelerated tests.
- [x] Add deterministic VM fixtures for CPU state, guest memory, page tables, PCI config space, disks, packets, interrupts, clocks, serial input, terminal input, and boot images.
- [x] Add reusable fake devices and failure injection for short I/O, DMA overrun, invalid descriptors, dropped interrupts, delayed timers, reset during I/O, and device removal.
- [x] Add cleanup guards for temporary disk images, child processes, Unix sockets, QMP sessions, serial logs, and terminal settings.
- [x] Add golden files for decoded instructions, firmware tables, boot handoff data, device registers, snapshots, serial output, and terminal output.
- [x] Add a VM test inventory mapping each source module and public API to its tests.

### 10.2 CPU, Memory, and Execution Tests

- [ ] Test decoder prefixes, REX, ModR/M, SIB, displacement, immediates, RIP-relative addressing, segment overrides, far operands, and malformed byte streams.
- [ ] Test executor arithmetic, flags, branches, stack operations, calls/returns, string operations, control/debug registers, MSRs, exceptions, and unsupported instructions.
- [ ] Test real, protected, compatibility, and long mode transitions, privilege levels, segment limits, page faults, interrupt gates, IF masking, STI shadow, and HLT wakeup.
- [ ] Test register reset values, instruction pointer progression, exception state, interrupt priority, and deterministic step limits.
- [ ] Test physical frame allocation/reuse, paging levels, permissions, COW, large pages, MMIO, unaligned access, out-of-range access, and memory statistics.
- [ ] Test execution engine profiles, block boundaries, translation cache hits/misses/invalidation, JIT fallback, max-step termination, and panic/error cleanup.

### 10.3 Device Tests

- [ ] Test PCI host bridge legacy ports, ECAM, device enumeration, BAR sizing, config writes, absent devices, and invalid bus/device/function values.
- [ ] Test APIC/PIC register behavior, MSR/MMIO coherence, IRR/ISR/TMR priority, TPR/PPR filtering, EOI, IPI, LVTs, timer modes, and interrupt routing.
- [ ] Test PIT modes, latching, read-back, BCD, divisor limits, IRQ0 delivery, and host-time progression.
- [ ] Test HPET counter, comparator, periodic/one-shot mode, legacy routing, enable/disable, overflow, and timer interrupt delivery.
- [ ] Test serial 16550 registers, divisor latch, FIFO, line status, transmit output, receive input, IRQ enable/priority, reset, and overrun.
- [ ] Test PS/2 keyboard and mouse queues, controller commands, self-tests, enable/disable, status bits, IRQ vectors, and input overflow.
- [ ] Test power control, shutdown, reboot, reset, repeated commands, and invalid power states.
- [ ] Test VGA text memory, cursor, palette, mode changes, VESA framebuffer, GOP modes, pixel formats, bounds, and reset.
- [ ] Test AHCI and NVMe identification, command setup, DMA reads/writes, queue limits, interrupts, flush, invalid PRDT/PRP, reset, and I/O failure.
- [ ] Test Virtio feature negotiation, queue setup, descriptor chains, indirect descriptors, readable/writable buffers, notifications, status/reset, and malformed chains.
- [ ] Test Virtio block flush/read/write, console input/output, RNG bounds, and net transmit/receive behavior.
- [ ] Test E1000 registers, descriptor rings, MAC filtering, transmit/receive, interrupts, reset, and invalid descriptors.
- [ ] Test raw, VHD, and QCOW2 image parsing, sector bounds, sparse/unsupported images, persistence, flush, and corruption errors.

### 10.4 Firmware, Boot, and SynOS Tests

- [ ] Test BIOS POST, real-mode entry, MBR load, INT 10h/13h/15h services, bad sectors, missing boot code, and handoff failure.
- [ ] Test UEFI tables, memory map, boot services, runtime variables, image loading, StartImage, invalid images, and service errors.
- [ ] Test Multiboot and SynOS boot information, kernel/initrd/cmdline placement, framebuffer data, entry-point validation, and memory overlap rejection.
- [ ] Test `Vm::new` and `Vm::with_config` defaults, serial enable/disable, custom ports, memory size, firmware, SMP count, boot args, and step limits.
- [ ] Test VM booting a minimal SynOS image, serial bootstrap, shell prompt, scheduler progress, IPC, capability checks, page mapping, SynFS mount, file read/write, shutdown, and reboot.
- [ ] Test BIOS and UEFI boot with one and multiple virtual CPUs and assert serial evidence plus bounded termination.

### 10.5 Snapshot, Terminal, Network, and Integration Tests

- [ ] Test snapshot serialization, restore, snapshot IDs, chain order, diffs, memory/device state, corrupted data, version mismatch, and partial restore.
- [ ] Test terminal input translation for printable bytes, Enter, Backspace, Tab, Ctrl-C, Ctrl-D, Escape sequences, EOF, and PS/2 fallback.
- [ ] Test terminal raw-mode ownership, restoration on success/error/panic/Ctrl-C/EOF, output flushing, resize, ANSI pass-through, and guest HLT wakeup.
- [ ] Test loopback hub routing, port isolation, MAC addresses, packet queues, backpressure, dropped frames, and deterministic network faults.
- [ ] Test VM storage/network device combinations used by SynOS boot and shell workflows.
- [ ] Test QMP lifecycle, serial capture, timeout handling, guest poweroff, QEMU crash, missing image, missing executable, and stale socket cleanup.
- [ ] Test QEMU filesystem workflows for directory, create, type, default directory, edit, link, delete, wildcard, version, snapshot, and failure cases.
- [ ] Test remote terminal workflows for prompt, command input, output, resize, reconnect, guest shutdown, and cleanup.

### 10.6 Cluster and Fault Tests

- [ ] Add a deterministic multi-VM network harness with controllable latency, loss, duplication, reordering, partitions, and reconnection.
- [ ] Add CXL and shared-memory device fixtures that model discovery, mapping, access, migration, and hot removal.
- [ ] Test two-node and multi-node SynOS boot, discovery, heartbeat, page fetch, coherence, migration, failover, fencing, and rejoin.
- [ ] Kill one node during IPC, filesystem commit, memory fetch, inference, and cluster membership changes; assert safe recovery and stable status.
- [ ] Test split-brain, stale epochs, duplicate node IDs, lost quorum, delayed heartbeats, corrupted shared memory, and transport recovery.
- [ ] Make cluster tests opt-in and bounded, with serial logs, QMP traces, network traces, and failure-injection metadata saved as evidence.

### 10.7 VM Quality Gates

- [ ] Make `cargo test` from the repository root include the VM tests, either by joining this crate to the root workspace or by using a tested top-level Cargo runner.
- [ ] Define the fast all-tests command for deterministic VM and SynOS tests, then define the opt-in full-validation command for QEMU, cluster, hardware, performance, fuzz, and soak tests.
- [ ] Require every VM source module and public API to have a named test in the VM inventory.
- [ ] Require every emulated device to have register/configuration, normal I/O, reset, interrupt, malformed input, and failure coverage.
- [ ] Require every SynOS boot path to have a VM or QEMU test with serial evidence.
- [ ] Require every VM bug fix to add a deterministic regression test.
- [ ] Add VM coverage reporting, mutation testing for device boundaries, nightly fuzzing for decoder/devices/images, and cross-platform CI.
- [ ] Publish separate pass/fail/skip status for unit, integration, QEMU, cluster, performance, and hardware-accelerated tests.
- [ ] Do not mark a VM feature complete until its tests pass with clean resource and terminal cleanup.

---

## Project Structure

```
virtual_machine/
├── src/
│   ├── main.rs              # VM entry point
│   ├── cpu/                 # CPU emulation
│   │   ├── mod.rs
│   │   ├── decoder.rs
│   │   └── executor.rs
│   ├── memory/              # MMU implementation
│   │   ├── mod.rs
│   │   ├── allocator.rs
│   │   └── paging.rs
│   ├── devices/             # Device emulations
│   │   ├── mod.rs
│   │   ├── apic.rs          # Local APIC (xAPIC)
│   │   ├── pit.rs           # 8254 Programmable Interval Timer
│   │   ├── hpet.rs          # High Precision Event Timer
│   │   ├── pci.rs
│   │   ├── ahci.rs
│   │   ├── e1000.rs
│   │   ├── vga.rs
│   │   └── virtio.rs
│   ├── firmware/            # BIOS/UEFI
│   │   ├── mod.rs
│   │   ├── bios.rs
│   │   └── uefi.rs
│   ├── boot/                # Boot loading
│   │   ├── mod.rs
│   │   └── loader.rs
│   └── lib.rs               # Public API
├── Cargo.toml
└── README.md
```

---

## Getting Started

```bash
# From the repository root, build the BIOS kernel
./scripts/build-bios-image.sh

# From virtual_machine/, build the VM
cargo build --release

# Run with SynOS kernel
./target/release/synos-vm --kernel ../build/bios/kernel.bin \
    --memory 128M \
    --append "console=serial0"
```
