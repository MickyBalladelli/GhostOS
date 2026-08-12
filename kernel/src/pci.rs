#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
use synos_legacy_pc_drivers::pci::{enumerate, PortConfig};

/// Enumerate PCI devices before driver services start.
///
/// The driver crate keeps this scan heap-free. The kernel records the result
/// in the boot log so later driver services can bind to the same hardware
/// inventory and operators can see what firmware exposed.
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn discover() -> usize {
    let mut config = PortConfig;
    let mut count = 0;
    enumerate(&mut config, |device| {
        count += 1;
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
    count
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
pub(crate) const fn discover() -> usize {
    0
}
