# Public VM contract

This document is the compatibility contract for embedders, guest drivers, and
operators of the `ghostos-vm` crate and binary. Public Rust items may expose more
mechanism than is listed here. The invariants below are the behavior callers
may rely on.

## Rust API invariants

### Construction and configuration

- `VmConfig::default()` creates one 128 MiB, single-core, BIOS VM with COM1
  enabled, a 1 GiB hotplug ceiling, software execution, and no disks.
- `Vm::try_with_config` and `Vm::try_with_config_and_clock` are the fallible
  constructors. They validate all disk specifications, open the requested
  acceleration backend, attach disks, and configure persistence before
  returning. On failure they release any disk resources already acquired.
- `Vm::with_config` and `Vm::with_config_and_clock` panic when construction
  fails. Code handling untrusted configuration must use a `try_` constructor.
- `max_memory_size` is raised to at least `memory_size`. `hotplug_memory`
  appends RAM and fails rather than exceeding that ceiling or wrapping an
  address calculation.
- Firmware, kernel/initrd selection, boot arguments, initial RAM, and disk
  topology are configuration state. They are not carried in a snapshot.
  Restore therefore requires a VM constructed with compatible configuration.

### Execution and lifecycle

- VM construction wires the machine but does not boot it. The first run method
  initializes firmware and boot artifacts exactly once until `reset`.
- `run` and the terminal run methods continue until monitor stop, guest
  shutdown, or an error. `run_for_steps` executes no more than the requested
  instruction count and returns the actual count, halt state, and RIP.
- A guest `HLT` does not end an unbounded run. Devices and timers continue to
  be polled and host input may wake the CPU. A bounded run may return early
  when the CPU is halted.
- Monitor callbacks run between guest batches. In an unbounded run, returning
  `false` is a clean stop that closes attached disks. Bounded runs stop and
  sync disks. Guest shutdown closes disks; reboot syncs them, resets the VM,
  and boots again.
- `reset` clears CPU, execution cache, RAM/MMU mappings, interrupt controller,
  APIC, timers, storage/Virtio devices, display, firmware, and port-device
  state. MMIO/port wiring and configured backing stores remain where their
  device reset contract preserves them. The current PCI-host reset clears its
  function inventory and does not rebuild it; embedders requiring PCI discovery
  after a direct reset must restore that inventory. Reset is not snapshot
  restore.
- `request_shutdown` and `request_reboot` change the host-visible power state
  and enqueue both host and guest-agent notifications.
- Public `Rc<RefCell<_>>` device handles refer to the live device wired into
  the VM. Callers must not hold a mutable borrow while entering a VM method
  that can access the same device.

### Disks, replay, and restore

- Disk IDs are non-empty and unique. There is at most one system disk and one
  attachment at bus 0, slot 0 for each of AHCI, NVMe, and Virtio block.
- `disks()` is deterministic. `attach_disk` makes a disk immediately visible;
  `detach_disk` flushes, requests durable host storage, and closes it.
  `flush_disks` flushes controller images and durably syncs the persistence
  image; `sync_disks` durably syncs every controller image.
- Writable persistent images have exclusive ownership markers. Read-only and
  temporary copy-on-write attachments do not claim the base image for writes.
- Host inputs, timer/device observations, DMA completions, and interrupt
  delivery are recorded by replay mode. A replay trace is separate from a VM
  snapshot and must be saved explicitly.
- `restore_snapshot` first validates the snapshot and requires the VM RAM size
  to match exactly. RAM size is the only topology compatibility check enforced
  by this API. It restores only the fields named in the snapshot schema and
  clears translated code. Failure does not mean external disks or host device
  backends were rolled back.
- Raw `save_snapshot`/`load_snapshot` data is only for offline conversion and
  fixtures. Persistent or remote checkpoints must use the authenticated APIs.

## Guest device map

The default topology is fixed by `Vm::build_with_config`. PCI addresses use
`bus:device.function`. Port and MMIO ranges are inclusive below.

