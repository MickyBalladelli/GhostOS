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
  - [ ] Snapshot save/restore functionality
  - [ ] Live migration support (future)
  - [ ] Monitor console access

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

- [ ] **Hardware Acceleration**
  - [ ] KVM hypercall interface
  - [ ] HAXM integration (Intel)
  - [ ] HVF integration (Apple Silicon)
  - [ ] WHPX integration (Windows)

- [ ] **Guest Features**
  - [ ] Guest agent communication
  - [ ] Time synchronization (PV clock)
  - [ ] Shutdown/reboot notifications
  - [ ] Memory hot-plug support

- [ ] **Cluster Testing**
  - [ ] Multi-node VM orchestration
  - [ ] Network simulation for clustering
  - [ ] CXL fabric emulation
  - [ ] Fault injection for testing

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
