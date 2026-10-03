use crate::task::AddressSpaceId;

pub const MAX_SILO_MEMORY_RANGES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiloObject {
    TenantProcess,
    TenantSynFs,
    TenantSocket,
    HostProcessTree,
    HostSynFsMount,
    HostNetworkSocket,
    FederationControl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiloOperation {
    Inspect,
    Read,
    Write,
    Connect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfidentialCpu {
    None,
    AmdSev,
    IntelTdx,
    ArmCca,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HardwareIsolation {
    pub confidential_cpu: ConfidentialCpu,
    pub cxl_ide_available: bool,
    pub cxl_ide_enabled: bool,
    pub memory_encryption_enabled: bool,
}

impl HardwareIsolation {
    pub const SOFTWARE_ONLY: Self = Self {
        confidential_cpu: ConfidentialCpu::None,
        cxl_ide_available: false,
        cxl_ide_enabled: false,
        memory_encryption_enabled: false,
    };

    pub fn protection(self) -> Result<MemoryProtection, SiloError> {
        let hardware = NativeHardware {
            confidential_cpu: self.confidential_cpu as u32,
            cxl_ide_available: self.cxl_ide_available,
            cxl_ide_enabled: self.cxl_ide_enabled,
            memory_encryption_enabled: self.memory_encryption_enabled,
        };
        let mut protection = 0;
        match unsafe { ghostos_silo_hardware_protection(hardware, &mut protection) } {
            0 => Ok(if protection == 1 {
                MemoryProtection::HardwareEncrypted
            } else {
                MemoryProtection::KernelIsolated
            }),
            3 => Err(SiloError::CxlIdeRequired),
            5 => Err(SiloError::MemoryEncryptionRequired),
            _ => unreachable!("invalid native silo hardware result"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryProtection {
    KernelIsolated,
    HardwareEncrypted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SiloMemoryRange {
    pub start: u64,
    pub length: u64,
    pub borrowed: bool,
}

impl SiloMemoryRange {
    pub const fn new(start: u64, length: u64, borrowed: bool) -> Result<Self, SiloError> {
        if length == 0 || start.checked_add(length).is_none() {
            Err(SiloError::InvalidRange)
        } else {
            Ok(Self {
                start,
                length,
                borrowed,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiloError {
    AccessDenied,
    Capacity,
    CxlIdeRequired,
    InvalidRange,
    MemoryEncryptionRequired,
    RangeConflict,
}

/// Kernel-enforced visibility boundary for a workload borrowed from another cluster.
///
/// There is no switch that grants host visibility. A silo can see only its own
/// process, filesystem namespace, and sockets.
pub struct BlindMicroSilo<const RANGES: usize = MAX_SILO_MEMORY_RANGES> {
    address_space: AddressSpaceId,
    protection: MemoryProtection,
    ranges: [NativeRange; RANGES],
}

impl<const RANGES: usize> BlindMicroSilo<RANGES> {
    pub fn new(
        address_space: AddressSpaceId,
        hardware: HardwareIsolation,
    ) -> Result<Self, SiloError> {
        Ok(Self {
            address_space,
            protection: hardware.protection()?,
            ranges: [NativeRange::EMPTY; RANGES],
        })
    }

    pub const fn address_space(&self) -> AddressSpaceId {
        self.address_space
    }

    pub const fn protection(&self) -> MemoryProtection {
        self.protection
    }

    pub const fn authorize(
        &self,
        object: SiloObject,
        _operation: SiloOperation,
    ) -> Result<(), SiloError> {
        match object {
            SiloObject::TenantProcess | SiloObject::TenantSynFs | SiloObject::TenantSocket => {
                Ok(())
            }
            SiloObject::HostProcessTree
            | SiloObject::HostSynFsMount
            | SiloObject::HostNetworkSocket
            | SiloObject::FederationControl => Err(SiloError::AccessDenied),
        }
    }

    pub fn map_memory(&mut self, range: SiloMemoryRange) -> Result<(), SiloError> {
        let native = NativeRange {
            start: range.start,
            length: range.length,
            borrowed: range.borrowed,
            occupied: true,
        };
        match unsafe {
            ghostos_silo_ranges_map(self.ranges.as_mut_ptr(), RANGES, native, cfg!(debug_assertions))
        } {
            0 => Ok(()),
            1 => Err(SiloError::RangeConflict),
            2 => Err(SiloError::Capacity),
            -1 => panic!("attempt to add with overflow"),
            _ => unreachable!("invalid native silo map result"),
        }
    }

    pub fn unmap_borrowed_memory(&mut self) -> usize {
        unsafe { ghostos_silo_ranges_unmap_borrowed(self.ranges.as_mut_ptr(), RANGES) }
    }

    pub fn contains_memory(&self, address: u64) -> bool {
        match unsafe {
            ghostos_silo_ranges_contains(self.ranges.as_ptr(), RANGES, address, cfg!(debug_assertions))
        } {
            -1 => panic!("attempt to add with overflow"),
            result => result != 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeRange {
    start: u64,
    length: u64,
    borrowed: bool,
    occupied: bool,
}

impl NativeRange {
    const EMPTY: Self = Self { start: 0, length: 0, borrowed: false, occupied: false };
}

#[repr(C)]
struct NativeHardware {
    confidential_cpu: u32,
    cxl_ide_available: bool,
    cxl_ide_enabled: bool,
    memory_encryption_enabled: bool,
}

const _: () = {
    assert!(core::mem::size_of::<NativeRange>() == 24);
    assert!(core::mem::offset_of!(NativeRange, occupied) == 17);
    assert!(core::mem::size_of::<NativeHardware>() == 8);
};

unsafe extern "C" {
    fn ghostos_silo_hardware_protection(hardware: NativeHardware, protection: *mut u32) -> i32;
    fn ghostos_silo_ranges_map(ranges: *mut NativeRange, capacity: usize, range: NativeRange, checked: bool) -> i32;
    fn ghostos_silo_ranges_unmap_borrowed(ranges: *mut NativeRange, capacity: usize) -> usize;
    fn ghostos_silo_ranges_contains(ranges: *const NativeRange, capacity: usize, address: u64, checked: bool) -> i32;
}
