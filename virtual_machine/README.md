# SynOS Virtual Machine

A lightweight virtual machine implementation in Rust designed to serve as a test environment for booting SynOS.

## Features

- **x86_64 CPU Emulation**: Instruction decoder and executor with support for protected mode and long mode transitions
- **Memory Management**: Simple memory-mapped I/O with page table manager
- **Hardware Emulation**: Basic PCI, interrupt controller, and BIOS support
- **Boot Support**: Multiboot specification support for loading SynOS kernel

## Building

Run these commands from the `virtual_machine/` directory. The plain
`cargo build` command creates `target/debug/synos-vm`; the usage example below
uses the release binary.

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
./scripts/build-bios-image.sh

cd virtual_machine
cargo build --release

./target/release/synos-vm \
  --kernel ../build/bios/kernel.bin \
  --memory 128M \
  --append "console=serial0"
```

Pass `--initrd <PATH>` only when you have a separate initrd image.

Run `synos-vm --help` for all boot and machine options. Use `--steps` for a
bounded run or `--integration` to run the SynOS integration checks.

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
backend. The storage example attaches a disk for guest I/O. The current CLI
does not implement PXE network boot or firmware boot directly from AHCI/NVMe;
those examples exercise device bring-up while the kernel is loaded by the
host-side SynOS loader.

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
