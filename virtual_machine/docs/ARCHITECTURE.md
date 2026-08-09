# Virtual machine architecture

The VM is a small, single-process x86_64 machine model. Guest code runs in
the emulated CPU. Guest memory lives in the MMU. Devices are connected to the
MMU through memory-mapped I/O or to the CPU through port-mapped I/O.

## Runtime shape

```text
                    synos-vm CLI or Vm API
                              |
                              v
         +--------------------+--------------------+
         |                    Vm                   |
         |  boot + CPU step + device polling       |
         +-----------+----------------+-------------+
                     |                |
                 +---v---+        +---v---+
                 |  CPU  |        |  MMU  |
                 +---+---+        +---+---+
                     |                |
              +------v------+   +-----v------+
              | PortBus     |   | MmioRegion |
              | COM1, PS/2, |   | APIC, PCI, |
              | PIT, Virtio |   | timers,    |
              +-------------+   | storage,   |
                                | display    |
                                +------------+
```

`Vm::with_config` builds the default PC-like topology. It creates one BSP CPU
and one local APIC, then attaches PCI, timers, display, serial, PS/2, storage,
network, and legacy Virtio devices. PCI configuration is available through
both `0xCF8/0xCFC` and the ECAM aperture at `0xE000_0000`.

The current execution topology has one CPU/BSP. The `--cpus` setting is kept
in `VmConfig` for machine configuration and boot examples, but it does not
create additional CPU execution contexts yet.

## Boot flow

1. `Vm::run` or `Vm::run_for_steps` initializes BIOS or UEFI.
2. A configured kernel and optional initrd are loaded by `boot::Loader`.
3. Boot parameters and the framebuffer description are written to guest RAM.
4. The loader hands control to the kernel at `KERNEL_LOAD_ADDR`.
5. The execution engine translates and runs guest instructions.
6. Pending DMA, network traffic, and timer interrupts are serviced between
   CPU dispatches.

BIOS kernel boot uses the SynOS multiboot handoff. UEFI mode starts the EFI
application supplied with `Vm::set_efi_application`. A kernel path cannot be
used as a UEFI application.

## Deterministic replay

`Vm::begin_replay_recording` starts one ordered trace. The MMU and `PortBus`
record device-read values with the guest instruction address. Each VM batch
also records the virtual clock value, timer result, accepted interrupt, and
all guest-RAM writes made by device completion. Serial, keyboard, mouse, and
terminal resize input are recorded before they reach the guest. Replay uses
the saved values and does not trust host clocks, terminal input, entropy, or
device completion bytes.

Use `Vm::save_replay` and `Vm::load_replay` for the bounded binary trace
format. The CLI exposes the same flow with `--replay-record PATH` and
`--replay PATH`. A mismatch returns `VmError::Replay` at the first divergent
event.

## Device map

| Device | Guest interface | Address | IRQ/vector |
| --- | --- | --- | --- |
| COM1 | I/O ports | `0x3F8..0x3FF` | `0x24` |
| PCI config | I/O ports + ECAM | `0xCF8`, `0xE000_0000` | — |
| PIT | I/O ports | `0x40..0x43` | `0x20` |
| Local APIC | MMIO + `IA32_APIC_BASE` | `0xFEE0_0000` | — |
| HPET | MMIO | `0xFED0_0000` | `0x20`/`0x21` legacy routes |
| AHCI | PCI BAR5 MMIO | `0xF100_0000` | `0x2B` |
| NVMe | PCI BAR0 MMIO | `0xF110_0000` | `0x31` |
| e1000 | PCI BAR0 MMIO | `0xF120_0000` | `0x2D` |
| virtio-net | legacy I/O BAR | `0x5000..0x50FF` | `0x2E` |
| virtio-blk | legacy I/O BAR | `0x5100..0x51FF` | `0x32` |
| virtio-console | legacy I/O BAR | `0x5200..0x52FF` | `0x33` |
| virtio-rng | legacy I/O BAR | `0x5300..0x53FF` | `0x34` |
| VGA text | MMIO | `0xB8000` | — |
| VESA framebuffer | MMIO | `0xF000_0000` | — |

The exact constants are exported from `synos_vm`, so guest-facing tools should
use the Rust constants instead of copying addresses where possible.

## Execution and state

The execution engine caches decoded straight-line blocks. Hot loop blocks can
be promoted to the portable compiled IR form. It is not host-native machine
code. Device boundaries, interrupts, control flow, and self-modifying code
end a block so the VM can poll devices safely.

Snapshots contain CPU, RAM, paging, interrupt, APIC, and BIOS state. Host
translation caches and network backends are not serialized. Restore rebuilds
the execution cache while keeping the VM device topology attached.

## Source layout

- `src/cpu`: registers, modes, decoding, and instruction execution.
- `src/memory`: physical RAM, paging, access checks, and MMIO dispatch.
- `src/devices`: PCI, interrupts, timers, display, serial, input, Virtio,
  network, and storage models.
- `src/firmware`: BIOS and UEFI contexts.
- `src/boot`: kernel, initrd, multiboot, and boot-parameter loading.
- `src/execution.rs`: translated block cache and profiling.
- `src/snapshot.rs`: full snapshots, page diffs, and snapshot chains.
- `src/integration.rs`: bounded SynOS kernel/service checks.
- `src/replay.rs`: ordered event trace, bounded binary persistence, and replay.
