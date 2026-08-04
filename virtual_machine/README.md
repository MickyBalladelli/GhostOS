# SynOS Virtual Machine

A lightweight virtual machine implementation in Rust designed to serve as a test environment for booting SynOS.

## Features

- **x86_64 CPU Emulation**: Instruction decoder and executor with support for protected mode and long mode transitions
- **Memory Management**: Simple memory-mapped I/O with page table manager
- **Hardware Emulation**: Basic PCI, interrupt controller, and BIOS support
- **Boot Support**: Multiboot specification support for loading SynOS kernel

## Building

Run these commands from the `virtual_machine/` directory. Because this crate
belongs to the repository workspace, Cargo writes the binary to the workspace
root at `../target/`.

```bash
cargo build --release
```

## Usage

From the repository root, build the BIOS kernel first:

```bash
./scripts/build-bios-image.sh
```

Then run the VM from the `virtual_machine/` directory. The BIOS build creates
`build/bios/kernel.bin`; it does not create an initrd by default.


The whole sequence of commands is: 
```bash
# from the root folder
./scripts/build-bios-image.sh

cd virtual_machine
cargo build --release

../target/release/synos-vm \
  --kernel ../build/bios/kernel.bin \
  --memory 128M \
  --append "console=serial0"

cargo build --release

```

Pass `--initrd <PATH>` only when you have a separate initrd image.

### Disk operations

Disk paths are always supplied explicitly. The VM canonicalizes them before
attachment; it never chooses a host disk. A new VM can provision and boot from
a system disk like this:

```bash
../target/release/synos-vm disk provision ./state/system.raw \
  --kernel ../build/bios/kernel.bin \
  --size 64M --boot-args "console=serial0"

../target/release/synos-vm \
  --system-disk ./state/system.raw --firmware bios --interactive
```

Reopen that VM with the same `--system-disk` path. Attach a data disk with
`--disk PATH`; choose `--disk-controller ahci|nvme|virtio-blk` and
`--disk-format raw|vhd|qcow2`. `--disk-size` checks an existing image's exact
capacity. `--create-if-missing` requires `--disk-size` and is the only way the
CLI creates an image.

Inspect disks without booting:

```bash
../target/release/synos-vm disk list --system-disk ./state/system.raw
../target/release/synos-vm disk inspect ./state/system.raw
../target/release/synos-vm disk validate ./state/system.raw
```

Use `--read-only` to share a base image safely, or `--copy-on-write` for a
temporary writable clone. Writable persistent attachments create a
`<image>.synos.lock` ownership marker. Use `disk lock PATH` to inspect its PID
and owner metadata, then `disk recover-lock PATH` only after the owner is
reported stale.

Run `synos-vm --help` for all boot and machine options. Use `--steps` for a
bounded run or `--integration` to run the SynOS integration checks.

### Interactive terminal

When stdin and stdout are TTYs, an unbounded VM run attaches the host terminal
to the guest serial console automatically:

```bash
../target/release/synos-vm \
  --kernel ../build/bios/kernel.bin \
  --append "console=serial0" \
  --interactive
```

Use `--interactive` or `--non-interactive` to choose the terminal behavior
explicitly. `--non-interactive` keeps pipe input usable without changing TTY
settings. Select `--serial-port com1`, `--serial-port com2`, or a hex I/O base;
COM1 (`0x3f8`) is the default. Guests using the PS/2 keyboard path can use
`--input ps2` instead of the default `--input serial`.

Input is passed through as terminal bytes, including Enter, Tab, Ctrl-D, and
ANSI escape sequences. Backspace is normalized to BS. Ctrl-C is sent to the
guest shell and does not stop the VM. EOF sends Ctrl-D to the guest. Only guest
ACPI poweroff ends the session; reboot requests reset and boot the guest again.
Guest serial output passes through to stdout with prompt flushing.
`--steps <COUNT>` remains the bounded,
non-interactive instruction-run mode.

## Documentation and examples

Read the guides in this order:

1. [Architecture](docs/ARCHITECTURE.md) — VM lifecycle, memory map, and
   device topology.
2. [Device emulation](docs/DEVICE_EMULATION.md) — MMIO, port I/O, DMA, and
   existing device families.
3. [Adding a device](docs/ADDING_A_DEVICE.md) — wiring a new device into the
   VM safely.
4. [Performance tuning](docs/PERFORMANCE.md) — translated blocks, profiling,
   and cache settings.

Runnable examples live in [`examples/`](examples/):

- `bash examples/minimal-bios.sh` boots a kernel with BIOS and serial output.
- `bash examples/multicore-bios.sh` shows the `--cpus 2` machine setting.
- `bash examples/virtio-net-bios.sh` boots with the virtio-net guest argument
  and a bounded run.
- `cargo run --release --example storage-bios -- --help` shows how to attach a
  RAW, fixed VHD, or QCOW2 image to AHCI, NVMe, or virtio-blk.

The default VM has a virtio-net device connected to an in-memory loopback
backend. The storage example attaches a disk for guest I/O. A disk attached
with `DiskRole::System` can also supply the kernel, initrd, persistent settings,
and SynFS root volume when no host kernel is configured. Explicit host kernel
and initrd paths take precedence.

## Architecture

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

## License

MIT OR Apache-2.0
