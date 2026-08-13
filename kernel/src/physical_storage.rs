//! Early AHCI system-volume mount used to bootstrap the Ring 3 storage stack.

#[cfg(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi")))]
mod platform {
    use core::mem::MaybeUninit;
    use core::sync::atomic::{AtomicUsize, Ordering};

    use synos_legacy_pc_drivers::pci::{ConfigAccess, PortConfig};
    use synos_legacy_pc_drivers::storage::{
        AhciCommandList, AhciCommandTable, AhciController, AhciPort,
    };
    use synos_legacy_pc_drivers::AhciBlockDevice;
    use synos_synfs::{
        MountedSystemVolume, ServiceManifest, SynFs, SystemDiskManifest, SYSTEM_DISK_MANIFEST_BYTES,
        SYSTEM_VOLUME_BLOCKS,
    };

    use crate::pci::PciInventory;

    const SPIN_LIMIT: usize = 20_000_000;
    const SERVICE_IMAGE_BYTES: usize = 16 * 1024;

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
        for device in inventory.iter().filter(|device| device.is_ahci()) {
            let Some(registers) = device.bars[5].memory_address() else {
                continue
            };
            let mut config = PortConfig;
            let command = unsafe { config.read_u32(device.address, 0x04) };
            unsafe { config.write_u32(device.address, 0x04, command | 0x6) };
            let Some(filesystem) = (unsafe { mount_controller(registers as usize) }) else {
                continue
            };
            crate::println!("physical SynFS mounted from AHCI {:02x}:{:02x}.{}",
                device.address.bus, device.address.device, device.address.function);
            return Some(filesystem)
        }
        crate::println!("no mountable AHCI SynOS system volume; using bootstrap filesystem");
        None
    }

    unsafe fn mount_controller(registers: usize) -> Option<SynFs<SYSTEM_VOLUME_BLOCKS>> {
        let mut controller = unsafe { AhciController::new(registers).ok()? };
        controller.enable();
        for index in 0..32 {
            let Some(mut port) = controller.port(index) else {
                continue
            };
            if !port.has_sata_device() || port.stop(SPIN_LIMIT).is_err() {
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
            if port
                .configure(command_list_physical, received_fis_physical)
                .is_err()
            {
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
                MountedSystemVolume::mount(
                    &mut block,
                    &mut *(&raw mut MANIFEST.0),
                    &mut *(&raw mut VOLUME.0),
                )
                .ok()
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
            synos_synfs::SystemDiskVolume::new(&mut block, *manifest).map_err(|_| ())?;
        filesystem.fsync(&mut volume).map(|_| ()).map_err(|_| ())
    }

    unsafe fn load_service_images(filesystem: &SynFs<SYSTEM_VOLUME_BLOCKS>) -> Option<()> {
        let mut scratch = [0; 2048];
        let manifest = ServiceManifest::load(filesystem, &mut scratch).ok()?;
        for entry in manifest.entries() {
            let role = entry.role as usize;
            if role >= SERVICE_LENGTHS.len() || entry.image_bytes as usize > SERVICE_IMAGE_BYTES {
                return None
            }
            let image = unsafe { &mut *(&raw mut SERVICE_IMAGES[role].0) };
            let length = manifest.load_package(filesystem, entry.role, image).ok()?;
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
) -> Option<synos_synfs::SynFs<{ synos_synfs::SYSTEM_VOLUME_BLOCKS }>> {
    None
}

#[cfg(not(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi"))))]
pub fn sync(
    _filesystem: &mut synos_synfs::SynFs<{ synos_synfs::SYSTEM_VOLUME_BLOCKS }>,
) -> Result<(), ()> {
    Err(())
}

#[cfg(not(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi"))))]
#[allow(dead_code)]
pub const fn service_image(_role: u8) -> Option<&'static [u8]> {
    None
}
