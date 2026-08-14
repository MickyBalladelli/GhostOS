use crate::cpu::{Cpu, CpuError, CpuMode, PrivilegeLevel};
use crate::devices::{GopPixelFormat, VESA_FB_SIZE, VESA_LFB_BASE};
use crate::memory::{Mmu, PageFlags, PAGE_SIZE};
use std::path::Path;
use synos_boot_protocol::{
    BootInfo, BootMethod, FramebufferInfo, MemoryKind, MemoryRegion, BOOT_INFO_MAGIC,
    BOOT_INFO_VERSION, FRAMEBUFFER_PIXEL_BGR, FRAMEBUFFER_PIXEL_RGB, MAX_MEMORY_REGIONS,
};

pub const KERNEL_LOAD_ADDR: u64 = 0x0010_0000;
pub const BOOT_INFO_ADDR: u64 = 0x0000_7000;
pub const CMDLINE_ADDR: u64 = 0x0000_8000;
pub const MULTIBOOT_INFO_ADDR: u64 = 0x0000_9000;
pub const MULTIBOOT_MODULES_ADDR: u64 = MULTIBOOT_INFO_ADDR + 0x80;
pub const KERNEL_STACK_TOP: u64 = 0x0200_0000;
pub const KERNEL_STACK_SIZE: u64 = 2 * 1024 * 1024;
pub const INITRD_ALIGNMENT: u64 = 0x1000;
pub const MULTIBOOT_HEADER_MAGIC: u32 = 0x1BADB002;
pub const MULTIBOOT_BOOTLOADER_MAGIC: u32 = 0x2BADB002;

const BOOT_INFO_SIZE: usize = 72 + MAX_MEMORY_REGIONS * 24;
const MULTIBOOT_INFO_SIZE: usize = 116;
const MULTIBOOT_MODULE_SIZE: usize = 16;
const MULTIBOOT_MMAP_ADDR: u64 = MULTIBOOT_INFO_ADDR + 0x200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KernelFormat {
    Raw,
    Elf64,
}

pub struct Loader {
    pub kernel: Option<Vec<u8>>,
    pub initrd: Option<Vec<u8>>,
    pub entry_point: u64,
    pub kernel_size: usize,
    pub initrd_size: usize,
    pub initrd_address: u64,
    pub boot_info_address: u64,
    pub cmdline_address: u64,
    pub multiboot_info_address: u64,
    cmdline: String,
    kernel_format: KernelFormat,
}

impl Loader {
    pub fn new() -> Self {
        Self {
            kernel: None,
            initrd: None,
            entry_point: 0,
            kernel_size: 0,
            initrd_size: 0,
            initrd_address: 0,
            boot_info_address: BOOT_INFO_ADDR,
            cmdline_address: CMDLINE_ADDR,
            multiboot_info_address: MULTIBOOT_INFO_ADDR,
            cmdline: String::new(),
            kernel_format: KernelFormat::Raw,
        }
    }

    pub fn load_kernel<P: AsRef<Path>>(&mut self, path: P) -> Result<(), LoaderError> {
        let bytes = std::fs::read(path).map_err(|_| LoaderError::FileNotFound)?;
        self.load_kernel_bytes(bytes)
    }

    pub fn load_kernel_bytes(&mut self, bytes: Vec<u8>) -> Result<(), LoaderError> {
        self.kernel_format = if is_elf64(&bytes) {
            KernelFormat::Elf64
        } else {
            KernelFormat::Raw
        };
        self.entry_point = match self.kernel_format {
            KernelFormat::Raw => KERNEL_LOAD_ADDR,
            KernelFormat::Elf64 => {
                let entry = elf_entry(&bytes).ok_or(LoaderError::InvalidFormat)?;
                let (start, end) = elf_load_range(&bytes)?;
                if entry < start || entry >= end {
                    return Err(LoaderError::InvalidFormat)
                }
                entry
            }
        };
        self.kernel_size = bytes.len();
        self.kernel = Some(bytes);
        Ok(())
    }

