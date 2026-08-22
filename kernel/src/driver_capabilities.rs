use ghostos_legacy_pc_drivers::{EthernetAdapter, EthernetKind, PciDevice};
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
pub(crate) const SERVICE_MMIO_BASE: u64 = crate::USER_SPACE_START + 0x20_000;
pub(crate) const SERVICE_MMIO_STRIDE: u64 = 0x10_000;
const PAGE_SIZE: u64 = crate::FRAME_SIZE;

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

#[derive(Clone, Copy)]
enum DriverKind {
    Ahci,
    Nvme,
    Ethernet,
}

impl DriverKind {
    fn from_role(role: u8) -> Option<Self> {
        match role {
            11 => Some(Self::Ahci),
            12 => Some(Self::Nvme),
            13 => Some(Self::Ethernet),
            _ => None,
        }
    }

    const fn resource_kind(self) -> u32 {
        match self {
            Self::Ahci => 1,
            Self::Nvme => 2,
            Self::Ethernet => 3,
        }
    }
}

pub(crate) fn grant<const CAPACITY: usize>(
    capabilities: &mut CapabilitySpace<CAPACITY>,
    owner: AddressSpaceId,
    role: u8,
    inventory: &PciInventory,
) -> Result<DriverGrant, GrantError> {
    let Some(kind) = DriverKind::from_role(role) else {
        return Ok(DriverGrant::EMPTY)
    };

    let mut grant = DriverGrant::EMPTY;
    for device in inventory.iter() {
        let Some(bar) = matching_bar(kind, &device) else {
            continue
        };
        if grant.manifest.count as usize == MAX_DRIVER_RESOURCES {
            return Err(GrantError::Capacity)
        }

        let physical = match bar {
            Some((address, length)) => {
                let Some(physical) = PhysicalRange::new(address, length)
                    .filter(|range| range.start % PAGE_SIZE == 0 && range.length % PAGE_SIZE == 0)
                else {
                    continue
                };
                Some(physical)
            }
            None => None,
        };
        let dma_device = DmaDeviceId::new(device_id(device)).ok_or(GrantError::Capacity)?;
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
                let index = grant.mapping_count as u64;
                let virtual_address = SERVICE_MMIO_BASE + index * SERVICE_MMIO_STRIDE;
                let mapping = MmioMapping {
                    physical,
                    virtual_address,
                };
                (Some(mmio_capability), Some(mapping))
            }
            None => (None, None),
        };

        let resource_index = grant.manifest.count as usize;
        grant.manifest.resources[resource_index] = ServiceResource {
            kind: kind.resource_kind(),
            capability: mmio_capability.map_or(0, CapabilityHandle::raw),
            dma_capability: dma_capability.raw(),
            physical: physical.map_or(0, |physical| physical.start),
            virtual_address: mapping.map_or(0, |mapping| mapping.virtual_address),
            length: mapping.map_or(0, |mapping| mapping.physical.length),
        };
        grant.manifest.count += 1;
        if let Some(mapping) = mapping {
            grant.mappings[grant.mapping_count] = Some(mapping);
            grant.mapping_count += 1;
        }
    }
    Ok(grant)
}

fn matching_bar(kind: DriverKind, device: &PciDevice) -> Option<Option<(u64, u64)>> {
    match kind {
        DriverKind::Ahci if device.is_ahci() => {
            Some(device.bars[5].memory_address().map(|address| (address, 0x2000)))
        }
        DriverKind::Nvme if device.is_nvme() => {
            Some(device.bars[0].memory_address().map(|address| (address, 0x1000)))
        }
        DriverKind::Ethernet => {
            let adapter = EthernetAdapter::from_pci(device)?;
            let length = match adapter.kind {
                EthernetKind::IntelE1000 => 0x4000,
                EthernetKind::RealtekRtl8169 => 0x1000,
                EthernetKind::VirtioNet => return Some(None),
            };
            Some(adapter.registers.memory_address().map(|address| (address, length)))
        }
        _ => None,
    }
}

fn device_id(device: PciDevice) -> u32 {
    1 + ((device.address.bus as u32) << 16)
        + ((device.address.device as u32) << 8)
        + device.address.function as u32
}