| Device | Guest address | PCI function/BAR | Notes |
| --- | --- | --- | --- |
| Legacy PICs | I/O `0x20-0x21`, `0xA0-0xA1` | none | Compatibility interrupt control |
| 8254 PIT | I/O `0x40-0x43` | none | Three channels plus command port |
| PS/2 i8042 | I/O `0x60-0x64` | none | Data at `0x60`, status/command at `0x64` |
| Power control | I/O `0x604-0x607` | none | ACPI-style shutdown/reboot control |
| Serial 16550 | I/O `serial_port..+7` | none | Default COM1 `0x3F8-0x3FF`; optional |
| VGA registers | I/O `0x3C0-0x3DA` | none | VGA controller subset |
| PCI config | I/O `0xCF8-0xCFF` | host bridge | Same topology as ECAM |
| Virtio net | I/O `0x5000-0x50FF` | `00:07.0`, BAR0 | Legacy Virtio PCI, loopback backend |
| Virtio block | I/O `0x5100-0x51FF` | `00:08.0`, BAR0 | One optional disk |
| Virtio console | I/O `0x5200-0x52FF` | `00:09.0`, BAR0 | Legacy Virtio console |
| Virtio RNG | I/O `0x5300-0x53FF` | `00:0A.0`, BAR0 | Legacy Virtio entropy device |
| GhostOS persistence | I/O `0x5400-0x540B` | none | Command, length, and data registers |
| VGA text | MMIO `0x000B8000-0x000BFFFF` | none | 32 KiB text aperture; 80x25 model |
| PCIe ECAM | MMIO `0xE0000000-0xEFFFFFFF` | host bridge | 256 buses, 32 devices, 8 functions |
| VESA framebuffer | MMIO `0xF0000000-0xF0FFFFFF` | none | 16 MiB linear framebuffer |
| AHCI | MMIO `0xF1000000-0xF1000FFF` | `00:04.0`, BAR5 | One SATA disk |
| NVMe | MMIO `0xF1100000-0xF1101FFF` | `00:05.0`, BAR0 | One namespace |
| Intel e1000 | MMIO `0xF1200000-0xF121FFFF` | `00:06.0`, BAR0 | ID `8086:100E`, loopback backend |
| Memory hotplug | MMIO `0xFEBE0000-0xFEBE0FFF` | none | GhostOS paravirtual device |
| Guest agent | MMIO `0xFEBF0000-0xFEBF0FFF` | none | Magic `SOGA`, interface version 1 |
| HPET | MMIO `0xFED00000-0xFED00FFF` | none | 32 timer slots |
| Local APIC | MMIO `0xFEE00000-0xFEE00FFF` | none | BSP APIC ID 0; also MSR `0x1B` |

COM2 is reserved at `0x2F8` but is not attached by default. Changing
`VmConfig.serial_port` moves the one emulated UART; callers must avoid overlap
with another mapped range.

## Interrupt routes

Devices signal the shared local APIC directly. The vector values are part of
the default guest-visible topology.

| Source | Compatibility route | APIC vector | Trigger/condition |
| --- | --- | ---: | --- |
| PIT channel 0 | ISA IRQ0 | `0x20` | Timer event |
| HPET timer 0 in legacy mode | ISA IRQ0 | `0x20` | Timer event |
| HPET timer 1 in legacy mode | ISA IRQ1 | `0x21` | Timer event |
| HPET timer `i` otherwise | IRQ `i % 24` | `0x20 + (i % 24)` | Timer event |
| PS/2 keyboard | ISA IRQ1 | `0x21` | Output becomes available and enabled |
| Serial UART | ISA IRQ4 | `0x24` | Enabled receive/transmit condition |
| AHCI | ISA IRQ11 | `0x2B` | Enabled controller/port completion |
| PS/2 mouse | ISA IRQ12 | `0x2C` | Auxiliary output becomes available |
| e1000 | fixed | `0x2D` | `ICR & IMS != 0` |
| Virtio net | fixed | `0x2E` | Queue/config interrupt |
| NVMe | IRQ17 | `0x31` | Enabled completion |
| Virtio block | fixed | `0x32` | Queue completion |
| Virtio console | fixed | `0x33` | Queue/config interrupt |
| Virtio RNG | fixed | `0x34` | Queue completion |
| Guest agent | fixed | `0x35` | Host message or guest event |
| Memory hotplug | fixed | `0x36` | New region notification |

