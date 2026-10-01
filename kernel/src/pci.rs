#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
use ghostos_legacy_pc_drivers::PciDevice;

pub(crate) const MAX_PCI_DEVICES: usize = 64;

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
#[repr(C)]
struct CBar {
    kind: u32,
    address: u64,
    prefetchable: bool,
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
#[repr(C)]
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

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
unsafe extern "C" {
    fn ghostos_pci_enumerate_x86(
        visit: extern "C" fn(*mut core::ffi::c_void, *const CPciDevice),
        context: *mut core::ffi::c_void,
    ) -> usize;
}

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
    let mut inventory = PciInventory::empty();
    // SAFETY: enumeration is boot-time and invokes the callback synchronously.
    unsafe {
        ghostos_pci_enumerate_x86(collect_device, (&mut inventory as *mut PciInventory).cast());
    }
    inventory
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
extern "C" fn collect_device(context: *mut core::ffi::c_void, raw: *const CPciDevice) {
    if context.is_null() || raw.is_null() {
        return
    }
    // SAFETY: C provides the live inventory pointer and a complete PCI record.
    let inventory = unsafe { &mut *context.cast::<PciInventory>() };
    if inventory.count == MAX_PCI_DEVICES {
        return
    }
    // SAFETY: raw points to a stack record valid for this synchronous callback.
    let raw = unsafe { &*raw };
    let address = ghostos_legacy_pc_drivers::pci::PciAddress::new(raw.bus, raw.device, raw.function)
        .expect("C PCI enumerator validates device coordinates");
    let bars = core::array::from_fn(|index| match raw.bars[index].kind {
        1 => ghostos_legacy_pc_drivers::pci::Bar::Io { port: raw.bars[index].address as u32 },
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
    let device = PciDevice {
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
    };
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
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
pub(crate) const fn discover() -> PciInventory {
    PciInventory::empty()
}
