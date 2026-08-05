# 16. The SynOS Virtual Machine

The `virtual_machine` crate is a Rust x86_64 machine used to boot and test SynOS. It is both an emulator and a contract test harness.

## VM layers

```text
Vm
├── Cpu: decoder + executor + modes + interrupts
├── Mmu: frames + paging + MMIO + COW
├── Devices: PCI, APIC, PIT, HPET, serial, PS/2, display, net, storage
├── Firmware: BIOS and UEFI models
├── Loader: kernel, initrd, Multiboot, framebuffer, cmdline
├── Execution: steps, profiles, translation cache
├── Snapshot: state, diffs, chains, restore
└── Terminal/cluster: host input, network faults, node fixtures
```

## CPU and memory

The CPU decoder handles x86_64 prefixes, REX, ModR/M, SIB, displacement, immediates, RIP-relative addressing, segment overrides, far operands, and malformed streams. The executor covers arithmetic, flags, branches, stacks, strings, control/debug registers, MSRs, exceptions, unsupported instructions, and HLT.

The MMU supports frame allocation, page tables, permissions, MMIO, unaligned and out-of-range access, large pages, Copy-on-Write, ballooning, overcommit, and memory statistics.

## Devices

The VM models:

- PCI Express host bridge and configuration space;
- xAPIC/PIC behavior, IRQ priority, IPI, LVT, and timer;
- PIT and HPET;
- 16550 serial and PS/2 keyboard/mouse;
- power control;
- VGA text, VESA framebuffer, UEFI GOP;
- AHCI, NVMe, Virtio block/console/RNG/net;
- Intel E1000 and loopback networking;
- guest agent, PV clock, memory hotplug;
- raw, fixed VHD, and QCOW2 disk images.

Every emulated device has quality-gate scenarios for register/configuration, normal I/O, reset, interrupt, malformed input, and failure.

## Disk management

Disk specifications include a stable ID, role, controller, location, image path, format, capacity, and persistence mode. The shared attachment path supports AHCI, NVMe, and Virtio block.

Writable images use ownership markers. Read-only, copy-on-write, and disposable modes protect base images. System disks store boot metadata, kernel/initrd, SynFS volume, settings, identity, and reserved update space.

Provision a system disk:

```sh
target/release/synos-vm disk provision ./state/system.raw \
  --kernel build/bios/kernel.bin \
  --size 64M --boot-args "console=serial0"
```

Boot it without host kernel arguments:

```sh
target/release/synos-vm \
  --system-disk ./state/system.raw \
  --firmware bios --interactive
```

## Terminal

The interactive terminal owns host stdin polling, raw mode, input translation, output flushing, and cleanup. It can forward serial bytes or inject PS/2 input. It handles Enter, Backspace, Tab, Ctrl-C, Ctrl-D, EOF, and escape sequences. Guest HLT is a wait state, not process exit.

## Snapshots and monitor

VM snapshots serialize CPU, memory, device, and disk-related state. Snapshot chains use diffs and stable IDs. A Unix monitor can report registers, status, disks, save state, and quit.

```sh
target/release/synos-vm --kernel build/bios/kernel.bin \
  --steps 100000 --snapshot-save ./state/checkpoint.vm
```

## Cluster fixtures

The VM cluster harness models multiple nodes, loopback network faults, latency, loss, duplication, reordering, partitions, CXL memory, shared memory, heartbeats, coherence, migration, fencing, failover, and rejoin. Fast tests use deterministic fixtures. QEMU cluster tests are opt-in.

## Easy VM command

```sh
./scripts/build-bios-image.sh
cargo build --release -p synos-vm
target/release/synos-vm \
  --kernel build/bios/kernel.bin \
  --append "console=serial0" \
  --steps 100000
```

The three memorable flags are:

- `--kernel`: what to boot;
- `--append`: what the guest should know;
- `--steps`: how long the deterministic run may execute.

