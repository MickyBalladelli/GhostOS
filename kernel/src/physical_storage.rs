//! Early AHCI system-volume mount used to bootstrap the Ring 3 storage stack.

use crate::pci::PciInventory;

const STORAGE_DEVICE_CAPACITY: usize = 64;

#[repr(C)]
#[derive(Clone, Copy)]
struct CStoragePciDevice {
    bus: u8,
    device: u8,
    function: u8,
    class_code: u8,
    subclass: u8,
    programming_interface: u8,
    bar5_kind: u8,
    bar5_address: u64,
}

impl CStoragePciDevice {
    const EMPTY: Self = Self {
        bus: 0,
        device: 0,
        function: 0,
        class_code: 0,
        subclass: 0,
        programming_interface: 0,
        bar5_kind: 0,
        bar5_address: 0,
    };
}

#[allow(unsafe_code)]
unsafe extern "C" {
    fn ghostos_physical_storage_missing_volume(
        devices: *const CStoragePciDevice,
        count: usize,
        mounted: bool,
    ) -> bool;
    fn ghostos_physical_storage_next_ahci(
        devices: *const CStoragePciDevice,
        count: usize,
        start: usize,
    ) -> usize;
}

#[allow(unsafe_code)]
fn storage_devices(inventory: &PciInventory) -> ([CStoragePciDevice; STORAGE_DEVICE_CAPACITY], usize) {
    let mut devices = [CStoragePciDevice::EMPTY; STORAGE_DEVICE_CAPACITY];
    let mut count = 0;
    for device in inventory.iter().take(STORAGE_DEVICE_CAPACITY) {
        let (bar5_kind, bar5_address) = match device.bars[5] {
            ghostos_legacy_pc_drivers::Bar::Memory32 { address, .. } => (2, address as u64),
            ghostos_legacy_pc_drivers::Bar::Memory64 { address, .. } => (3, address),
            _ => (0, 0),
        };
        devices[count] = CStoragePciDevice {
            bus: device.address.bus,
            device: device.address.device,
            function: device.address.function,
            class_code: device.class,
            subclass: device.subclass,
            programming_interface: device.programming_interface,
            bar5_kind,
            bar5_address,
        };
        count += 1;
    }
    (devices, count)
}

pub(crate) fn missing_expected_system_volume(inventory: &PciInventory, mounted: bool) -> bool {
    let (devices, count) = storage_devices(inventory);
    // SAFETY: the C policy reads only the initialized entries within count.
    unsafe { ghostos_physical_storage_missing_volume(devices.as_ptr(), count, mounted) }
}

#[cfg(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi")))]
mod platform {
    use super::{ghostos_physical_storage_next_ahci, storage_devices};
    use core::mem::MaybeUninit;
    use core::sync::atomic::{AtomicUsize, Ordering};

    use ghostos_legacy_pc_drivers::pci::{ConfigAccess, PortConfig};
    use ghostos_legacy_pc_drivers::storage::{
        AhciCommandList, AhciCommandTable, AhciController, AhciPort,
    };
    use ghostos_legacy_pc_drivers::AhciBlockDevice;
    use ghostos_ghostfs::{
        MountedSystemVolume, ServiceManifest, SynFs, SystemDiskManifest, SYSTEM_DISK_MANIFEST_BYTES,
        SYSTEM_VOLUME_BLOCKS,
    };

    use crate::pci::PciInventory;
    use crate::arch::paging::SERVICE_IMAGE_BYTES;

    const SPIN_LIMIT: usize = 20_000_000;

    #[repr(C, align(256))]
    struct Fis([u8; 256]);
    #[repr(C, align(4096))]
    struct Dma([u8; 4096]);
    #[repr(C, align(4096))]
    struct Manifest([u8; SYSTEM_DISK_MANIFEST_BYTES]);
    #[repr(C, align(4096))]
    struct Volume([u8; SynFs::<SYSTEM_VOLUME_BLOCKS>::volume_bytes()]);
    #[repr(C, align(4096))]
    struct ServiceImage([u8; SERVICE_IMAGE_BYTES]);

