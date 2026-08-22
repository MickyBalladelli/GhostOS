#![no_main]

use libfuzzer_sys::fuzz_target;
use ghostos_vm::{PciDeviceId, PciHostBridge, PortBus};

fuzz_target!(|data: &[u8]| {
    let mut pci = PciHostBridge::new();
    pci.add_device(
        data.first().copied().unwrap_or(0),
        data.get(1).copied().unwrap_or(0) & 0x1f,
        data.get(2).copied().unwrap_or(0) & 0x07,
        PciDeviceId {
            vendor: u16::from(data.get(3).copied().unwrap_or(0)),
            device: u16::from(data.get(4).copied().unwrap_or(0)),
            revision: data.get(5).copied().unwrap_or(0),
            prog_if: data.get(6).copied().unwrap_or(0),
            subclass: data.get(7).copied().unwrap_or(0),
            class: data.get(8).copied().unwrap_or(0),
        },
    );
    let _ = pci.enumerate();
    let bus = data.get(9).copied().unwrap_or(0);
    let device = data.get(10).copied().unwrap_or(0);
    let function = data.get(11).copied().unwrap_or(0);
    let offset = data.get(12).copied().unwrap_or(0);
    let value = pci.read_config(bus, device, function, offset);
    pci.write_config(bus, device, function, offset, value);

    let mut ports = PortBus::new();
    let port = u16::from_le_bytes([
        data.get(13).copied().unwrap_or(0),
        data.get(14).copied().unwrap_or(0),
    ]);
    let size = data.get(15).copied().unwrap_or(1).clamp(1, 8);
    let _ = ports.read(port, size);
    let _ = ports.write(port, value as u64, size);
    ports.reset();
});