The PIT, HPET legacy base, serial, storage, network, Virtio, PS/2, guest-agent,
and hotplug vectors can be changed through their device APIs. A changed route
is runtime topology and is not preserved by the current snapshot schema unless
it is represented in serialized APIC/controller state.

## Snapshot schema

The authoritative state boundary is [SNAPSHOT_STATE_INVENTORY.md](SNAPSHOT_STATE_INVENTORY.md).
The byte-level upgrade rules are in [SNAPSHOT_FORMAT.md](SNAPSHOT_FORMAT.md).

### Raw payload

All integers are little-endian. The raw payload is ordered as follows:

1. magic `SYNOVM01` (8 bytes)
2. format version (`u32`)
3. version 2 feature flags (`u64`); absent in version 1
4. RAM size (`u64`)
5. CPU state
6. MMU state, including all RAM and memory-management collections
7. interrupt-controller state
8. local-APIC state
9. BIOS lifecycle byte
10. 256-entry BIOS IVT (`u16` entries), 256-byte BDA, and 128-byte EGA state
11. BIOS reset vector (`u64`)

Versions 1 and 2 are readable and writable. Version 1 implies all five feature
bits. Version 2 explicitly requires `CPU_STATE`, `MMU_STATE`,
`INTERRUPT_CONTROLLER`, `APIC_STATE`, and `BIOS_STATE`. Unknown, missing, or
trailing data is rejected. Snapshot and RAM limits are 64 GiB; collection
lengths are bounded before allocation.

The authenticated `SYNOSIG1` envelope contains envelope version 1, algorithm
1 (HMAC-SHA256), three zero reserved bytes, a 16-byte key ID, `u64` payload
length, the raw payload, and a 32-byte tag over all preceding envelope bytes.
Keys are exactly 32 bytes. Authentication is integrity and peer-key selection,
not encryption.

### Restore boundary

The schema includes CPU, RAM/MMU, interrupt controller, local APIC, and selected
BIOS state. It excludes disks and their bytes, all device queues, serial/PS2
buffers, PIT/HPET progress, network queues/backends, RNG state, display state,
guest-agent and hotplug queues, UEFI runtime pointers, host accelerator handles,
configuration, and replay traces. Those are rebuilt or intentionally discarded.
A caller needing exact continuation must quiesce excluded devices and recreate
the same topology and external disk state before restore.

## Disk contract and formats

All controllers expose 512-byte logical sectors. Capacity must be non-zero and
a multiple of 512. Reads and writes are range checked; writes to a read-only
image fail. The supported backing formats are:

| Format | Detection and logical capacity | Supported subset |
| --- | --- | --- |
| RAW | Fallback when no VHD/QCOW2 magic matches; file length is capacity | Direct sector data; sparse host files are allowed |
| VHD | `conectix` footer; footer current size is capacity | Fixed VHD only; dynamic/differencing types are rejected; original/current size, exact file length, and footer checksum are validated |
| QCOW2 | `QFI\xFB` header; header virtual size is capacity | Versions 2 and 3, cluster bits 9-21, L1/L2 allocation; no backing file, encryption, compression, or incompatible dirty/corrupt features |

Format detection is content based; an explicit `DiskSpec.format` must match.
An explicit capacity must equal the detected logical capacity exactly.

Each controller has one slot at bus 0, slot 0. `Persistent` writes the original
image and uses an exclusive `<canonical-image>.ghostos.lock` for writable use.
`CopyOnWrite` and `Disposable` operate on a temporary clone discarded at close;
read-only mode never modifies or locks the base. The lock records version,
canonical image identity, owner, PID/start marker, host identity, and format.
Only the stale-lock recovery API may remove a stale marker.

System disks use format version 1 (`SYSTEM_DISK_FORMAT_VERSION`) and two manifest
slots. Payloads are synced before a new manifest generation is published.
Readers choose the newest complete, checksum-valid generation and may fall back
to the other slot after an interrupted update. A system disk is external to a
VM snapshot and must be backed up, restored, and validated separately.

## Migration protocol

