# Device emulation guide

The VM uses two small traits:

```rust
pub trait Device {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError>
    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError>
    fn reset(&mut self)
}

pub trait PortDevice {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError>
    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError>
    fn reset(&mut self)
}
```

Use `Device` for MMIO registers. Use `PortDevice` for `in`/`out` registers.
The MMU dispatches MMIO accesses by address. `PortBus` dispatches port accesses
by the attached base and size. Unclaimed port reads return all ones and
unclaimed writes are ignored.

## DMA rule

Do not borrow guest RAM from inside a device callback when the CPU executor is
already borrowing the MMU. Queue the request in the device, then process it in
the VM's device-poll phase. Storage and NIC devices follow this pattern:

```text
guest writes command registers
        |
        v
device records pending work
        |
        v
Vm::poll_devices borrows device + MMU
        |
        v
device reads/writes guest physical memory and raises an interrupt
```

This keeps Rust aliasing rules clear and makes device completion happen at a
stable boundary between CPU dispatches.

## Existing device families

- PCI devices expose a `PciDeviceId`, configuration space, and BAR metadata
  through `PciHostBridge`.
- AHCI and NVMe use `DiskImage` for 512-byte sector I/O. RAW, fixed VHD, and
  QCOW2 images are recognized.
- e1000 and virtio-net use the `NetBackend` trait. The default VM connects both
  NICs to a two-port in-memory loopback hub.
- Legacy Virtio block, console, and RNG devices use port BARs and split
  virtqueues.
- Timers signal the shared local APIC. The VM delivers maskable interrupts
  only when guest IF is set and the STI shadow has ended.

## Register behavior

Keep register offsets and access widths explicit. Return
`DeviceError::UnsupportedSize` for widths the guest hardware contract does not
allow. Reset should clear guest-visible state but preserve permanent wiring,
such as an attached APIC or backend, unless the hardware reset contract says
otherwise.

For a register with side effects, implement the side effect in the device, not
in the bus. For example, reading a UART interrupt register should clear the
UART's pending interrupt state, while the bus should only route the read.