    pub fn load_initrd<P: AsRef<Path>>(&mut self, path: P) -> Result<(), LoaderError> {
        let bytes = std::fs::read(path).map_err(|_| LoaderError::FileNotFound)?;
        self.load_initrd_bytes(bytes)
    }

    pub fn load_initrd_bytes(&mut self, bytes: Vec<u8>) -> Result<(), LoaderError> {
        self.initrd_size = bytes.len();
        self.initrd = Some(bytes);
        Ok(())
    }

    pub fn set_cmdline<S: Into<String>>(&mut self, cmdline: S) {
        self.cmdline = cmdline.into();
    }

    pub fn cmdline(&self) -> &str {
        &self.cmdline
    }

    pub fn load_to_memory(&mut self, mmu: &mut Mmu, load_addr: u64) -> Result<(), LoaderError> {
        let kernel = self.kernel.as_deref().ok_or(LoaderError::KernelMissing)?;
        let (kernel_start, kernel_end) = match self.kernel_format {
            KernelFormat::Raw => (
                load_addr,
                load_addr
                    .checked_add(kernel.len() as u64)
                    .ok_or(LoaderError::InvalidFormat)?,
            ),
            KernelFormat::Elf64 => elf_load_range(kernel)?,
        };

        let initrd_address = if let Some(initrd) = self.initrd.as_deref() {
            let address = align_up(kernel_end, INITRD_ALIGNMENT).ok_or(LoaderError::InvalidFormat)?;
            let end = address
                .checked_add(initrd.len() as u64)
                .ok_or(LoaderError::InvalidFormat)?;
            self.validate_layout(kernel_start, kernel_end, address, end, FramebufferInfo::EMPTY)?;
            address
        } else {
            self.validate_layout(kernel_start, kernel_end, 0, 0, FramebufferInfo::EMPTY)?;
            0
        };

        match self.kernel_format {
            KernelFormat::Raw => {
                mmu.write_phys(load_addr, kernel)
                    .map_err(|_| LoaderError::LoadFailed)?;
                self.entry_point = load_addr;
            }
            KernelFormat::Elf64 => {
                load_elf64(mmu, kernel, &mut self.entry_point)?;
            }
        }

        self.initrd_address = initrd_address;
        if let Some(initrd) = self.initrd.as_deref() {
            mmu.write_phys(self.initrd_address, initrd)
                .map_err(|_| LoaderError::LoadFailed)?;
        }

        let cmdline = self.cmdline.as_bytes();
        if !cmdline.is_empty() {
            let mut bytes = Vec::with_capacity(cmdline.len() + 1);
            bytes.extend_from_slice(cmdline);
            bytes.push(0);
            mmu.write_phys(self.cmdline_address, &bytes)
                .map_err(|_| LoaderError::LoadFailed)?;
        }
        Ok(())
    }

    pub fn install_boot_parameters(
        &self,
        mmu: &mut Mmu,
        memory_size: usize,
        method: BootMethod,
        framebuffer: FramebufferInfo,
    ) -> Result<(), LoaderError> {
        let kernel_start = if self.kernel.is_some() {
            match self.kernel_format {
                KernelFormat::Raw => KERNEL_LOAD_ADDR,
                KernelFormat::Elf64 => {
                    elf_load_start(self.kernel.as_deref().ok_or(LoaderError::KernelMissing)?)
                        .ok_or(LoaderError::InvalidFormat)?
                }
            }
        } else {
            0
        };
        let kernel_end = match self.kernel_format {
            KernelFormat::Raw => kernel_start
                .checked_add(self.kernel_size as u64)
                .ok_or(LoaderError::InvalidFormat)?,
            KernelFormat::Elf64 => {
                elf_load_range(self.kernel.as_deref().ok_or(LoaderError::KernelMissing)?)?.1
            }
        };
        let initrd_end = self
            .initrd_address
            .checked_add(self.initrd_size as u64)
            .ok_or(LoaderError::InvalidFormat)?;
        if KERNEL_STACK_TOP > memory_size as u64 {
            return Err(LoaderError::OutOfMemory)
        }

        self.validate_layout(
            kernel_start,
            kernel_end,
            self.initrd_address,
            initrd_end,
            framebuffer,
        )?;

        let mut info = BootInfo::empty(method);
        info.framebuffer = framebuffer;
        for region in memory_regions(
            memory_size as u64,
            kernel_start,
            kernel_end,
            self.initrd_address,
            initrd_end,
            framebuffer.address,
            framebuffer.size,
            self.boot_info_address,
            BOOT_INFO_SIZE as u64,
            self.cmdline_address,
            self.cmdline.len() as u64 + 1,
            self.multiboot_info_address,
            MULTIBOOT_INFO_SIZE as u64,
        ) {
            if !info.push_region(region) {
                return Err(LoaderError::LoadFailed);
            }
        }

        mmu.write_phys(self.boot_info_address, &boot_info_bytes(&info))
            .map_err(|_| LoaderError::LoadFailed)?;
        self.write_multiboot_info(mmu, memory_size as u64)?;
        Ok(())
    }

