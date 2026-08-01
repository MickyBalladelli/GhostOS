# SynOS Virtual Machine

A lightweight virtual machine implementation in Rust designed to serve as a test environment for booting SynOS.

## Features

- **x86_64 CPU Emulation**: Instruction decoder and executor with support for protected mode and long mode transitions
- **Memory Management**: Simple memory-mapped I/O with page table manager
- **Hardware Emulation**: Basic PCI, interrupt controller, and BIOS support
- **Boot Support**: Multiboot specification support for loading SynOS kernel

## Building

```bash
cargo build --release
```

## Usage

```bash
./target/release/synos-vm --kernel ../kernel/build/bios/kernel.bin \
    --initrd ../kernel/build/bios/initrd.img \
    --memory 128M \
    --append "console=serial0"
```

Run `synos-vm --help` for all boot and machine options. Use `--steps` for a
bounded run or `--integration` to run the SynOS integration checks.

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
