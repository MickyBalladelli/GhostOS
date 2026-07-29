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
        if self.memory_region_count == MAX_MEMORY_REGIONS {
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
    }
}

#[cfg(test)]
mod tests;