    fn validate_layout(
        &self,
        kernel_start: u64,
        kernel_end: u64,
        initrd_start: u64,
        initrd_end: u64,
        framebuffer: FramebufferInfo,
    ) -> Result<(), LoaderError> {
        let cmdline_end = self
            .cmdline_address
            .checked_add(self.cmdline.len() as u64 + 1)
            .ok_or(LoaderError::InvalidFormat)?;
        let ranges = [
            ("kernel", kernel_start, kernel_end),
            ("initrd", initrd_start, initrd_end),
            (
                "boot-info",
                self.boot_info_address,
                self.boot_info_address + BOOT_INFO_SIZE as u64,
            ),
            (
                "cmdline",
                self.cmdline_address,
                cmdline_end,
            ),
            (
                "multiboot-info",
                self.multiboot_info_address,
                self.multiboot_info_address + MULTIBOOT_INFO_SIZE as u64,
            ),
            (
                "multiboot-map",
                MULTIBOOT_MMAP_ADDR,
                MULTIBOOT_MMAP_ADDR + 0x1000,
            ),
            (
                "framebuffer",
                framebuffer.address,
                framebuffer.address.saturating_add(framebuffer.size),
            ),
        ];
        for (index, (_, start, end)) in ranges.iter().enumerate() {
            if *start >= *end {
                continue
            }
            for (_, other_start, other_end) in ranges.iter().skip(index + 1) {
                if *other_start < *other_end && *start < *other_end && *other_start < *end {
                    return Err(LoaderError::MemoryOverlap)
                }
            }
        }
        if ranges[..2]
            .iter()
            .any(|(_, start, end)| *start <= KERNEL_STACK_TOP && KERNEL_STACK_TOP < *end)
        {
            return Err(LoaderError::MemoryOverlap)
        }
        Ok(())
    }

    pub fn entry_point(&self) -> u64 {
        self.entry_point
    }

    pub fn kernel_size(&self) -> usize {
        self.kernel_size
    }

    pub fn initrd_size(&self) -> usize {
        self.initrd_size
    }

    pub fn handoff(&self, cpu: &mut Cpu, mmu: &mut Mmu) -> Result<(), LoaderError> {
        let flags = PageFlags::PRESENT | PageFlags::WRITABLE;
        let mut addr = 0u64;
        while addr < mmu.ram_size() as u64 {
            mmu.map_page(addr, addr, flags)
                .map_err(|_| LoaderError::OutOfMemory)?;
            addr += PAGE_SIZE as u64;
        }
        let mut framebuffer = VESA_LFB_BASE;
        while framebuffer < VESA_LFB_BASE + VESA_FB_SIZE as u64 {
            mmu.map_page(framebuffer, framebuffer, flags)
                .map_err(|_| LoaderError::OutOfMemory)?;
            framebuffer += PAGE_SIZE as u64;
        }

        if cpu.mode() != CpuMode::Long64 {
            cpu.state.cr4 |= 1 << 5;
            cpu.state.efer |= 1 << 8;
            cpu.enter_protected_mode(mmu, 0)
                .map_err(LoaderError::CpuError)?;
            cpu.enter_long_mode(mmu, mmu.cr3())
                .map_err(LoaderError::CpuError)?;
        } else {
            mmu.set_paging(true, cpu.state.cr3);
        }
        cpu.state.rip = self.entry_point;
        cpu.state.rsp = KERNEL_STACK_TOP;
        cpu.state.rdi = self.boot_info_address;
        cpu.state.rsi = self.multiboot_info_address;
        cpu.state.rax = MULTIBOOT_BOOTLOADER_MAGIC as u64;
        cpu.state.rbx = self.multiboot_info_address;
        // Firmware enters the kernel with maskable interrupts disabled. The
        // kernel installs its IDT before enabling them for user processes.
        cpu.state.rflags = 0x2;
        cpu.state.halted = false;
        cpu.state.mode = CpuMode::Long64;
        cpu.state.privilege = PrivilegeLevel::Ring0;
        mmu.set_privilege(false);
        Ok(())
    }