    static mut COMMAND_LIST: AhciCommandList = AhciCommandList::new();
    static mut COMMAND_TABLE: AhciCommandTable = AhciCommandTable::new();
    static mut RECEIVED_FIS: Fis = Fis([0; 256]);
    static mut DMA: Dma = Dma([0; 4096]);
    static mut MANIFEST: Manifest = Manifest([0; SYSTEM_DISK_MANIFEST_BYTES]);
    static mut VOLUME: Volume = Volume([0; SynFs::<SYSTEM_VOLUME_BLOCKS>::volume_bytes()]);
    static mut ACTIVE_PORT: MaybeUninit<AhciPort> = MaybeUninit::uninit();
    static mut ACTIVE_MANIFEST: MaybeUninit<SystemDiskManifest> = MaybeUninit::uninit();
    static ACTIVE_VOLUME_READY: AtomicUsize = AtomicUsize::new(0);
    static mut SERVICE_IMAGES: [ServiceImage; 15] =
        [const { ServiceImage([0; SERVICE_IMAGE_BYTES]) }; 15];
    static SERVICE_LENGTHS: [AtomicUsize; 15] = [const { AtomicUsize::new(0) }; 15];

    pub fn mount(inventory: &PciInventory) -> Option<SynFs<SYSTEM_VOLUME_BLOCKS>> {
        let (devices, count) = storage_devices(inventory);
        let mut start = 0;
        loop {
            // SAFETY: the C selector reads only initialized records in this array.
            let index = unsafe {
                ghostos_physical_storage_next_ahci(devices.as_ptr(), count, start)
            };
            if index >= count {
                break
            }
            start = index + 1;
            let device = devices[index];
            let mut config = PortConfig;
            let address = ghostos_legacy_pc_drivers::pci::PciAddress::new(
                device.bus,
                device.device,
                device.function,
            )?;
            let command = unsafe { config.read_u32(address, 0x04) };
            unsafe { config.write_u32(address, 0x04, command | 0x6) };
            let Some(filesystem) = (unsafe { mount_controller(device.bar5_address as usize) }) else {
                continue
            };
            crate::println!("physical GhostFS mounted from AHCI {:02x}:{:02x}.{}",
                device.bus, device.device, device.function);
            return Some(filesystem)
        }
        crate::println!("no mountable AHCI GhostOS system volume; using bootstrap filesystem");
        None
    }

    unsafe fn mount_controller(registers: usize) -> Option<SynFs<SYSTEM_VOLUME_BLOCKS>> {
        let mut controller = unsafe { AhciController::new(registers).ok()? };
        controller.enable();
        for index in 0..32 {
            let Some(mut port) = controller.port(index) else {
                continue
            };
            if !port.has_sata_device() {
                crate::println!("AHCI port {index}: no SATA device");
                continue
            }
            if let Err(error) = port.stop(SPIN_LIMIT) {
                crate::println!("AHCI port {index}: stop failed {error:?}");
                continue
            }
            unsafe {
                core::ptr::write_bytes((&raw mut RECEIVED_FIS.0).cast::<u8>(), 0, 256);
                core::ptr::write_bytes(
                    (&raw mut COMMAND_LIST).cast::<u8>(),
                    0,
                    core::mem::size_of::<AhciCommandList>(),
                );
                core::ptr::write_bytes(
                    (&raw mut COMMAND_TABLE).cast::<u8>(),
                    0,
                    core::mem::size_of::<AhciCommandTable>(),
                );
            }
            let (command_list_physical, received_fis_physical) = unsafe {
                (
                    (&raw const COMMAND_LIST) as u64,
                    (&raw const RECEIVED_FIS.0) as u64,
                )
            };
            if let Err(error) = port.configure(command_list_physical, received_fis_physical) {
                crate::println!("AHCI port {index}: configure failed {error:?}");
                continue
            }
            port.start();
            let mut block = unsafe {
                AhciBlockDevice::new(
                    &mut port,
                    &mut *(&raw mut COMMAND_LIST),
                    &mut *(&raw mut COMMAND_TABLE),
                    (&raw const COMMAND_TABLE) as u64,
                    &mut *(&raw mut DMA.0),
                    (&raw const DMA.0) as u64,
                    512,
                    SPIN_LIMIT,
                )
                .ok()?
            };
            let mounted = unsafe {
                match MountedSystemVolume::mount(
                    &mut block,
                    &mut *(&raw mut MANIFEST.0),
                    &mut *(&raw mut VOLUME.0),
                ) {
                    Ok(mounted) => Some(mounted),
                    Err(error) => {
                        crate::println!("AHCI port {index}: system volume mount failed {error:?}");
                        None
                    }
                }
            };
            let Some(mounted) = mounted else {
                let _ = port.stop(SPIN_LIMIT);
                continue
            };
            let MountedSystemVolume {
                manifest,
                filesystem,
                volume,
            } = mounted;
            drop(volume);
            drop(block);
            if unsafe { load_service_images(&filesystem) }.is_none() {
                crate::println!("AHCI port {index}: service image load failed");
                let _ = port.stop(SPIN_LIMIT);
                continue
            }
            unsafe {
                (&mut *core::ptr::addr_of_mut!(ACTIVE_PORT)).write(port);
                (&mut *core::ptr::addr_of_mut!(ACTIVE_MANIFEST)).write(manifest);
                ACTIVE_VOLUME_READY.store(1, Ordering::Release);
            }
            return Some(filesystem)
        }
        None
    }

