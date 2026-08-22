# VM compatibility matrices

This page records what can be mixed safely. `Supported` means the current VM
accepts the combination. `Conditional` means the stated limits apply.
`Rejected` means the parser or validator must fail. `Not portable` means the
bytes may be valid, but the state is host-owned and is not carried across a
snapshot or migration.

## Snapshot and migration matrix

| Artifact | Read | Write | Migration | Compatibility rule |
| --- | --- | --- | --- | --- |
| Raw `SYNOVM01`, format 1 | Supported | Supported | Conditional | Version 1 implies all feature bits; use only for offline conversion or local fixtures |
| Raw `SYNOVM01`, format 2 | Supported | Supported | Conditional | Explicit `u64` feature word; all current feature bits are required |
| `SYNOSIG1` authenticated envelope, auth version 1 | Supported with the 32-byte key | Supported | Supported by protocol 3 | HMAC-SHA256, 16-byte key ID, bounded payload; authentication does not encrypt |
| Unknown snapshot format/version/feature | Rejected | N/A | Rejected | No unknown fields, missing required bits, trailing bytes, or oversized lengths |
| `SYNOMIG3` stream | N/A | N/A | Supported | Schema negotiation, mutual HMAC challenge, freshness, replay ledger, and 10-second I/O timeouts |
| `SYNOMIG1` or `SYNOMIG2` stream | N/A | N/A | Rejected | Legacy unauthenticated migration is not accepted |

Current local schema is version range `1..=2` with `CPU_STATE`, `MMU_STATE`,
`INTERRUPT_CONTROLLER`, `APIC_STATE`, and `BIOS_STATE`. Negotiation chooses the
highest overlapping version. A VM restore requires exactly matching RAM size;
configuration, device topology, disk contents, queues, timers, network
backends, RNG, terminal, and acceleration handles are outside the snapshot.
Use the same VM topology and external disks at the destination.

The raw payload and authenticated envelope are capped at 64 GiB; migration
allocation is capped at 8 GiB. Snapshot compatibility is therefore a pair of
format and resource checks, not just a version check.

## System-disk matrix

| System-disk property | RAW | Fixed VHD | QCOW2 |
| --- | --- | --- | --- |
| Data-disk attachment | Supported | Supported, capacity max 4 GiB | Supported |
| System-disk provisioning | Supported | Supported, capacity max 4 GiB | Supported |
| Logical sectors | 512 bytes | 512 bytes | 512 bytes |
| Minimum new system image | 16 MiB | 16 MiB | 16 MiB |
| New-image alignment | 1 MiB capacity alignment | 1 MiB capacity alignment | 1 MiB capacity alignment |
| Format detection | File length fallback | `conectix` footer | `QFI\xFB` header |
| Unsupported features | None beyond range/sector checks | Dynamic/differencing VHD | Backing file, encryption, compression, dirty/corrupt incompatible features |
| System metadata version | Format version 1 | Format version 1 | Format version 1 |
| Copy-on-write/disposable base | Supported | Supported | Supported |

All images must have non-zero sector-aligned capacity. Existing image capacity
must equal `--disk-size` when supplied. The system-disk header, two redundant
manifest slots, checksums, settings extent, and GhostFS system volume must all
validate. The newest complete manifest generation wins; a torn update may fall
back to the other slot. A newer system-disk format is rejected, not silently
mounted.

`--read-only` opens the base without a writable ownership lock. `--copy-on-write`
and `--disposable` clone the base and discard guest writes at close. Persistent
writable use claims `<canonical-image>.ghostos.lock`; stale recovery is explicit.

## Guest boot-image matrix

| Input | BIOS | UEFI without `--efi` | UEFI with `--efi` | Required format and result |
| --- | --- | --- | --- | --- |
| `--kernel PATH` | Supported | Supported | Ignored for boot handoff | Raw bytes are loaded at 1 MiB without architecture validation; a valid ELF64 uses its ELF load range and entry |
| `--initrd PATH` | Conditional | Conditional | Ignored for EFI-app handoff | Optional arbitrary bytes, page-aligned after the kernel; layout overlap is rejected |
| `--system-disk PATH` | Supported | Supported | Ignored for EFI-app handoff | Manifest v1 supplies kernel/initrd/settings/GhostFS volume; kernel follows the same raw/ELF64 rules |
| `--efi PATH` | Rejected | N/A | Supported | PE32+ x86-64 (`MZ`, `PE\0\0`, machine `0x8664`, optional-header magic `0x20B`) |
| ELF32 or another non-ELF kernel | Conditional | Conditional | Conditional | Treated as opaque raw bytes, not rejected by the loader; the long-mode handoff requires the payload itself to be x86-64-compatible |
| Non-x86-64 EFI application | Rejected | Rejected | Rejected | PE must be PE32+ with machine `0x8664` |