    fn write_multiboot_info(&self, mmu: &mut Mmu, memory_size: u64) -> Result<(), LoaderError> {
        let mut info = vec![0u8; MULTIBOOT_INFO_SIZE];
        let mut flags = 1u32 | 2u32 | 4u32 | 64u32;
        put_u32(&mut info, 0, flags);
        put_u32(&mut info, 4, 640);
        put_u32(
            &mut info,
            8,
            memory_size.saturating_div(1024).saturating_sub(1024) as u32,
        );

        if self.cmdline.is_empty() {
            flags &= !4;
            put_u32(&mut info, 0, flags);
        } else {
            put_u32(&mut info, 16, self.cmdline_address as u32);
        }

        if self.initrd_size == 0 {
            flags &= !8;
            put_u32(&mut info, 0, flags);
        } else {
            put_u32(&mut info, 20, 1);
            put_u32(&mut info, 24, MULTIBOOT_MODULES_ADDR as u32);
            let mut module = [0u8; MULTIBOOT_MODULE_SIZE];
            module[0..4].copy_from_slice(&(self.initrd_address as u32).to_le_bytes());
            module[4..8].copy_from_slice(
                &((self.initrd_address + self.initrd_size as u64) as u32).to_le_bytes(),
            );
            mmu.write_phys(MULTIBOOT_MODULES_ADDR, &module)
                .map_err(|_| LoaderError::LoadFailed)?;
        }

        put_u32(&mut info, 48, multiboot_mmap(memory_size).len() as u32);
        put_u32(&mut info, 52, MULTIBOOT_MMAP_ADDR as u32);
        mmu.write_phys(self.multiboot_info_address, &info)
            .map_err(|_| LoaderError::LoadFailed)?;
        mmu.write_phys(MULTIBOOT_MMAP_ADDR, &multiboot_mmap(memory_size))
            .map_err(|_| LoaderError::LoadFailed)
    }
}