    pub fn sync(filesystem: &mut SynFs<SYSTEM_VOLUME_BLOCKS>) -> Result<(), ()> {
        if ACTIVE_VOLUME_READY.load(Ordering::Acquire) == 0 {
            return Err(())
        }
        let manifest = unsafe {
            (&*core::ptr::addr_of!(ACTIVE_MANIFEST)).assume_init_ref()
        };
        let mut block = unsafe {
            AhciBlockDevice::new(
                (&mut *core::ptr::addr_of_mut!(ACTIVE_PORT)).assume_init_mut(),
                &mut *core::ptr::addr_of_mut!(COMMAND_LIST),
                &mut *core::ptr::addr_of_mut!(COMMAND_TABLE),
                (&raw const COMMAND_TABLE) as u64,
                &mut *core::ptr::addr_of_mut!(DMA.0),
                (&raw const DMA.0) as u64,
                512,
                SPIN_LIMIT,
            )
            .map_err(|_| ())?
        };
        let mut volume =
            ghostos_ghostfs::SystemDiskVolume::new(&mut block, *manifest).map_err(|_| ())?;
        filesystem.fsync(&mut volume).map(|_| ()).map_err(|_| ())
    }

    unsafe fn load_service_images(filesystem: &SynFs<SYSTEM_VOLUME_BLOCKS>) -> Option<()> {
        let mut scratch = [0; 2048];
        let manifest = match ServiceManifest::load(filesystem, &mut scratch) {
            Ok(manifest) => manifest,
            Err(error) => {
                crate::println!("service manifest load failed {error:?}");
                return None
            }
        };
        for entry in manifest.entries() {
            let role = entry.role as usize;
            if role >= SERVICE_LENGTHS.len() || entry.image_bytes as usize > SERVICE_IMAGE_BYTES {
                crate::println!("service package bounds failed role={role}");
                return None
            }
            let image = unsafe { &mut *(&raw mut SERVICE_IMAGES[role].0) };
            let length = match manifest.load_package(filesystem, entry.role, image) {
                Ok(length) => length,
                Err(error) => {
                    crate::println!("service package load failed role={role} error={error:?}");
                    return None
                }
            };
            SERVICE_LENGTHS[role].store(length, Ordering::Release)
        }
        Some(())
    }

    pub fn service_image(role: u8) -> Option<&'static [u8]> {
        let role = role as usize;
        let length = SERVICE_LENGTHS.get(role)?.load(Ordering::Acquire);
        if length == 0 {
            return None
        }
        unsafe {
            Some(core::slice::from_raw_parts(
                (&raw const SERVICE_IMAGES[role].0).cast::<u8>(),
                length,
            ))
        }
    }
}

#[cfg(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi")))]
pub use platform::{mount, service_image, sync};

#[cfg(not(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi"))))]
pub fn mount(
    _inventory: &crate::pci::PciInventory,
) -> Option<ghostos_ghostfs::SynFs<{ ghostos_ghostfs::SYSTEM_VOLUME_BLOCKS }>> {
    None
}

#[cfg(not(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi"))))]
#[allow(dead_code)]
pub const fn service_image(_role: u8) -> Option<&'static [u8]> {
    None
}

#[cfg(test)]
mod tests {
    use ghostos_legacy_pc_drivers::{Bar, PciAddress, PciDevice};

    use super::missing_expected_system_volume;
    use crate::pci::PciInventory;

    fn ahci_device() -> PciDevice {
        PciDevice {
            address: PciAddress::new(0, 31, 2).expect("valid PCI address"),
            vendor_id: 0x8086,
            device_id: 0x2922,
            revision: 0,
            programming_interface: 0x01,
            subclass: 0x06,
            class: 0x01,
            header_type: 0,
            bars: [Bar::Unused; 6],
            interrupt_line: 0,
            interrupt_pin: 0,
        }
    }

    #[test]
    fn missing_volume_is_ignored_without_ahci() {
        assert!(!missing_expected_system_volume(&PciInventory::empty(), false));
        assert!(!missing_expected_system_volume(&PciInventory::empty(), true));
    }

    #[test]
    fn missing_volume_is_fatal_when_ahci_is_present() {
        let mut inventory = PciInventory::empty();
        inventory.push(ahci_device());
        assert!(missing_expected_system_volume(&inventory, false));
        assert!(!missing_expected_system_volume(&inventory, true));
    }
}