With no EFI application selected, UEFI initializes its firmware services and
then the VM's direct loader can boot `--kernel` or a system-disk kernel. When
`--efi` is selected, the EFI application is the handoff target and direct
kernel/system-disk boot loading is skipped. `--efi` requires `--firmware uefi`.

Boot information is supplied in GhostOS's versioned boot protocol and a
Multiboot-compatible structure. The kernel receives the boot-info pointer in
`RDI`, the Multiboot pointer in `RSI`/`RBX`, and the Multiboot bootloader magic
in `RAX`. `--append` overrides a system-disk `boot_args` value; otherwise the
system-disk setting/manifest value is used.

## Device-model matrix

| Model | Guest transport/topology | Backing compatibility | Snapshot state | Host limitation |
| --- | --- | --- | --- | --- |
| Legacy PIC + local APIC | I/O PIC; APIC MMIO/MSR | Always present | Controller/APIC state serialized | One BSP APIC; PCI inventory is cleared by direct `reset` |
| PIT / HPET | PIT I/O; HPET MMIO | Always present | Timer live countdown excluded | Rebuilt from injected monotonic clock |
| Serial 16550 | Configured I/O base, default COM1 | Optional | RX/TX and host output excluded | Host stdout output; one UART |
| PS/2 keyboard/mouse | I/O `0x60-0x64` | Always present | Input FIFO excluded | Host input must be replayed or re-injected |
| VGA/VESA display | VGA ports, text MMIO, 16 MiB LFB | Always present | Display/palette excluded | Host display is not portable |
| AHCI SATA | PCI `00:04.0`, BAR5 MMIO | RAW/VHD/QCOW2, one disk | Controller queues excluded | One disk slot |
| NVMe | PCI `00:05.0`, BAR0 MMIO | RAW/VHD/QCOW2, one namespace | Queue state excluded | One namespace |
| Intel e1000 | PCI `00:06.0`, MMIO | Loopback network backend | Rings/packets/backend excluded | No external NIC backend |
| Virtio net | Legacy PCI/I/O `00:07.0` | Loopback network backend | Rings/packets/backend excluded | Legacy transport only |
| Virtio block | Legacy PCI/I/O `00:08.0` | RAW/VHD/QCOW2, one disk | Queue state excluded | One disk slot |
| Virtio console | Legacy PCI/I/O `00:09.0` | No external backing | Queue/output excluded | Host-facing transient console |
| Virtio RNG | Legacy PCI/I/O `00:0A.0` | Host entropy | RNG state excluded | Host entropy is nondeterministic |
| Guest agent / PV clock | GhostOS MMIO and KVM-compatible MSRs | Always present | Agent queues and host time excluded | Reattach to destination clock/APIC |
| Memory hotplug | GhostOS MMIO | RAM up to `max_memory_size` | Pending request excluded | Destination must have compatible RAM limits |

The device model, transport, PCI identity, BAR, and interrupt vector are
guest-visible ABI. A snapshot does not carry the model inventory. Restoring or
migrating between different rows is therefore only safe after the guest is
quiesced and the destination explicitly provides the same topology.

## CLI option matrix

### Boot and machine options