impl Default for Loader {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum LoaderError {
    FileNotFound,
    KernelMissing,
    InvalidFormat,
    LoadFailed,
    OutOfMemory,
    MemoryOverlap,
    NotSupported,
    CpuError(CpuError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultibootInfo {
    pub flags: u32,
    pub mem_lower: u32,
    pub mem_upper: u32,
    pub boot_device: u32,
    pub cmdline: u32,
    pub mods_count: u32,
    pub mods_addr: u32,
}

pub fn multiboot_header(kernel: &[u8]) -> Option<u32> {
    let limit = kernel.len().min(8192);
    (0..limit.saturating_sub(11)).step_by(4).find_map(|i| {
        let magic = u32::from_le_bytes(kernel[i..i + 4].try_into().ok()?);
        let flags = u32::from_le_bytes(kernel[i + 4..i + 8].try_into().ok()?);
        let checksum = u32::from_le_bytes(kernel[i + 8..i + 12].try_into().ok()?);
        (magic == MULTIBOOT_HEADER_MAGIC && magic.wrapping_add(flags).wrapping_add(checksum) == 0)
            .then_some(i as u32)
    })
}

pub fn parse_multiboot(header: &[u8]) -> Option<MultibootInfo> {
    if header.len() < 28 {
        return None;
    }
    Some(MultibootInfo {
        flags: read_u32(header, 0)?,
        mem_lower: read_u32(header, 4)?,
        mem_upper: read_u32(header, 8)?,
        boot_device: read_u32(header, 12)?,
        cmdline: read_u32(header, 16)?,
        mods_count: read_u32(header, 20)?,
        mods_addr: read_u32(header, 24)?,
    })
}

fn is_elf64(bytes: &[u8]) -> bool {
    bytes.len() >= 5 && &bytes[..4] == b"\x7FELF" && bytes[4] == 2
}

fn elf_entry(bytes: &[u8]) -> Option<u64> {
    if !is_elf64(bytes) || bytes.len() < 0x40 {
        return None;
    }
    Some(u64::from_le_bytes(bytes[0x18..0x20].try_into().ok()?))
}

fn elf_load_start(bytes: &[u8]) -> Option<u64> {
    elf_segments(bytes)
        .ok()?
        .into_iter()
        .map(|segment| segment.address)
        .min()
}

fn elf_load_range(bytes: &[u8]) -> Result<(u64, u64), LoaderError> {
    let segments = elf_segments(bytes)?;
    let start = segments
        .iter()
        .map(|segment| segment.address)
        .min()
        .ok_or(LoaderError::InvalidFormat)?;
    let end = segments
        .iter()
        .map(|segment| segment.address.saturating_add(segment.memory_size))
        .max()
        .ok_or(LoaderError::InvalidFormat)?;
    Ok((start, end))
}

struct ElfSegment<'a> {
    address: u64,
    data: &'a [u8],
    memory_size: u64,
}

fn elf_segments(bytes: &[u8]) -> Result<Vec<ElfSegment<'_>>, LoaderError> {
    if !is_elf64(bytes) || bytes.len() < 0x40 {
        return Err(LoaderError::InvalidFormat);
    }
    let phoff = usize::try_from(
        u64::from_le_bytes(
            bytes[0x20..0x28]
                .try_into()
                .map_err(|_| LoaderError::InvalidFormat)?,
        ),
    )
    .map_err(|_| LoaderError::InvalidFormat)?;
    let phentsize = u16::from_le_bytes(
        bytes[0x36..0x38]
            .try_into()
            .map_err(|_| LoaderError::InvalidFormat)?,
    ) as usize;
    let phnum = u16::from_le_bytes(
        bytes[0x38..0x3A]
            .try_into()
            .map_err(|_| LoaderError::InvalidFormat)?,
    ) as usize;
    if phentsize < 56
        || phentsize
            .checked_mul(phnum)
            .and_then(|size| phoff.checked_add(size))
            .filter(|end| *end <= bytes.len())
            .is_none()
    {
        return Err(LoaderError::InvalidFormat);
    }

