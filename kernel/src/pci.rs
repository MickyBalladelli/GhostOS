use ghostos_legacy_pc_drivers::PciDevice;

pub(crate) const MAX_PCI_DEVICES: usize = 64;

#[repr(C)]
#[derive(Clone, Copy)]
struct CBar {
    kind: u32,
    address: u64,
    prefetchable: bool,
}

impl CBar {
    const EMPTY: Self = Self {
        kind: 0,
        address: 0,
        prefetchable: false,
    };
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CPciDevice {
    bus: u8,
    device: u8,
    function: u8,
    vendor_id: u16,
    device_id: u16,
    revision: u8,
    programming_interface: u8,
    subclass: u8,
    class_code: u8,
    header_type: u8,
    bars: [CBar; 6],
    interrupt_line: u8,
    interrupt_pin: u8,
}

impl CPciDevice {
    const EMPTY: Self = Self {
        bus: 0,
        device: 0,
        function: 0,
        vendor_id: 0,
        device_id: 0,
        revision: 0,
        programming_interface: 0,
        subclass: 0,
        class_code: 0,
        header_type: 0,
        bars: [CBar::EMPTY; 6],
        interrupt_line: 0,
        interrupt_pin: 0,
    };
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CPciInventory {
    devices: [CPciDevice; MAX_PCI_DEVICES],
    count: usize,
}

impl CPciInventory {
    const EMPTY: Self = Self {
        devices: [CPciDevice::EMPTY; MAX_PCI_DEVICES],
        count: 0,
    };
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
unsafe extern "C" {
    fn ghostos_pci_enumerate_x86(inventory: *mut CPciInventory);
}

#[derive(Clone, Copy)]
pub(crate) struct PciInventory {
    raw: CPciInventory,
}

impl PciInventory {
    pub(crate) const fn empty() -> Self {
        Self {
            raw: CPciInventory::EMPTY,
        }
    }

    pub(crate) const fn len(self) -> usize {
        self.raw.count
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = PciDevice> + '_ {
        self.raw.devices[..self.raw.count]
            .iter()
            .map(from_c_device)
    }

    #[cfg(test)]
    pub(crate) fn push(&mut self, device: PciDevice) {
        assert!(self.raw.count < MAX_PCI_DEVICES);
        self.raw.devices[self.raw.count] = to_c_device(device);
        self.raw.count += 1;
    }
}

fn from_c_device(raw: &CPciDevice) -> PciDevice {
    let address = ghostos_legacy_pc_drivers::pci::PciAddress::new(
        raw.bus,
        raw.device,
        raw.function,
    )
    .expect("C PCI enumerator validates device coordinates");
    let bars = core::array::from_fn(|index| match raw.bars[index].kind {
        1 => ghostos_legacy_pc_drivers::pci::Bar::Io {
            port: raw.bars[index].address as u32,
        },
        2 => ghostos_legacy_pc_drivers::pci::Bar::Memory32 {
            address: raw.bars[index].address as u32,
            prefetchable: raw.bars[index].prefetchable,
        },
        3 => ghostos_legacy_pc_drivers::pci::Bar::Memory64 {
            address: raw.bars[index].address,
            prefetchable: raw.bars[index].prefetchable,
        },
        _ => ghostos_legacy_pc_drivers::pci::Bar::Unused,
    });
    PciDevice {
        address,
        vendor_id: raw.vendor_id,
        device_id: raw.device_id,
        revision: raw.revision,
        programming_interface: raw.programming_interface,
        subclass: raw.subclass,
        class: raw.class_code,
        header_type: raw.header_type,
        bars,
        interrupt_line: raw.interrupt_line,
        interrupt_pin: raw.interrupt_pin,
    }
}

#[cfg(test)]
fn to_c_device(device: PciDevice) -> CPciDevice {
    let bars = core::array::from_fn(|index| match device.bars[index] {
        ghostos_legacy_pc_drivers::pci::Bar::Unused => CBar::EMPTY,
        ghostos_legacy_pc_drivers::pci::Bar::Io { port } => CBar {
            kind: 1,
            address: port as u64,
            prefetchable: false,
        },
        ghostos_legacy_pc_drivers::pci::Bar::Memory32 {
            address,
            prefetchable,
        } => CBar {
            kind: 2,
            address: address as u64,
            prefetchable,
        },
        ghostos_legacy_pc_drivers::pci::Bar::Memory64 {
            address,
            prefetchable,
        } => CBar {
            kind: 3,
            address,
            prefetchable,
        },
    });
    CPciDevice {
        bus: device.address.bus,
        device: device.address.device,
        function: device.address.function,
        vendor_id: device.vendor_id,
        device_id: device.device_id,
        revision: device.revision,
        programming_interface: device.programming_interface,
        subclass: device.subclass,
        class_code: device.class,
        header_type: device.header_type,
        bars,
        interrupt_line: device.interrupt_line,
        interrupt_pin: device.interrupt_pin,
    }
}

/// Enumerate PCI devices before driver services start.
///
/// C owns enumeration and bounded inventory storage. The adapter preserves
/// the existing driver-facing Rust iterator and boot log.
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn discover() -> PciInventory {
    let mut inventory = PciInventory::empty();
    // SAFETY: C writes only the repr(C) inventory supplied here.
    unsafe { ghostos_pci_enumerate_x86(&mut inventory.raw) };
    for device in inventory.iter() {
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
    }
    inventory
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
pub(crate) const fn discover() -> PciInventory {
    PciInventory::empty()
}
