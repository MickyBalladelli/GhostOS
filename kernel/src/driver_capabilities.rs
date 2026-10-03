use ghostos_legacy_pc_drivers::{pci::Bar, PciDevice};
use ghostos_status::{IntoStatus, Status};

use crate::capability::{
    CapabilityError, CapabilityHandle, CapabilityObject, CapabilitySpace, DmaDeviceId,
    PhysicalRange, Rights,
};
use crate::pci::PciInventory;
use crate::task::AddressSpaceId;

pub(crate) const MAX_DRIVER_RESOURCES: usize = 16;
pub(crate) const SERVICE_RESOURCE_MAGIC: u64 = 0x5359_4e4f_4452_5653;
pub(crate) const SERVICE_RESOURCE_VERSION: u32 = 1;
pub(crate) const SERVICE_RESOURCE_STATE_OFFSET: u64 = 8;
pub(crate) const SERVICE_MMIO_STRIDE: u64 = 0x10_000;
/// First byte after the Ring 3 code+stack mapping, rounded up to the MMIO stride
/// so a 64 KiB stack cannot share page-table slots with driver BARs.
pub(crate) const SERVICE_MMIO_BASE: u64 = {
    let stack_end = crate::USER_SPACE_START
        + (crate::SERVICE_CODE_PAGE_COUNT + crate::SERVICE_STACK_PAGE_COUNT) as u64
            * crate::FRAME_SIZE;
    let stride = SERVICE_MMIO_STRIDE;
    ((stack_end + stride - 1) / stride) * stride
};
pub(crate) fn service_mmio_overlaps_image(virtual_address: u64, length: u64) -> bool {
    unsafe { ghostos_service_mmio_overlaps_image(virtual_address, length) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MmioMapping {
    pub physical: PhysicalRange,
    pub virtual_address: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ServiceResource {
    pub kind: u32,
    pub capability: u64,
    pub dma_capability: u64,
    pub physical: u64,
    pub virtual_address: u64,
    pub length: u64,
}

impl ServiceResource {
    const EMPTY: Self = Self {
        kind: 0,
        capability: 0,
        dma_capability: 0,
        physical: 0,
        virtual_address: 0,
        length: 0,
    };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ServiceResourceManifest {
    pub magic: u64,
    pub version: u32,
    pub count: u32,
    pub resources: [ServiceResource; MAX_DRIVER_RESOURCES],
}

impl ServiceResourceManifest {
    pub(crate) const EMPTY: Self = Self {
        magic: SERVICE_RESOURCE_MAGIC,
        version: SERVICE_RESOURCE_VERSION,
        count: 0,
        resources: [ServiceResource::EMPTY; MAX_DRIVER_RESOURCES],
    };
}

#[derive(Clone, Copy)]
pub(crate) struct DriverGrant {
    pub manifest: ServiceResourceManifest,
    pub mappings: [Option<MmioMapping>; MAX_DRIVER_RESOURCES],
    pub mapping_count: usize,
}

impl DriverGrant {
    pub(crate) const EMPTY: Self = Self {
        manifest: ServiceResourceManifest::EMPTY,
        mappings: [None; MAX_DRIVER_RESOURCES],
        mapping_count: 0,
    };

    pub(crate) fn mmio_mappings(&self) -> &[Option<MmioMapping>] {
        &self.mappings[..self.mapping_count]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GrantError {
    Capability(CapabilityError),
    Capacity,
}

impl From<CapabilityError> for GrantError {
    fn from(error: CapabilityError) -> Self {
        Self::Capability(error)
    }
}

impl GrantError {
    pub(crate) fn status(self) -> Status {
        match self {
            Self::Capability(error) => error.status(),
            Self::Capacity => Status::NO_SPACE,
        }
    }
}

pub(crate) fn grant<const CAPACITY: usize>(
    capabilities: &mut CapabilitySpace<CAPACITY>,
    owner: AddressSpaceId,
    role: u8,
    inventory: &PciInventory,
) -> Result<DriverGrant, GrantError> {
    if !matches!(role, 11 | 12 | 13) {
        return Ok(DriverGrant::EMPTY)
    }
    let mut grant = DriverGrant::EMPTY;
    for device in inventory.iter() {
        let native = NativeDevice::from(device);
        let mut candidate = NativeCandidate::EMPTY;
        match unsafe {
            ghostos_driver_select(role, &native, grant.manifest.count,
                grant.mapping_count, &mut candidate)
        } {
            0 => continue,
            1 => {},
            2 => return Err(GrantError::Capacity),
            _ => unreachable!("invalid native driver selection"),
        }
        let physical = candidate.has_mmio.then_some(PhysicalRange {
            start: candidate.physical.start, length: candidate.physical.length,
        });
        let dma_device = DmaDeviceId::new(candidate.device).ok_or(GrantError::Capacity)?;
        let dma_capability = capabilities.mint_root(
            owner,
            CapabilityObject::DmaDevice(dma_device),
            Rights::DMA_READ.union(Rights::DMA_WRITE).union(Rights::DELEGATE),
        )?;

        let (mmio_capability, mapping) = match physical {
            Some(physical) => {
                let mmio_capability = capabilities.mint_mmio(
                    owner,
                    physical,
                    Rights::MAP
                        .union(Rights::READ)
                        .union(Rights::WRITE)
                        .union(Rights::DELEGATE),
                )?;
                let virtual_address = candidate.virtual_address;
                let mapping = MmioMapping {
                    physical,
                    virtual_address,
                };
                (Some(mmio_capability), Some(mapping))
            }
            None => (None, None),
        };

        unsafe {
            ghostos_driver_append_resource(&mut grant.manifest, &candidate,
                dma_capability.raw(), mmio_capability.map_or(0, CapabilityHandle::raw))
        };
        if let Some(mapping) = mapping {
            grant.mappings[grant.mapping_count] = Some(mapping);
            grant.mapping_count += 1;
        }
    }
    Ok(grant)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeBar { kind: u32, address: u64 }

#[repr(C)]
struct NativeDevice {
    bus: u8,
    device: u8,
    function: u8,
    vendor_id: u16,
    device_id: u16,
    programming_interface: u8,
    subclass: u8,
    class_code: u8,
    bars: [NativeBar; 6],
}

impl From<PciDevice> for NativeDevice {
    fn from(device: PciDevice) -> Self {
        Self {
            bus: device.address.bus, device: device.address.device,
            function: device.address.function, vendor_id: device.vendor_id,
            device_id: device.device_id, programming_interface: device.programming_interface,
            subclass: device.subclass, class_code: device.class,
            bars: device.bars.map(|bar| match bar {
                Bar::Unused => NativeBar { kind: 0, address: 0 },
                Bar::Io { port } => NativeBar { kind: 1, address: port as u64 },
                Bar::Memory32 { address, .. } => NativeBar { kind: 2, address: address as u64 },
                Bar::Memory64 { address, .. } => NativeBar { kind: 3, address },
            }),
        }
    }
}

#[repr(C)]
struct NativeRange { start: u64, length: u64 }

#[repr(C)]
struct NativeCandidate {
    kind: u32,
    device: u32,
    physical: NativeRange,
    virtual_address: u64,
    has_mmio: bool,
}

impl NativeCandidate {
    const EMPTY: Self = Self {
        kind: 0, device: 0, physical: NativeRange { start: 0, length: 0 },
        virtual_address: 0, has_mmio: false,
    };
}

const _: () = {
    assert!(core::mem::size_of::<NativeDevice>() == 112);
    assert!(core::mem::offset_of!(NativeDevice, bars) == 16);
    assert!(core::mem::size_of::<NativeCandidate>() == 40);
    assert!(core::mem::size_of::<ServiceResource>() == 48);
    assert!(core::mem::size_of::<ServiceResourceManifest>() == 784);
    assert!(crate::SERVICE_CODE_PAGE_COUNT == 19);
    assert!(crate::SERVICE_STACK_PAGE_COUNT == 16);
    assert!(crate::FRAME_SIZE == 4096);
    assert!(crate::USER_SPACE_START == 0x0000_0080_0000_0000);
    assert!(SERVICE_MMIO_BASE == 0x0000_0080_0003_0000);
};

unsafe extern "C" {
    fn ghostos_service_mmio_overlaps_image(address: u64, length: u64) -> bool;
    fn ghostos_driver_select(role: u8, device: *const NativeDevice, count: u32,
        mapping_count: usize, candidate: *mut NativeCandidate) -> u32;
    fn ghostos_driver_append_resource(manifest: *mut ServiceResourceManifest,
        candidate: *const NativeCandidate, dma_capability: u64, mmio_capability: u64);
}
