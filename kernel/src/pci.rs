#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
use ghostos_legacy_pc_drivers::pci::{enumerate, PortConfig};

use ghostos_legacy_pc_drivers::PciDevice;

pub(crate) const MAX_PCI_DEVICES: usize = 64;

#[derive(Clone, Copy)]
pub(crate) struct PciInventory {
    #[allow(dead_code)]
    devices: [Option<PciDevice>; MAX_PCI_DEVICES],
    count: usize,
}

impl PciInventory {
    pub(crate) const fn empty() -> Self {
        Self {
            devices: [None; MAX_PCI_DEVICES],
            count: 0,
        }
    }

    pub(crate) const fn len(self) -> usize {
        self.count
    }

    #[allow(dead_code)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = PciDevice> + '_ {
        self.devices[..self.count].iter().flatten().copied()
    }

    #[cfg(test)]
    pub(crate) fn push(&mut self, device: PciDevice) {
        assert!(self.count < MAX_PCI_DEVICES);
        self.devices[self.count] = Some(device);
        self.count += 1;
    }
}

/// Enumerate PCI devices before driver services start.
///
/// The driver crate keeps this scan heap-free. The kernel records the result
/// in the boot log so later driver services can bind to the same hardware
/// inventory and operators can see what firmware exposed.
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn discover() -> PciInventory {
    let mut config = PortConfig;
    let mut inventory = PciInventory::empty();
    enumerate(&mut config, |device| {
        if inventory.count == MAX_PCI_DEVICES {
            return
        }
        inventory.devices[inventory.count] = Some(device);
        inventory.count += 1;
        crate::println!(
            "PCI {:02x}:{:02x}.{} vendor={:04x} device={:04x} class={:02x}:{:02x}.{:02x}",
            device.address.bus,
            device.address.device,
            device.address.function,
            device.vendor_id,
            device.device_id,
            device.class,
            device.subclass,
            device.programming_interface,
        )
    });
    inventory
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
pub(crate) const fn discover() -> PciInventory {
    PciInventory::empty()
}
