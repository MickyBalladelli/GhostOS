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

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn bits(self) -> u8 {
        self.0
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

pub struct DmaManager<const CAPACITY: usize = MAX_DMA_MAPPINGS> {
    state: NativeState,
    mappings: [NativeRecord; CAPACITY],
}

impl<const CAPACITY: usize> DmaManager<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            state: NativeState { next_id: 1, next_iova: FIRST_IOVA },
            mappings: [NativeRecord::EMPTY; CAPACITY],
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
        let mut required_bits = 0;
        native_result(unsafe {
            ghostos_dma_request(offset, length, permissions.bits(), &mut required_bits)
        })?;
        let required = native_rights(required_bits);
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
        let mut physical = NativeRange::EMPTY;
        native_result(unsafe {
            ghostos_dma_buffer(buffer.rights.bits(), required_bits, buffer.backing.is_some(),
                buffer.backing.map_or(NativeRange::EMPTY, NativeRange::from), offset, length,
                &mut physical)
        })?;
        let mut plan = NativePlan::EMPTY;
        native_result(unsafe {
            ghostos_dma_prepare_map(&self.state, self.mappings.as_ptr(), CAPACITY, caller.raw(),
                device_authority.raw(), buffer_authority.raw(), device.raw(), physical,
                permissions.bits(), &mut plan)
        })?;
        let mapping = plan.record.mapping.to_public();
        if !iommu.map(device, mapping.iova, mapping.physical, permissions) {
            return Err(DmaError::IommuRejected)
        }
        unsafe { ghostos_dma_commit_map(&mut self.state, self.mappings.as_mut_ptr(), &plan) };
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
        let mut plan = NativePlan::EMPTY;
        let mut required_bits = 0;
        native_result(unsafe {
            ghostos_dma_prepare_unmap(self.mappings.as_ptr(), CAPACITY, caller.raw(),
                device_authority.raw(), buffer_authority.raw(), id, &mut plan, &mut required_bits)
        })?;
        let mapping = plan.record.mapping.to_public();
        let required = native_rights(required_bits);
        capabilities
            .authorize(
                caller,
                device_authority,
                CapabilityObject::DmaDevice(mapping.device),
                required,
            )
            .map_err(|_| DmaError::InvalidCapability)?;
        capabilities
            .inspect(caller, buffer_authority)
            .map_err(|_| DmaError::InvalidCapability)?;
        if !iommu.unmap(
            mapping.device,
            mapping.iova,
            mapping.physical.length,
        ) {
            return Err(DmaError::IommuRejected)
        }
        unsafe { ghostos_dma_commit_unmap(self.mappings.as_mut_ptr(), &plan) };
        Ok(mapping)
    }

    pub fn mapping(&self, id: u64) -> Option<DmaMapping> {
        let mut mapping = NativeMapping::EMPTY;
        unsafe { ghostos_dma_records_get(self.mappings.as_ptr(), CAPACITY, id, &mut mapping) }
            .then(|| mapping.to_public())
    }
}

impl<const CAPACITY: usize> Default for DmaManager<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
struct NativeState { next_id: u64, next_iova: u64 }

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeRange { start: u64, length: u64 }

impl NativeRange {
    const EMPTY: Self = Self { start: 0, length: 0 };
}

impl From<PhysicalRange> for NativeRange {
    fn from(range: PhysicalRange) -> Self {
        Self { start: range.start, length: range.length }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeMapping {
    id: u64,
    device: u32,
    iova: u64,
    physical: NativeRange,
    permissions: u8,
}

impl NativeMapping {
    const EMPTY: Self = Self { id: 0, device: 0, iova: 0, physical: NativeRange::EMPTY, permissions: 0 };

    fn to_public(self) -> DmaMapping {
        DmaMapping {
            id: self.id,
            device: DmaDeviceId::new(self.device).expect("native DMA device"),
            iova: self.iova,
            physical: PhysicalRange { start: self.physical.start, length: self.physical.length },
            permissions: DmaPermissions(self.permissions),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeRecord {
    mapping: NativeMapping,
    owner: u32,
    device_authority: u64,
    buffer_authority: u64,
    occupied: bool,
}

impl NativeRecord {
    const EMPTY: Self = Self {
        mapping: NativeMapping::EMPTY, owner: 0, device_authority: 0,
        buffer_authority: 0, occupied: false,
    };
}

#[repr(C)]
struct NativePlan { record: NativeRecord, slot: usize, next_iova: u64 }

impl NativePlan {
    const EMPTY: Self = Self { record: NativeRecord::EMPTY, slot: 0, next_iova: 0 };
}

fn native_rights(bits: u16) -> Rights {
    Rights::from_bits(bits).expect("native DMA rights")
}

fn native_result(result: i32) -> Result<(), DmaError> {
    Err(match result {
        0 => return Ok(()),
        1 => DmaError::InvalidRange,
        2 => DmaError::InvalidPermissions,
        3 => DmaError::InvalidCapability,
        4 => DmaError::AccessDenied,
        5 => DmaError::Capacity,
        6 => DmaError::IovaExhausted,
        7 => DmaError::IommuRejected,
        8 => DmaError::MappingNotFound,
        _ => unreachable!("invalid native DMA result"),
    })
}

const _: () = {
    assert!(DMA_PAGE_SIZE == 4096);
    assert!(core::mem::size_of::<NativeState>() == 16);
    assert!(core::mem::size_of::<NativeMapping>() == 48);
    assert!(core::mem::offset_of!(NativeMapping, physical) == 24);
    assert!(core::mem::size_of::<NativeRecord>() == 80);
    assert!(core::mem::offset_of!(NativeRecord, occupied) == 72);
    assert!(core::mem::size_of::<NativePlan>() == 96);
};

unsafe extern "C" {
    fn ghostos_dma_request(offset: u64, length: u64, permissions: u8, rights: *mut u16) -> i32;
    fn ghostos_dma_buffer(rights: u16, required: u16, has_backing: bool, backing: NativeRange,
        offset: u64, length: u64, physical: *mut NativeRange) -> i32;
    fn ghostos_dma_prepare_map(state: *const NativeState, records: *const NativeRecord,
        capacity: usize, caller: u32, device_authority: u64, buffer_authority: u64,
        device: u32, physical: NativeRange, permissions: u8, plan: *mut NativePlan) -> i32;
    fn ghostos_dma_commit_map(state: *mut NativeState, records: *mut NativeRecord, plan: *const NativePlan);
    fn ghostos_dma_prepare_unmap(records: *const NativeRecord, capacity: usize, caller: u32,
        device_authority: u64, buffer_authority: u64, id: u64, plan: *mut NativePlan,
        rights: *mut u16) -> i32;
    fn ghostos_dma_commit_unmap(records: *mut NativeRecord, plan: *const NativePlan);
    fn ghostos_dma_records_get(records: *const NativeRecord, capacity: usize, id: u64,
        mapping: *mut NativeMapping) -> bool;
}
