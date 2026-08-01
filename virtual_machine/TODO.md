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

- [ ] **SynOS Integration**
  - [ ] Verify SynOS kernel boots successfully
  - [ ] Test basic kernel functionality (scheduler, IPC)
  - [ ] Validate memory management (paging, capabilities)
  - [ ] Test inter-process communication

- [ ] **Test Environments**
  - [ ] QEMU integration tests
  - [ ] Network connectivity tests
  - [ ] Storage I/O performance tests
  - [ ] Multi-CPU SMP boot tests

### 6. Performance & Optimization

- [ ] **Execution Engine**
  - [ ] Dynamic binary translation (optional)
  - [ ] JIT compilation for hot loops
  - [ ] Translation cache management
  - [ ] Execution profiling hooks

- [ ] **Memory Efficiency**
  - [ ] Copy-on-write for memory pages
  - [ ] Memory ballooning support
  - [ ] Large page support (2MB, 1GB)
  - [ ] Memory overcommit handling

---

## 7. CLI & Management

- [ ] **Command-Line Interface**
  - [ ] VM configuration via CLI arguments
  - [ ] Snapshot save/restore functionality
  - [ ] Live migration support (future)
  - [ ] Monitor console access

- [ ] **Snapshot & State**
  - [ ] VM state serialization
  - [ ] Checkpoint/restore implementation
  - [ ] Diff-based snapshotting
  - [ ] Snapshot chain management

---

## 8. Documentation & Examples

- [ ] **Developer Documentation**
  - [ ] Architecture overview
  - [ ] Device emulation guide
  - [ ] Adding new device support
  - [ ] Performance tuning guide

- [ ] **SynOS Boot Examples**
  - [ ] Minimal boot configuration
  - [ ] Multi-core boot setup
  - [ ] Network boot via virtio-net
  - [ ] Storage boot via AHCI/NVMe

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
# Build the VM
cargo build --release

# Run with SynOS kernel
./target/release/synos-vm --kernel ../kernel/build/bios/kernel.bin \
    --initrd ../kernel/build/bios/initrd.img \
    --memory 128M \
    --append "console=serial0"