    let mut segments = Vec::new();
    for index in 0..phnum {
        let ph = phoff
            .checked_add(
                index
                    .checked_mul(phentsize)
                    .ok_or(LoaderError::InvalidFormat)?,
            )
            .ok_or(LoaderError::InvalidFormat)?;
        let kind = u32::from_le_bytes(
            bytes[ph..ph + 4]
                .try_into()
                .map_err(|_| LoaderError::InvalidFormat)?,
        );
        if kind != 1 {
            continue;
        }
        let file_offset = u64::from_le_bytes(
            bytes[ph + 8..ph + 16]
                .try_into()
                .map_err(|_| LoaderError::InvalidFormat)?,
        );
        let virtual_address = u64::from_le_bytes(
            bytes[ph + 16..ph + 24]
                .try_into()
                .map_err(|_| LoaderError::InvalidFormat)?,
        );
        let physical_address = u64::from_le_bytes(
            bytes[ph + 24..ph + 32]
                .try_into()
                .map_err(|_| LoaderError::InvalidFormat)?,
        );
        let file_size = u64::from_le_bytes(
            bytes[ph + 32..ph + 40]
                .try_into()
                .map_err(|_| LoaderError::InvalidFormat)?,
        );
        let memory_size = u64::from_le_bytes(
            bytes[ph + 40..ph + 48]
                .try_into()
                .map_err(|_| LoaderError::InvalidFormat)?,
        );
        let address = if physical_address != 0 {
            physical_address
        } else {
            virtual_address
        };
        let start = usize::try_from(file_offset).map_err(|_| LoaderError::InvalidFormat)?;
        let end = file_offset
            .checked_add(file_size)
            .and_then(|end| usize::try_from(end).ok())
            .filter(|end| *end <= bytes.len())
            .ok_or(LoaderError::InvalidFormat)?;
        if memory_size < file_size {
            return Err(LoaderError::InvalidFormat);
        }
        segments.push(ElfSegment {
            address,
            data: &bytes[start..end],
            memory_size,
        });
    }
    if segments.is_empty() {
        return Err(LoaderError::InvalidFormat);
    }
    Ok(segments)
}

fn load_elf64(mmu: &mut Mmu, bytes: &[u8], entry: &mut u64) -> Result<u64, LoaderError> {
    let segments = elf_segments(bytes)?;
    let mut end = 0;
    for segment in segments {
        let segment_end = segment
            .address
            .checked_add(segment.memory_size)
            .filter(|end| *end <= mmu.ram_size() as u64)
            .ok_or(LoaderError::OutOfMemory)?;
        mmu.write_phys(segment.address, segment.data)
            .map_err(|_| LoaderError::LoadFailed)?;
        let zeroes = segment.memory_size - segment.data.len() as u64;
        if zeroes > 0 {
            let zeroes =
                vec![0u8; usize::try_from(zeroes).map_err(|_| LoaderError::InvalidFormat)?];
            let zero_start = segment
                .address
                .checked_add(segment.data.len() as u64)
                .ok_or(LoaderError::OutOfMemory)?;
            mmu.write_phys(zero_start, &zeroes)
                .map_err(|_| LoaderError::LoadFailed)?;
        }
        end = end.max(segment_end);
    }
    if *entry == 0 {
        *entry = KERNEL_LOAD_ADDR;
    }
    Ok(end)
}

fn align_up(value: u64, alignment: u64) -> Option<u64> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

fn memory_regions(
    memory_size: u64,
    kernel_start: u64,
    kernel_end: u64,
    initrd_start: u64,
    initrd_end: u64,
    framebuffer_start: u64,
    framebuffer_size: u64,
    boot_info_start: u64,
    boot_info_size: u64,
    cmdline_start: u64,
    cmdline_size: u64,
    multiboot_start: u64,
    multiboot_size: u64,
) -> Vec<MemoryRegion> {
    let mut reserved = vec![
        (0, 0x100000, MemoryKind::Reserved),
        (kernel_start, kernel_end, MemoryKind::Kernel),
        (initrd_start, initrd_end, MemoryKind::Bootloader),
        (
            framebuffer_start,
            framebuffer_start.saturating_add(framebuffer_size),
            MemoryKind::Framebuffer,
        ),
        (
            boot_info_start,
            boot_info_start.saturating_add(boot_info_size),
            MemoryKind::Bootloader,
        ),
        (
            cmdline_start,
            cmdline_start.saturating_add(cmdline_size),
            MemoryKind::Bootloader,
        ),
        (
            multiboot_start,
            multiboot_start.saturating_add(multiboot_size),
            MemoryKind::Bootloader,
        ),
        (
            KERNEL_STACK_TOP.saturating_sub(KERNEL_STACK_SIZE),
            KERNEL_STACK_TOP,
            MemoryKind::Reserved,
        ),
        (
            MULTIBOOT_MMAP_ADDR,
            MULTIBOOT_MMAP_ADDR + 0x1000,
            MemoryKind::Bootloader,
        ),
    ];
    reserved.retain(|(start, end, _)| *start < *end && *start < memory_size);
    reserved.sort_by_key(|(start, _, _)| *start);

    let mut regions = Vec::new();
    let mut cursor = 0;
    for (start, end, kind) in reserved {
        let start = start.min(memory_size);
        let end = end.min(memory_size);
        if cursor < start {
            regions.push(MemoryRegion {
                start: cursor,
                length: start - cursor,
                kind: MemoryKind::Usable,
                attributes: 0,
            });
        }
        if cursor < end {
            regions.push(MemoryRegion {
                start: cursor.max(start),
                length: end - cursor.max(start),
                kind,
                attributes: 0,
            });
            cursor = end;
        }
    }
    if cursor < memory_size {
        regions.push(MemoryRegion {
            start: cursor,
            length: memory_size - cursor,
            kind: MemoryKind::Usable,
            attributes: 0,
        });
    }
    if framebuffer_start >= memory_size && framebuffer_size != 0 {
        regions.push(MemoryRegion {
            start: framebuffer_start,
            length: framebuffer_size,
            kind: MemoryKind::Framebuffer,
            attributes: 0,
        });
    }
    regions
}

fn boot_info_bytes(info: &BootInfo) -> Vec<u8> {
    let mut bytes = vec![0u8; BOOT_INFO_SIZE];
    bytes[0..8].copy_from_slice(&info.magic.to_le_bytes());
    bytes[8..12].copy_from_slice(&info.version.to_le_bytes());
    bytes[12..16].copy_from_slice(&(info.method as u32).to_le_bytes());
    bytes[16..24].copy_from_slice(&info.physical_address_offset.to_le_bytes());
    bytes[24..32].copy_from_slice(&info.rsdp_address.to_le_bytes());
    bytes[32..40].copy_from_slice(&info.framebuffer.address.to_le_bytes());
    bytes[40..48].copy_from_slice(&info.framebuffer.size.to_le_bytes());
    bytes[48..52].copy_from_slice(&info.framebuffer.width.to_le_bytes());
    bytes[52..56].copy_from_slice(&info.framebuffer.height.to_le_bytes());
    bytes[56..60].copy_from_slice(&info.framebuffer.stride.to_le_bytes());
    bytes[60..64].copy_from_slice(&info.framebuffer.pixel_format.to_le_bytes());
    bytes[64..72].copy_from_slice(&info.memory_region_count.to_le_bytes());
    for (index, region) in info.regions().iter().enumerate() {
        let offset = 72 + index * 24;
        bytes[offset..offset + 8].copy_from_slice(&region.start.to_le_bytes());
        bytes[offset + 8..offset + 16].copy_from_slice(&region.length.to_le_bytes());
        bytes[offset + 16..offset + 20].copy_from_slice(&(region.kind as u32).to_le_bytes());
        bytes[offset + 20..offset + 24].copy_from_slice(&region.attributes.to_le_bytes());
    }
    bytes
}

fn multiboot_mmap(memory_size: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    let entries: [(u64, u64, u32); 3] = [
        (0, memory_size.min(0x9FC00), 1),
        (0x9FC00, memory_size.saturating_sub(0x9FC00).min(0x60400), 2),
        (0x100000, memory_size.saturating_sub(0x100000), 1),
    ];
    for (base, length, kind) in entries {
        if length == 0 {
            continue;
        }
        bytes.extend_from_slice(&20u32.to_le_bytes());
        bytes.extend_from_slice(&base.to_le_bytes());
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
    }
    bytes
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

pub fn framebuffer_info(gop: &crate::devices::UefiGop) -> FramebufferInfo {
    let pixel_format = match gop.modes.first().map(|mode| mode.pixel_format) {
        Some(GopPixelFormat::BgrxRgb8 | GopPixelFormat::BgraRgb8) => FRAMEBUFFER_PIXEL_BGR,
        Some(GopPixelFormat::XbgrRgb8 | GopPixelFormat::XrgbRgb8) => FRAMEBUFFER_PIXEL_RGB,
        None => 0,
    };
    let mode = gop
        .modes
        .get(gop.current_mode)
        .or_else(|| gop.modes.first());
    FramebufferInfo {
        address: gop.framebuffer_base,
        size: gop.framebuffer_size,
        width: mode.map(|mode| mode.width).unwrap_or(0),
        height: mode.map(|mode| mode.height).unwrap_or(0),
        stride: mode.map(|mode| mode.pixels_per_scanline).unwrap_or(0),
        pixel_format,
    }
}

pub fn boot_info_magic() -> u64 {
    BOOT_INFO_MAGIC
}

pub fn boot_info_version() -> u32 {
    BOOT_INFO_VERSION
}