| Option | Accepted values/default | Compatibility and conflicts |
| --- | --- | --- |
| `--firmware`, `-f` | `bios` (default), `uefi` | `--efi` requires `uefi`; both direct kernels work without an EFI app |
| `--efi` | PE32+ x86-64 path | UEFI only; selects EFI handoff and skips direct kernel/system-disk loading |
| `--kernel`, `-k` | Path to raw or ELF64 bytes | Required by `--integration`; may be replaced by a system-disk kernel |
| `--initrd`, `-i` | Any readable file | Optional; must fit without overlap in guest RAM |
| `--append`, `-a` | UTF-8 command line | Overrides system-disk boot args |
| `--memory`, `-m` | Positive bytes with `K/M/G`, `KB/MB/GB`, `KiB/MiB/GiB` | Snapshot restore requires exact snapshot RAM; default 128 MiB |
| `--cpus`, `-c` | Positive integer, default 1 | Stored/reported as requested SMP count; current execution engine remains one CPU |
| `--accel` | `software`, `auto`, `kvm`, `haxm`, `hvf`, `whpx` | Explicit unavailable backend fails; `auto` probes and reports fallback |
| `--serial` / `--no-serial` | Enabled by default | Interactive serial input requires serial enabled |
| `--serial-port` | `com1`, `com2`, or hexadecimal I/O base | Moves the single UART; caller must avoid address overlap |
| `--interactive` / `--terminal` | Force raw terminal | Cannot combine with `--steps` or `--monitor` |
| `--non-interactive` / `--no-terminal` | Disable raw terminal | Pipe input remains usable; monitor implies this when unspecified |
| `--input` | `serial` (default), `ps2`/`keyboard` | Chooses host-to-guest input path; serial mode needs serial for interactive use |
| `--steps` | Non-negative `u64` instruction bound | Bounded run; cannot combine with interactive terminal |

### Disk options

| Option | Accepted values/default | Compatibility and conflicts |
| --- | --- | --- |
| `--disk` | Data-image path; repeatable | Each controller has one slot; IDs are generated `disk0`, `disk1`, ... |
| `--system-disk` | One system-image path | Supplies boot artifacts when no direct kernel is selected |
| `--disk-controller` | `ahci`/`sata`, `nvme`, `virtio`/`virtio-blk` | Controller choice is guest-visible; one attached image per controller |
| `--disk-format` | `raw`/`img`, `vhd`, `qcow`/`qcow2` | Explicit format must match image; otherwise format is detected |
| `--disk-size` | Positive byte size | Exact capacity check for existing images; required when creating missing image |
| `--read-only` / `--read-write` | Read-only or writable (default) | Read-only does not claim a writable lock |
| `--copy-on-write` | Temporary clone | Guest writes discarded; base is not modified |
| `--disposable` | Temporary clone | Same current clone/discard behavior as copy-on-write |
| `--create-if-missing` | Explicit creation opt-in | Requires `--disk-size`; never creates an unspecified host path |
| `--list-disks` | Inspect and exit | Does not boot; accepts disk options and reports health/identity |

### State, monitor, replay, and commands

| Option/command | Required companions | Compatibility rule |
| --- | --- | --- |
| `--snapshot-save PATH` | `--snapshot-key KEY` | Saves authenticated envelope when VM stops; not allowed with `--integration` |
| `--snapshot-restore PATH` | `--snapshot-key KEY` | Loads authenticated checkpoint before run; RAM must match or be inferred |
| `--snapshot-key PATH` | 32 raw bytes or 64 hex characters | Snapshot and monitor keys must differ |
| `--monitor SOCKET` | `--monitor-auth-key KEY` | Unix monitor; cannot combine with interactive terminal |
| `--monitor-auth-key PATH` | `--monitor` | Private regular file; permissions are checked |
| `--monitor-allow LIST` | `--monitor` | Comma list: `status`, `device(s)`, `disk(s)`, `migration`, `save`, `quit`, `sensitive`, or `all`; save also needs snapshot key |
| `--replay-record PATH` | None | Mutually exclusive with `--replay`; deterministic input/device trace |
| `--replay PATH` | None | Mutually exclusive with recording; terminal is forced non-interactive |
| `--integration` | `--kernel` | Conflicts with snapshot and replay options; uses bounded integration checks |
| `disk provision PATH` | `--kernel`; optional `--size`, `--format`, `--initrd` | Creates/installs system disk; replacement requires explicit `--replace` and size |
| `disk inspect PATH` | None | Read-only structural report; never repairs |
| `disk validate PATH` | None | Validates installed system manifest and payload checksums |
| `disk repair PATH` | None | Explicitly repairs supported redundant metadata only |
| `disk lock PATH` / `recover-lock PATH` | None | Diagnose; recovery succeeds only for a verified stale owner |
| `migrate send/receive` | `--key`, `--peer-key-id`, `--audit-log`, `--secure-transport` | Protocol 3 only; secure transport flag acknowledges external TLS/VPN/SSH protection |

Unknown options, missing values, invalid enum values, and forbidden option
combinations are parser errors. This matrix should be updated in the same
change as a guest-visible option, image-format, device-model, or snapshot
schema change.
