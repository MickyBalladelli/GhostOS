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
        if self.cxl_ide_available && !self.cxl_ide_enabled {
            return Err(SiloError::CxlIdeRequired);
        }
        if self.confidential_cpu != ConfidentialCpu::None && !self.memory_encryption_enabled {
            return Err(SiloError::MemoryEncryptionRequired);
        }
        Ok(if self.cxl_ide_enabled && self.memory_encryption_enabled {
            MemoryProtection::HardwareEncrypted
        } else {
            MemoryProtection::KernelIsolated
        })
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

    const fn overlaps(self, other: Self) -> bool {
        self.start < other.start + other.length && other.start < self.start + self.length
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
    ranges: [Option<SiloMemoryRange>; RANGES],
}

impl<const RANGES: usize> BlindMicroSilo<RANGES> {
    pub fn new(
        address_space: AddressSpaceId,
        hardware: HardwareIsolation,
    ) -> Result<Self, SiloError> {
        Ok(Self {
            address_space,
            protection: hardware.protection()?,
            ranges: [None; RANGES],
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
        if self
            .ranges
            .iter()
            .flatten()
            .any(|mapped| mapped.overlaps(range))
        {
            return Err(SiloError::RangeConflict);
        }
        let slot = self
            .ranges
            .iter_mut()
            .find(|mapped| mapped.is_none())
            .ok_or(SiloError::Capacity)?;
        *slot = Some(range);
        Ok(())
    }

    pub fn unmap_borrowed_memory(&mut self) -> usize {
        let mut unmapped = 0;
        for range in &mut self.ranges {
            if range.is_some_and(|range| range.borrowed) {
                *range = None;
                unmapped += 1
            }
        }
        unmapped
    }

    pub fn contains_memory(&self, address: u64) -> bool {
        self.ranges
            .iter()
            .flatten()
            .any(|range| address >= range.start && address < range.start + range.length)
    }
}
