#![no_std]
#![forbid(unsafe_code)]

pub const BOOT_INFO_MAGIC: u64 = 0x5359_4e4f_5342_4f4f;
pub const BOOT_INFO_VERSION: u32 = 1;
pub const MAX_MEMORY_REGIONS: usize = 128;
pub const FRAMEBUFFER_PIXEL_RGB: u32 = 1;
pub const FRAMEBUFFER_PIXEL_BGR: u32 = 2;

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootMethod {
    Bios = 1,
    Uefi = 2,
}

impl BootMethod {
    pub const fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(Self::Bios),
            2 => Some(Self::Uefi),
            _ => None,
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryKind {
    Usable = 1,
    Reserved = 2,
    AcpiReclaimable = 3,
    AcpiNonVolatile = 4,
    Bootloader = 5,
    Kernel = 6,
    Framebuffer = 7,
}

impl MemoryKind {
    pub const fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(Self::Usable),
            2 => Some(Self::Reserved),
            3 => Some(Self::AcpiReclaimable),
            4 => Some(Self::AcpiNonVolatile),
            5 => Some(Self::Bootloader),
            6 => Some(Self::Kernel),
            7 => Some(Self::Framebuffer),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MemoryRegion {
    pub start: u64,
    pub length: u64,
    pub kind: MemoryKind,
    pub attributes: u32,
}

impl MemoryRegion {
    pub const EMPTY: Self = Self {
        start: 0,
        length: 0,
        kind: MemoryKind::Reserved,
        attributes: 0,
    };

    pub const fn end(self) -> u64 {
        self.start.saturating_add(self.length)
    }

    pub fn is_valid(self) -> bool {
        self.length != 0
            && (self.start != 0 || self.kind == MemoryKind::Reserved)
            && self.start.checked_add(self.length).is_some()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FramebufferInfo {
    pub address: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: u32,
}

impl FramebufferInfo {
    pub const EMPTY: Self = Self {
        address: 0,
        size: 0,
        width: 0,
        height: 0,
        stride: 0,
        pixel_format: 0,
    };

    pub fn is_valid(self) -> bool {
        if self.address == 0
            && self.size == 0
            && self.width == 0
            && self.height == 0
            && self.stride == 0
            && self.pixel_format == 0
        {
            return true
        }
        if self.address == 0
            || self.size == 0
            || self.width == 0
            || self.height == 0
            || self.stride < self.width
            || !matches!(self.pixel_format, FRAMEBUFFER_PIXEL_RGB | FRAMEBUFFER_PIXEL_BGR)
        {
            return false
        }
        match (self.stride as u64).checked_mul(self.height as u64) {
            Some(pixels) => match pixels.checked_mul(4) {
                Some(bytes) => bytes <= self.size,
                None => false,
            },
            None => false,
        }
    }
}

#[repr(C, align(16))]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    pub method: BootMethod,
    pub physical_address_offset: u64,
    pub rsdp_address: u64,
    pub framebuffer: FramebufferInfo,
    pub memory_region_count: usize,
    pub memory_regions: [MemoryRegion; MAX_MEMORY_REGIONS],
}

impl BootInfo {
    pub const fn empty(method: BootMethod) -> Self {
        Self {
            magic: BOOT_INFO_MAGIC,
            version: BOOT_INFO_VERSION,
            method,
            physical_address_offset: 0,
            rsdp_address: 0,
            framebuffer: FramebufferInfo::EMPTY,
            memory_region_count: 0,
            memory_regions: [MemoryRegion::EMPTY; MAX_MEMORY_REGIONS],
        }
    }

    pub fn push_region(&mut self, region: MemoryRegion) -> bool {
        if self.memory_region_count == MAX_MEMORY_REGIONS
            || !region.is_valid()
            || self
                .regions()
                .last()
                .is_some_and(|previous| region.start < previous.end())
        {
            return false
        }

        self.memory_regions[self.memory_region_count] = region;
        self.memory_region_count += 1;
        true
    }

    pub fn regions(&self) -> &[MemoryRegion] {
        &self.memory_regions[..self.memory_region_count]
    }

    pub fn is_valid(&self) -> bool {
        self.magic == BOOT_INFO_MAGIC
            && self.version == BOOT_INFO_VERSION
            && self.memory_region_count <= MAX_MEMORY_REGIONS
            && BootMethod::from_raw(self.method as u32).is_some()
            && self.framebuffer.is_valid()
            && self.regions().iter().all(|region| region.is_valid())
            && self
                .regions()
                .windows(2)
                .all(|regions| regions[1].start >= regions[0].end())
    }
}

#[cfg(test)]
mod tests;