The CLI accepts only protocol 3 with magic `SYNOMIG3`. Protocols 1 and 2 are
legacy unauthenticated forms and are rejected. The TCP connection has 10-second
connect, read, and write timeouts. TCP is not encrypted: `--secure-transport`
is an operator assertion that mutually authenticated TLS, an authorized VPN,
or an SSH tunnel already protects it.

The protocol sequence is:

1. Sender writes magic, protocol version, its snapshot schema, 16-byte key ID,
   a 32-byte sender nonce, and an HMAC sender proof.
2. Receiver verifies authorization and proof, negotiates the highest common
   snapshot version with all required feature bits, then returns success,
   negotiated schema, key ID, a 32-byte receiver nonce, and its HMAC proof.
3. Sender converts the authenticated snapshot to that schema and writes
   issuance time (`u64` Unix seconds), SHA-256 checkpoint ID, payload length
   (`u64`), authenticated snapshot bytes, and a 32-byte frame HMAC.
4. Receiver checks freshness, length before allocation, HMAC, SHA-256 identity,
   authenticated snapshot envelope, and exact negotiated schema.
5. Receiver reserves the checkpoint identity in its durable replay ledger,
   writes and syncs a same-directory temporary file, atomically publishes it,
   syncs the directory, reopens and authenticates the result, and preserves the
   old destination on any failed publication.

Payload allocation is capped at 8 GiB. Checkpoints older than 24 hours or more
than five minutes in the future are rejected. The replay ledger holds up to
4096 fixed-size records and rejects repeat delivery during the freshness
window. The shared 32-byte key authenticates both peers and frames; the
configured peer key ID must be allow-listed. The private audit log is
fail-closed and records `started`, `succeeded`, and `failed` events.

Migration transfers a checkpoint file, not a running VM process. It does not
transfer excluded device state, disk bytes, image locks, network connections,
host terminal state, or acceleration handles. Destination configuration and
external storage must already be compatible.

## Terminal contract

- `TerminalSession::new(Some(true))` requests interaction;
  `Some(false)` never changes terminal settings; `None` enables interaction
  only when both stdin and stdout are TTYs.
- Linux and macOS use the Unix raw-mode path. It opens `/dev/tty`, saves
  termios, and installs restoration handlers for normal termination and fatal
  crash signals. Settings are restored on EOF, session drop (including unwind
  or after an error), or those signals before the signal is re-raised. Only one
  process-wide Unix raw session may be active.
- Windows raw mode saves input/output console modes, disables processed,
  line, echo, and quick-edit input, enables virtual-terminal input/output, and
  restores both handles on error or drop. Other platforms use non-mutating
  fallback behavior and report no terminal size.
- Non-TTY and `new_with_io` sessions never change host terminal state. Input
  reading happens on a background thread; `poll` only drains available events
  and does not block VM execution.
- Input policy changes DEL (`0x7F`) to backspace (`0x08`) and otherwise keeps
  bytes unchanged. EOF produces one EOT byte (`0x04`) and restores raw mode.
  Escape sequences pass through unchanged. In raw mode Ctrl-C is input byte
  `0x03`; it is not interpreted as a VM stop command. Guest escape sequences
  are output bytes, not host diagnostics.
- Terminal size is checked at most every 250 ms of the injected monotonic
  clock and only changed sizes are emitted. Serial mode sends
  `ESC [ 8 ; <rows> ; <columns> t`; PS/2 mode emits set-1 make/break sequences
  for supported ASCII and does not synthesize a resize sequence.
- `write_output` writes the complete byte slice; flushing is explicit.
  Structured failures expose only operation, `io::ErrorKind`, and optional OS
  error number, never guest-controlled bytes or an arbitrary error string.
- `transcript()` records raw and translated input, EOF policy, and resize
  events in order. `replay()` returns deterministic `TerminalInput` values
  independent of host time and terminal size. Session diagnostics count polls,
  bytes, flushes, EOFs, resizes, and the last structured failure.
- `Vm::run_with_terminal` routes input to serial. The mode variant may route it
  to serial or PS/2. Input wakes a halted guest. VM serial output is flushed by
  the UART to process stdout; it does not use the `TerminalSession` output
  object. The session's output methods are separate embedder helpers, and the
  terminal lifetime remains host-owned.
