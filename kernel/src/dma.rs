use crate::capability::{
    CapabilityHandle, CapabilityObject, CapabilitySpace, DmaDeviceId, PhysicalRange, Rights,
};
use crate::task::AddressSpaceId;

pub const MAX_DMA_MAPPINGS: usize = 256;
pub const DMA_PAGE_SIZE: u64 = crate::FRAME_SIZE;
const FIRST_IOVA: u64 = DMA_PAGE_SIZE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DmaPermissions(u8);

impl DmaPermissions {
    pub const NONE: Self = Self(0);
    pub const DEVICE_READ: Self = Self(1 << 0);
    pub const DEVICE_WRITE: Self = Self(1 << 1);

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    fn required_rights(self) -> Option<Rights> {
        let mut rights = Rights::NONE;
        if self.contains(Self::DEVICE_READ) {
            rights = rights.union(Rights::DMA_READ)
        }
        if self.contains(Self::DEVICE_WRITE) {
            rights = rights.union(Rights::DMA_WRITE)
        }
        if rights.is_empty() { None } else { Some(rights) }
    }
}

pub trait Iommu {
    /// Install a device-scoped translation. Returning false rejects the map.
    fn map(
        &mut self,
        device: DmaDeviceId,
        iova: u64,
        physical: PhysicalRange,
        permissions: DmaPermissions,
    ) -> bool;

    /// Remove a device-scoped translation. Returning false keeps the mapping.
    fn unmap(&mut self, device: DmaDeviceId, iova: u64, length: u64) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DmaMapping {
    pub id: u64,
    pub device: DmaDeviceId,
    pub iova: u64,
    pub physical: PhysicalRange,
    pub permissions: DmaPermissions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmaError {
    InvalidRange,
    InvalidPermissions,
    InvalidCapability,
    AccessDenied,
    Capacity,
    IovaExhausted,
    IommuRejected,
    MappingNotFound,
}

#[derive(Clone, Copy)]
struct DmaRecord {
    mapping: DmaMapping,
    owner: AddressSpaceId,
    device_authority: CapabilityHandle,
    buffer_authority: CapabilityHandle,
}

pub struct DmaManager<const CAPACITY: usize = MAX_DMA_MAPPINGS> {
    next_id: u64,
    next_iova: u64,
    mappings: [Option<DmaRecord>; CAPACITY],
}

impl<const CAPACITY: usize> DmaManager<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            next_id: 1,
            next_iova: FIRST_IOVA,
            mappings: [None; CAPACITY],
        }
    }

    /// Map a buffer only when the caller owns both the device authority and
    /// the physical-buffer authority. The IOMMU backend must approve first.
    pub fn map<const CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        caller: AddressSpaceId,
        device_authority: CapabilityHandle,
        buffer_authority: CapabilityHandle,
        device: DmaDeviceId,
        offset: u64,
        length: u64,
        permissions: DmaPermissions,
        iommu: &mut impl Iommu,
    ) -> Result<DmaMapping, DmaError> {
        validate_range(offset, length)?;
        let required = permissions
            .required_rights()
            .ok_or(DmaError::InvalidPermissions)?;
        capabilities
            .authorize(
                caller,
                device_authority,
                CapabilityObject::DmaDevice(device),
                required,
            )
            .map_err(|_| DmaError::InvalidCapability)?;
        let buffer = capabilities
            .inspect(caller, buffer_authority)
            .map_err(|_| DmaError::InvalidCapability)?;
        if !buffer.rights.contains(Rights::MAP.union(required)) {
            return Err(DmaError::AccessDenied)
        }
        let backing = buffer.backing.ok_or(DmaError::InvalidCapability)?;
        if backing.start % DMA_PAGE_SIZE != 0 {
            return Err(DmaError::InvalidCapability)
        }
        let physical_start = backing
            .start
            .checked_add(offset)
            .ok_or(DmaError::InvalidRange)?;
        let physical = PhysicalRange::new(physical_start, length)
            .filter(|range| backing.contains(*range))
            .ok_or(DmaError::AccessDenied)?;
        let slot = self
            .mappings
            .iter()
            .position(Option::is_none)
            .ok_or(DmaError::Capacity)?;
        let iova = align_up(self.next_iova)?;
        let next = iova.checked_add(length).ok_or(DmaError::IovaExhausted)?;
        if !iommu.map(device, iova, physical, permissions) {
            return Err(DmaError::IommuRejected)
        }
        let mapping = DmaMapping {
            id: self.next_id,
            device,
            iova,
            physical,
            permissions,
        };
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.next_iova = next;
        self.mappings[slot] = Some(DmaRecord {
            mapping,
            owner: caller,
            device_authority,
            buffer_authority,
        });
        Ok(mapping)
    }

    pub fn unmap<const CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        caller: AddressSpaceId,
        device_authority: CapabilityHandle,
        buffer_authority: CapabilityHandle,
        id: u64,
        iommu: &mut impl Iommu,
    ) -> Result<DmaMapping, DmaError> {
        let slot = self
            .mappings
            .iter()
            .position(|record| record.is_some_and(|record| record.mapping.id == id))
            .ok_or(DmaError::MappingNotFound)?;
        let record = self.mappings[slot].ok_or(DmaError::MappingNotFound)?;
        if record.owner != caller
            || record.device_authority != device_authority
            || record.buffer_authority != buffer_authority
        {
            return Err(DmaError::AccessDenied)
        }
        let required = record
            .mapping
            .permissions
            .required_rights()
            .ok_or(DmaError::InvalidPermissions)?;
        capabilities
            .authorize(
                caller,
                device_authority,
                CapabilityObject::DmaDevice(record.mapping.device),
                required,
            )
            .map_err(|_| DmaError::InvalidCapability)?;
        capabilities
            .inspect(caller, buffer_authority)
            .map_err(|_| DmaError::InvalidCapability)?;
        if !iommu.unmap(
            record.mapping.device,
            record.mapping.iova,
            record.mapping.physical.length,
        ) {
            return Err(DmaError::IommuRejected)
        }
        self.mappings[slot] = None;
        Ok(record.mapping)
    }

    pub fn mapping(&self, id: u64) -> Option<DmaMapping> {
        self.mappings
            .iter()
            .flatten()
            .find(|record| record.mapping.id == id)
            .map(|record| record.mapping)
    }
}

impl<const CAPACITY: usize> Default for DmaManager<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_range(offset: u64, length: u64) -> Result<(), DmaError> {
    if offset % DMA_PAGE_SIZE != 0
        || length == 0
        || length % DMA_PAGE_SIZE != 0
        || offset.checked_add(length).is_none()
    {
        Err(DmaError::InvalidRange)
    } else {
        Ok(())
    }
}

fn align_up(value: u64) -> Result<u64, DmaError> {
    value
        .checked_add(DMA_PAGE_SIZE - 1)
        .map(|value| value & !(DMA_PAGE_SIZE - 1))
        .ok_or(DmaError::IovaExhausted)
}
