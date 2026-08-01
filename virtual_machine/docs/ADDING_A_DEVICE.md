# Adding a device

This is the shortest safe path for adding a guest-visible device.

## 1. Pick the transport

Choose MMIO (`Device`) or port I/O (`PortDevice`). If the guest driver expects
PCI discovery, also create a PCI function and assign a BAR. Keep the address
and interrupt vector in a named constant.

## 2. Add the module

Place the implementation in the closest existing family:

```text
src/devices/clock.rs
src/devices/storage/my_controller.rs
src/devices/net/my_nic.rs
```

Declare it in `src/devices/mod.rs` or the relevant child module, then re-export
the public type and constants. Add a module-level comment that states the
guest interface, supported commands, and DMA behavior.

## 3. Implement the device contract

Start with explicit reset state and narrow register access. Use
`DeviceError::InvalidAddress` for offsets outside the device aperture and
`DeviceError::UnsupportedSize` for invalid widths. Keep host-only objects such
as files, callbacks, and network backends out of snapshot state unless they
have a deliberate serialization format.

For DMA devices, store a pending command and add a poll method such as
`poll_dma(&mut self, mmu: &mut Mmu)`. The VM must call it from
`Vm::poll_devices` after CPU execution.

## 4. Wire it into `Vm::with_config`

The normal wiring sequence is:

```rust
let device = Rc::new(RefCell::new(MyDevice::new()));
device.borrow_mut().attach_apic(apic.clone());
mmu.attach_mmio(MY_MMIO_BASE, MY_MMIO_SIZE, Box::new(device.clone()));
ports.attach(MY_IO_BASE, MY_IO_SIZE, Box::new(device.clone()));
pci.borrow_mut().add_device(0, 11, 0, MY_PCI_ID);
```

Use only the transport that the device actually implements. Add a public
accessor on `Vm` if host-side tests or tools need to inspect or configure the
device after construction.

## 5. Add device polling and reset

If the device has deferred work, call its poll method once per VM step. Add it
to `Vm::reset`. Preserve stable wiring on reset: the APIC connection, PCI
identity, and an intentionally attached backend should remain available.

## 6. Verify the guest contract

Use a focused guest or unit-level check for:

- reset values and register access widths;
- PCI vendor/device IDs and BAR sizing;
- queue setup, malformed descriptors, and completion interrupts;
- DMA bounds and missing-backing-store errors;
- interrupt masking, EOI, and reset behavior.

Keep the check bounded with `Vm::run_for_steps` when booting a guest. The
integration command is also useful for confirming that a new device does not
break kernel boot, paging, scheduler, capabilities, or IPC.
