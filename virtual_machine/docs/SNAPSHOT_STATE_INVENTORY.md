# VM Snapshot State Inventory

This is the state boundary for `VmSnapshot` version 1. Every mutable
`Vm` field has one status:

- **Serialized**: captured in `VmSnapshot` and encoded by `snapshot.rs`.
- **Rebuilt**: derived from `VmConfig` or host topology when a VM is
  constructed. It is not part of the portable checkpoint.
- **Intentionally excluded**: host-owned, nondeterministic, or transient
  state. It must be quiesced or recreated by the caller when exact
  continuation needs it.

`VmSnapshot::restore_into` restores only serialized state. It also clears the
translation cache. It does not silently serialize device queues, host handles,
or external disk contents.

## `Vm` fields

| Field | Status | Snapshot/restore contract |
| --- | --- | --- |
| `cpu` | Serialized | `VmSnapshot.cpu` |
| `mmu` | Serialized | `VmSnapshot.mmu`, including RAM and page mappings |
| `interrupt_controller` | Serialized | `InterruptControllerState` |
| `ports` | Rebuilt | Port registrations come from `build_with_config`; stateful devices are listed below |
| `serial` | Intentionally excluded | Host serial output and input FIFO are not checkpointed |
| `power_state` | Intentionally excluded | Host lifecycle control is not guest execution state |
| `power_notifications` | Intentionally excluded | Transient host notification queue |
| `guest_power_notifications` | Intentionally excluded | Transient guest notification queue |
| `guest_agent` | Intentionally excluded | Host/guest-agent event and APIC wiring are rebuilt |
| `pv_clock` | Rebuilt | Reattached to the new host timebase; host timestamps are not portable |
| `memory_hotplug` | Intentionally excluded | Pending hotplug request is transient; configured limits come from `VmConfig` |
| `ps2` | Intentionally excluded | Keyboard/mouse input FIFO and pending IRQ are not checkpointed |
| `pci` | Rebuilt | PCI topology and BAR defaults come from `build_with_config` |
| `apic` | Serialized | `LocalApicState`, including IRR/ISR/TMR and timer counters |
| `pit` | Intentionally excluded | Host-time timer progress is not portable |
| `hpet` | Intentionally excluded | Host-time timer progress is not portable |
| `ahci` | Rebuilt | Controller and attached disk topology come from configuration |
| `nvme` | Rebuilt | Controller and attached namespace topology come from configuration |
| `e1000` | Rebuilt | NIC topology and backend wiring come from VM construction |
| `virtio_net` | Rebuilt | NIC topology and backend wiring come from VM construction |
| `virtio_blk` | Rebuilt | Disk attachment comes from disk configuration |
| `virtio_console` | Intentionally excluded | Console output buffer and queue are transient host-facing state |
| `virtio_rng` | Intentionally excluded | Host entropy handle and fallback RNG state are nondeterministic |
| `persistence` | Rebuilt | External persistence image is reopened from disk topology |
| `display` | Intentionally excluded | Host display/palette state is not in the portable checkpoint |
| `bios` | Serialized (selected state) | See the firmware inventory below |
| `execution` | Rebuilt | Translation cache is cleared; profiling counters are not checkpointed |
| `hardware_acceleration` | Rebuilt | Backend is reopened from `VmConfig.hardware_acceleration` |
| `disk_manager` | Rebuilt | Host image paths, locks, and attachments come from disk configuration |
| `booted_system_disk` | Rebuilt | Boot artifacts are reloaded from the configured system image |
| `config` | Rebuilt | Caller supplies configuration; it is not encoded in `VmSnapshot` |
| `initialized` | Rebuilt | Derived from restored BIOS state |

## Serialized nested state

| State | Serialized fields |
| --- | --- |
| `CpuState` | Registers, RIP/RFLAGS, control registers, EFER, segments, descriptor tables, mode, privilege, IST, syscall MSRs, FS/GS bases, halt and interrupt-shadow state |
| `MmuState` | RAM, frame allocator, code version, identity/COW/lazy mappings, balloon state, overcommit state, paging enablement, CR3, and privilege |
| `InterruptControllerState` | IDT base/limit, IRQ routing, PIC mapping |
| `LocalApicState` | APIC registers, IRR/ISR/TMR/level-pending tables, LVTs, timer counters, divide mode, and timer bookkeeping |
| `BiosState` | BIOS lifecycle state |
| BIOS tables | IVT, BDA, EGA state, and BIOS reset vector |

## Explicitly excluded device state

These mutable device fields are not hidden behind the top-level classification:

| State family | Excluded state |
| --- | --- |
| Device queues | AHCI command state, NVMe admin/I/O SQ/CQ state, Virtio avail/used indices, E1000/Virtio-net rings, and deferred DMA work |
| Serial/input buffers | Serial RX/TX/output buffers, PS/2 keyboard/mouse output queue, and pending input IRQ |
| Timers | PIT/HPET live countdowns and host-time progression |
| RNG | `/dev/urandom` handle and fallback generator state |
| Disks | Open file handles, locks, host cache state, and disk bytes; image contents remain external |
| Network topology | Loopback queues, backend handles, link state, and pending packets |
| Guest agent | Event queues and host APIC attachment |
| Hotplug | Pending memory request and device event |
| Acceleration | Native accelerator handles and host capability state |
| Firmware | UEFI runtime/application host pointers and firmware display wiring |

To make an excluded item checkpointable, add it to `VmSnapshot`, its binary
encoder/decoder, restore validation, and this inventory in the same change.
