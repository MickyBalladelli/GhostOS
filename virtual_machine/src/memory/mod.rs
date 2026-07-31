//! Memory management: frame allocator, four-level paging, MMIO routing, and
//! access validation.

use crate::devices::{Device, DeviceError, MmioRegion};
use std::collections::HashMap;

pub const PAGE_SIZE: usize = 4096;
pub const PAGE_SHIFT: u64 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    InvalidAddress,
    PageFault,
    AccessDenied,
    AlignmentError,
    OutOfMemory,
    MmioError,
}

impl From<DeviceError> for MemoryError {
    fn from(_: DeviceError) -> Self {
        MemoryError::MmioError
    }
}

/// Requested memory access kind; used for permission validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    Write,
    Execute,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PageFlags: u64 {
        const PRESENT = 1 << 0;
        const WRITABLE = 1 << 1;
        const USER = 1 << 2;
        const WRITE_THROUGH = 1 << 3;
        const CACHE_DISABLE = 1 << 4;
        const ACCESSED = 1 << 5;
        const DIRTY = 1 << 6;
        const PAT = 1 << 7;
        const GLOBAL = 1 << 8;
        const NX = 1 << 63;
    }
}

/// Frame-based physical memory allocator.
pub struct FrameAllocator {
    total_frames: usize,
    free: Vec<usize>,
}

impl FrameAllocator {
    pub fn new(size: usize) -> Self {
        let total_frames = size / PAGE_SIZE;
        // Ascending collection means pop() returns the highest free frame
        // first, which avoids ever returning frame 0 for the allocator's own
        // page-table structures (frame 0 is the real-mode IVT region).
        Self {
            total_frames,
            free: (0..total_frames).collect(),
        }
    }

    pub fn total_frames(&self) -> usize {
        self.total_frames
    }

    pub fn free_frames(&self) -> usize {
        self.free.len()
    }

    pub fn alloc(&mut self) -> Option<u64> {
        self.free.pop().map(|f| f as u64 * PAGE_SIZE as u64)
    }

    pub fn alloc_zeroed(&mut self, ram: &mut [u8]) -> Option<u64> {
        let frame = self.alloc()?;
        let start = frame as usize;
        let end = (start + PAGE_SIZE).min(ram.len());
        ram[start..end].fill(0);
        Some(frame)
    }

    pub fn free(&mut self, addr: u64) {
        let frame = (addr / PAGE_SIZE as u64) as usize;
        if frame < self.total_frames && !self.free.contains(&frame) {
            self.free.push(frame);
        }
    }

    pub fn reset(&mut self) {
        self.free = (0..self.total_frames).collect();
    }
}

/// A single page-table entry (PTE) as stored in guest memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pte {
    raw: u64,
}

impl Pte {
    fn present(&self) -> bool {
        self.raw & 1 != 0
    }

    fn writable(&self) -> bool {
        self.raw & 2 != 0
    }

    fn user(&self) -> bool {
        self.raw & 4 != 0
    }

    fn nx(&self) -> bool {
        self.raw & (1 << 63) != 0
    }

    fn large(&self) -> bool {
        self.raw & 0x80 != 0
    }

    fn phys(&self) -> u64 {
        self.raw & 0x000F_FFFF_FFFF_F000
    }
}

/// Resolves `virt` through a four-level page table whose root (PML4) is at
/// physical address `cr3`, reading tables from guest RAM.
fn walk_page_table(mem: &[u8], cr3: u64, virt: u64) -> Option<(u64, PageFlags)> {
    let read_ptr = |addr: u64| -> Option<u64> {
        let addr = addr as usize;
        if addr.saturating_add(8) > mem.len() {
            return None;
        }
        Some(u64::from_le_bytes(
            mem[addr..addr + 8].try_into().unwrap(),
        ))
    };

    let pml4_idx = ((virt >> 39) & 0x1FF) as usize;
    let pdpt_idx = ((virt >> 30) & 0x1FF) as usize;
    let pd_idx = ((virt >> 21) & 0x1FF) as usize;
    let pt_idx = ((virt >> 12) & 0x1FF) as usize;

    let pml4e = read_ptr(cr3 + (pml4_idx as u64) * 8)?;
    let pml4e = Pte { raw: pml4e };
    if !pml4e.present() {
        return None;
    }

    let pdpte = read_ptr(pml4e.phys() + (pdpt_idx as u64) * 8)?;
    let pdpte = Pte { raw: pdpte };
    if !pdpte.present() {
        return None;
    }

    // 1 GiB page
    if pdpte.large() {
        let mask = 0x3FFF_FFFFu64;
        return Some((
            pdpte.phys() + (virt & mask),
            PageFlags::from_bits_truncate(pdpte.raw),
        ));
    }

    let pde = read_ptr(pdpte.phys() + (pd_idx as u64) * 8)?;
    let pde = Pte { raw: pde };
    if !pde.present() {
        return None;
    }

    // 2 MiB page
    if pde.large() {
        let mask = 0x1F_FFFFu64;
        return Some((
            pde.phys() + (virt & mask),
            PageFlags::from_bits_truncate(pde.raw),
        ));
    }

    let pte = read_ptr(pde.phys() + (pt_idx as u64) * 8)?;
    let pte = Pte { raw: pte };
    if !pte.present() {
        return None;
    }

    let offset = virt & (PAGE_SIZE as u64 - 1);
    Some((
        pte.phys() + offset,
        PageFlags::from_bits_truncate(pte.raw),
    ))
}

/// Central MMU: physical RAM, frame allocation, page tables, MMIO routing,
/// and permission checks.
pub struct Mmu {
    ram: Vec<u8>,
    allocator: FrameAllocator,
    mmio: Vec<MmioRegion>,
    // Identity/physical mappings the CPU sets up for bootstrap. Key: physical
    // frame address, value: PageFlags.
    identity: HashMap<u64, PageFlags>,
    paging_enabled: bool,
    cr3: u64,
    privilege: bool, // false = kernel (ring 0), true = user (ring 3)
}

fn validate_flags(
    flags: PageFlags,
    access: AccessKind,
    privilege: bool,
) -> Result<(), MemoryError> {
    if !flags.contains(PageFlags::PRESENT) {
        return Err(MemoryError::PageFault);
    }
    if access == AccessKind::Execute && flags.contains(PageFlags::NX) {
        return Err(MemoryError::AccessDenied);
    }
    if access == AccessKind::Write && !flags.contains(PageFlags::WRITABLE) {
        return Err(MemoryError::AccessDenied);
    }
    if privilege && !flags.contains(PageFlags::USER) {
        return Err(MemoryError::AccessDenied);
    }
    if !privilege && access == AccessKind::Read && !flags.contains(PageFlags::PRESENT) {
        return Err(MemoryError::PageFault);
    }
    Ok(())
}

impl Mmu {
    pub fn new(size: usize) -> Self {
        let mut ram = Vec::with_capacity(size);
        ram.resize(size, 0);
        Self {
            allocator: FrameAllocator::new(size),
            ram,
            mmio: Vec::new(),
            identity: HashMap::new(),
            paging_enabled: false,
            cr3: 0,
            privilege: false,
        }
    }

    pub fn ram_size(&self) -> usize {
        self.ram.len()
    }

    pub fn allocator(&self) -> &FrameAllocator {
        &self.allocator
    }

    pub fn allocator_mut(&mut self) -> &mut FrameAllocator {
        &mut self.allocator
    }

    pub fn set_paging(&mut self, enabled: bool, cr3: u64) {
        self.paging_enabled = enabled;
        self.cr3 = cr3;
    }

    pub fn paging_enabled(&self) -> bool {
        self.paging_enabled
    }

    pub fn cr3(&self) -> u64 {
        self.cr3
    }

    pub fn set_privilege(&mut self, user: bool) {
        self.privilege = user;
    }

    pub fn attach_mmio(&mut self, base: u64, size: u64, device: Box<dyn Device>) {
        self.mmio.push(MmioRegion::new(base, size, device));
    }

    pub fn mmio_regions(&self) -> &[MmioRegion] {
        &self.mmio
    }

    pub fn reset_mmio(&mut self) {
        for region in &mut self.mmio {
            region.reset();
        }
    }

    fn find_mmio(&self, addr: u64, size: u64) -> Option<&MmioRegion> {
        self.mmio.iter().find(|r| r.contains(addr, size))
    }

    fn find_mmio_mut(&mut self, addr: u64, size: u64) -> Option<&mut MmioRegion> {
        self.mmio.iter_mut().find(|r| r.contains(addr, size))
    }

    fn physical_address(
        &self,
        virt: u64,
        access: AccessKind,
        size: u64,
    ) -> Result<u64, MemoryError> {
        if self.paging_enabled {
            let max_addr = u64::MAX - size + 1;
            if virt > max_addr {
                return Err(MemoryError::PageFault);
            }
            let (phys, flags) =
                walk_page_table(&self.ram, self.cr3, virt).ok_or(MemoryError::PageFault)?;
            validate_flags(flags, access, self.privilege)?;
            Ok(phys)
        } else {
            // Identity mapping.
            if let Some(&flags) = self.identity.get(&(virt & !(PAGE_SIZE as u64 - 1))) {
                validate_flags(flags, access, self.privilege)?;
            }
            Ok(virt)
        }
    }

    fn ram_read(&self, phys: u64, size: usize) -> Result<u64, MemoryError> {
        let start = phys as usize;
        let end = start.checked_add(size).ok_or(MemoryError::InvalidAddress)?;
        if end > self.ram.len() {
            return Err(MemoryError::InvalidAddress);
        }
        let mut bytes = [0u8; 8];
        bytes[..size].copy_from_slice(&self.ram[start..end]);
        Ok(u64::from_le_bytes(bytes))
    }

    fn ram_write(&mut self, phys: u64, value: u64, size: usize) -> Result<(), MemoryError> {
        let start = phys as usize;
        let end = start.checked_add(size).ok_or(MemoryError::InvalidAddress)?;
        if end > self.ram.len() {
            return Err(MemoryError::InvalidAddress);
        }
        self.ram[start..end].copy_from_slice(&value.to_le_bytes()[..size]);
        Ok(())
    }

    /// Read `size` bytes from a (possibly translated) address. If the address
    /// falls inside a registered MMIO region, the whole access is routed to
    /// the device in one call; otherwise it reads from RAM. This is the
    /// canonical read used by the CPU executor's memory operands.
    pub fn read_from_addr(&self, addr: u64, size: u8) -> Result<u64, MemoryError> {
        let phys = self.physical_address(addr, AccessKind::Read, size as u64)?;
        if let Some(region) = self.find_mmio(phys, size as u64) {
            return region
                .read(addr, size)
                .map_err(|_| MemoryError::MmioError);
        }
        self.ram_read(phys, size as usize)
    }

    /// Write `value` (low `size` bytes) to a (possibly translated) address.
    /// MMIO regions receive the write; RAM gets a plain store.
    pub fn write_to_addr(&mut self, addr: u64, value: u64, size: u8) -> Result<(), MemoryError> {
        let phys = self.physical_address(addr, AccessKind::Write, size as u64)?;
        if let Some(region) = self.find_mmio_mut(phys, size as u64) {
            return region
                .write(addr, value, size)
                .map_err(|_| MemoryError::MmioError);
        }
        self.ram_write(phys, value, size as usize)
    }

    pub fn read_bytes(&self, virt: u64, len: usize) -> Result<Vec<u8>, MemoryError> {
        let mut out = Vec::with_capacity(len);
        let mut offset = 0usize;
        while offset < len {
            let addr = virt + offset as u64;
            let remaining = len - offset;
            let chunk = remaining.min(PAGE_SIZE - (addr as usize & (PAGE_SIZE - 1)));
            let phys = self.physical_address(addr, AccessKind::Read, chunk as u64)?;
            if let Some(region) = self.find_mmio(phys, chunk as u64) {
                for i in 0..chunk {
                    out.push(region.read(addr + i as u64, 1)? as u8);
                }
            } else {
                let start = phys as usize;
                let end = start + chunk;
                if end > self.ram.len() {
                    return Err(MemoryError::InvalidAddress);
                }
                out.extend_from_slice(&self.ram[start..end]);
            }
            offset += chunk;
        }
        Ok(out)
    }

    pub fn write_bytes(&mut self, virt: u64, bytes: &[u8]) -> Result<(), MemoryError> {
        let len = bytes.len();
        let mut offset = 0usize;
        while offset < len {
            let addr = virt + offset as u64;
            let remaining = len - offset;
            let chunk = remaining.min(PAGE_SIZE - (addr as usize & (PAGE_SIZE - 1)));
            let phys = self.physical_address(addr, AccessKind::Write, chunk as u64)?;
            if self.find_mmio(phys, chunk as u64).is_some() {
                return Err(MemoryError::AccessDenied);
            }
            let start = phys as usize;
            let end = start + chunk;
            if end > self.ram.len() {
                return Err(MemoryError::InvalidAddress);
            }
            self.ram[start..end].copy_from_slice(&bytes[offset..offset + chunk]);
            offset += chunk;
        }
        Ok(())
    }

    pub fn read_byte(&self, addr: u64) -> Result<u8, MemoryError> {
        if self.paging_enabled || !self.identity.is_empty() {
            let phys = self.physical_address(addr, AccessKind::Read, 1)?;
            if let Some(region) = self.find_mmio(phys, 1) {
                return Ok(region.read(addr, 1)? as u8);
            }
            return Ok(self.ram[phys as usize]);
        }
        if let Some(region) = self.find_mmio(addr, 1) {
            return Ok(region.read(addr, 1)? as u8);
        }
        if (addr as usize) < self.ram.len() {
            Ok(self.ram[addr as usize])
        } else {
            Err(MemoryError::InvalidAddress)
        }
    }

    pub fn write_byte(&mut self, addr: u64, value: u8) -> Result<(), MemoryError> {
        if self.paging_enabled || !self.identity.is_empty() {
            let phys = self.physical_address(addr, AccessKind::Write, 1)?;
            if self.find_mmio(phys, 1).is_some() {
                return Err(MemoryError::AccessDenied);
            }
            if (phys as usize) < self.ram.len() {
                self.ram[phys as usize] = value;
                return Ok(());
            }
            return Err(MemoryError::InvalidAddress);
        }
        if self.find_mmio(addr, 1).is_some() {
            return Err(MemoryError::AccessDenied);
        }
        if (addr as usize) < self.ram.len() {
            self.ram[addr as usize] = value;
            Ok(())
        } else {
            Err(MemoryError::InvalidAddress)
        }
    }

    pub fn read_u16(&self, addr: u64) -> Result<u16, MemoryError> {
        let bytes = self.read_bytes(addr, 2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn write_u16(&mut self, addr: u64, value: u16) -> Result<(), MemoryError> {
        self.write_bytes(addr, &value.to_le_bytes())
    }

    pub fn read_u32(&self, addr: u64) -> Result<u32, MemoryError> {
        let bytes = self.read_bytes(addr, 4)?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    pub fn write_u32(&mut self, addr: u64, value: u32) -> Result<(), MemoryError> {
        self.write_bytes(addr, &value.to_le_bytes())
    }

    pub fn read_u64(&self, addr: u64) -> Result<u64, MemoryError> {
        let bytes = self.read_bytes(addr, 8)?;
        Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
    }

    pub fn write_u64(&mut self, addr: u64, value: u64) -> Result<(), MemoryError> {
        self.write_bytes(addr, &value.to_le_bytes())
    }

    /// Read a 16-byte IDT/GDT descriptor from guest memory.
    pub fn read_descriptor(&self, addr: u64) -> Result<[u8; 16], MemoryError> {
        let bytes = self.read_bytes(addr, 16)?;
        Ok(bytes.try_into().unwrap())
    }

    /// Allocate a physical frame from the allocator.
    pub fn alloc_frame(&mut self) -> Result<u64, MemoryError> {
        self.allocator.alloc().ok_or(MemoryError::OutOfMemory)
    }

    /// Allocate a zeroed physical frame from the allocator.
    pub fn alloc_zeroed_frame(&mut self) -> Result<u64, MemoryError> {
        self.allocator
            .alloc_zeroed(&mut self.ram)
            .ok_or(MemoryError::OutOfMemory)
    }

    /// Free a physical frame back to the allocator.
    pub fn free_frame(&mut self, addr: u64) {
        self.allocator.free(addr);
    }

    /// Direct physical (non-translated) write. Used by the loader to place
    /// the kernel image at a physical address regardless of paging state.
    pub fn write_phys(&mut self, phys: u64, bytes: &[u8]) -> Result<(), MemoryError> {
        let start = phys as usize;
        let end = start.checked_add(bytes.len()).ok_or(MemoryError::InvalidAddress)?;
        if end > self.ram.len() {
            return Err(MemoryError::InvalidAddress);
        }
        self.ram[start..end].copy_from_slice(bytes);
        Ok(())
    }

    /// Direct physical (non-translated) read. Used by the BIOS and loader to
    /// inspect guest memory without a mapping.
    pub fn read_phys(&self, phys: u64, len: usize) -> Result<Vec<u8>, MemoryError> {
        let start = phys as usize;
        let end = start.checked_add(len).ok_or(MemoryError::InvalidAddress)?;
        if end > self.ram.len() {
            return Err(MemoryError::InvalidAddress);
        }
        Ok(self.ram[start..end].to_vec())
    }

    /// Install an identity mapping for a physical range in the bootstrap
    /// identity map (used while paging is disabled or in early boot).
    pub fn identity_map_range(
        &mut self,
        start: u64,
        size: u64,
        flags: PageFlags,
    ) -> Result<(), MemoryError> {
        let page = PAGE_SIZE as u64;
        let mut addr = start & !(page - 1);
        let end = start.saturating_add(size);
        while addr < end {
            self.identity.insert(addr, flags);
            addr = addr.saturating_add(page);
        }
        Ok(())
    }

    pub fn clear_identity_map(&mut self) {
        self.identity.clear();
    }

    /// Map one virtual page to one physical frame using guest-visible page
    /// tables stored in RAM. Returns the physical frame allocated for the
    /// page-table entries if new tables were needed.
    pub fn map_page(
        &mut self,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), MemoryError> {
        if virt & (PAGE_SIZE as u64 - 1) != 0 || phys & (PAGE_SIZE as u64 - 1) != 0 {
            return Err(MemoryError::AlignmentError);
        }
        self.page_tables_map(virt, phys, flags)
    }

    pub fn unmap_page(&mut self, virt: u64) -> Result<(), MemoryError> {
        if virt & (PAGE_SIZE as u64 - 1) != 0 {
            return Err(MemoryError::AlignmentError);
        }
        self.page_tables_unmap(virt)
    }

    fn page_tables_map(
        &mut self,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), MemoryError> {
        let mut cr3 = self.cr3;
        if cr3 == 0 {
            cr3 = self.alloc_zeroed_frame()?;
            self.cr3 = cr3;
        }

        let pml4_idx = (virt >> 39) & 0x1FF;
        let pdpt_idx = (virt >> 30) & 0x1FF;
        let pd_idx = (virt >> 21) & 0x1FF;
        let pt_idx = (virt >> 12) & 0x1FF;

        let pml4e_addr = cr3 + pml4_idx * 8;
        let pml4e = Pte {
            raw: self.ram[(pml4e_addr as usize)..(pml4e_addr as usize + 8)]
                .try_into()
                .map(u64::from_le_bytes)
                .unwrap(),
        };
        let pdpt_addr = if pml4e.present() {
            pml4e.phys()
        } else {
            let f = self.alloc_zeroed_frame()?;
            self.ram[(pml4e_addr as usize)..(pml4e_addr as usize + 8)]
                .copy_from_slice(&(f | 0x007).to_le_bytes());
            f
        };

        let pdpte_addr = pdpt_addr + pdpt_idx * 8;
        let pdpte = Pte {
            raw: self.ram[(pdpte_addr as usize)..(pdpte_addr as usize + 8)]
                .try_into()
                .map(u64::from_le_bytes)
                .unwrap(),
        };
        let pd_addr = if pdpte.present() {
            pdpte.phys()
        } else {
            let f = self.alloc_zeroed_frame()?;
            self.ram[(pdpte_addr as usize)..(pdpte_addr as usize + 8)]
                .copy_from_slice(&(f | 0x007).to_le_bytes());
            f
        };

        let pde_addr = pd_addr + pd_idx * 8;
        let pde = Pte {
            raw: self.ram[(pde_addr as usize)..(pde_addr as usize + 8)]
                .try_into()
                .map(u64::from_le_bytes)
                .unwrap(),
        };
        let pt_addr = if pde.present() {
            pde.phys()
        } else {
            let f = self.alloc_zeroed_frame()?;
            self.ram[(pde_addr as usize)..(pde_addr as usize + 8)]
                .copy_from_slice(&(f | 0x007).to_le_bytes());
            f
        };

        let pte_addr = pt_addr + pt_idx * 8;
        // Force PRESENT so the mapping is usable, but respect the caller's
        // writable/user/NX bits for permission enforcement.
        let pte = phys | 0x001 | flags.bits();
        self.ram[(pte_addr as usize)..(pte_addr as usize + 8)].copy_from_slice(&pte.to_le_bytes());
        Ok(())
    }

    fn page_tables_unmap(&mut self, virt: u64) -> Result<(), MemoryError> {
        if self.cr3 == 0 {
            return Ok(());
        }
        let pml4_idx = ((virt >> 39) & 0x1FF) as usize;
        let pdpt_idx = ((virt >> 30) & 0x1FF) as usize;
        let pd_idx = ((virt >> 21) & 0x1FF) as usize;
        let pt_idx = ((virt >> 12) & 0x1FF) as usize;

        let read = |mem: &[u8], addr: u64| -> Option<u64> {
            let a = addr as usize;
            if a + 8 > mem.len() {
                return None;
            }
            Some(u64::from_le_bytes(mem[a..a + 8].try_into().ok()?))
        };

        let pml4e = read(&self.ram, self.cr3 + (pml4_idx as u64) * 8)
            .ok_or(MemoryError::InvalidAddress)?;
        let pml4e = Pte { raw: pml4e };
        if !pml4e.present() {
            return Ok(());
        }
        let pdpte = read(&self.ram, pml4e.phys() + (pdpt_idx as u64) * 8)
            .ok_or(MemoryError::InvalidAddress)?;
        let pdpte = Pte { raw: pdpte };
        if !pdpte.present() {
            return Ok(());
        }
        let pde = read(&self.ram, pdpte.phys() + (pd_idx as u64) * 8)
            .ok_or(MemoryError::InvalidAddress)?;
        let pde = Pte { raw: pde };
        if !pde.present() || pde.large() {
            return Ok(());
        }
        let pte_addr = pde.phys() + (pt_idx as u64) * 8;
        if (pte_addr as usize) + 8 <= self.ram.len() {
            self.ram[pte_addr as usize..pte_addr as usize + 8].fill(0);
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.allocator.reset();
        self.ram.fill(0);
        self.identity.clear();
        self.paging_enabled = false;
        self.cr3 = 0;
        self.privilege = false;
    }
}

impl Default for Mmu {
    fn default() -> Self {
        Self::new(128 * 1024 * 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{Device, DeviceError};

    struct TestDevice {
        value: u64,
    }

    impl Device for TestDevice {
        fn read(&self, addr: u64, _size: u8) -> Result<u64, DeviceError> {
            Ok(self.value + (addr & 0xFF))
        }
        fn write(&mut self, addr: u64, value: u64, _size: u8) -> Result<(), DeviceError> {
            self.value = value;
            Ok(())
        }
        fn reset(&mut self) {
            self.value = 0;
        }
    }

    #[test]
    fn frame_allocator_round_trip() {
        let mut mmu = Mmu::new(1 << 20);
        {
            let a = mmu.allocator();
            assert_eq!(a.total_frames(), 256);
            assert_eq!(a.free_frames(), 256);
        }
        let f1 = mmu.alloc_frame().unwrap();
        let f2 = mmu.alloc_frame().unwrap();
        assert_ne!(f1, f2);
        assert!(f1 % PAGE_SIZE as u64 == 0);
        mmu.free_frame(f1);
        let f3 = mmu.alloc_frame().unwrap();
        assert_eq!(f1, f3);
        assert_ne!(f2, f3);
    }

    #[test]
    fn physical_read_write() {
        let mut mmu = Mmu::new(1 << 20);
        mmu.write_phys(0x1000, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
        assert_eq!(mmu.read_phys(0x1000, 4).unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn identity_map_permissions() {
        let mut mmu = Mmu::new(1 << 20);
        mmu.identity_map_range(0, PAGE_SIZE as u64, PageFlags::PRESENT | PageFlags::WRITABLE)
            .unwrap();
        mmu.write_byte(0x100, 0x42).unwrap();
        assert_eq!(mmu.read_byte(0x100).unwrap(), 0x42);

        // User access to a supervisor page is denied.
        mmu.set_privilege(true);
        assert_eq!(mmu.read_byte(0x100), Err(MemoryError::AccessDenied));
    }

    #[test]
    fn mmio_routing_on_top() {
        let mut mmu = Mmu::new(1 << 20);
        mmu.attach_mmio(0x8000_0000, 0x100, Box::new(TestDevice { value: 0x77 }));
        assert_eq!(mmu.read_byte(0x8000_0010).unwrap(), 0x77 + 0x10);
        // Writes through the byte path are denied for MMIO regions (device
        // writes go through the executor's mmio_write path).
        assert_eq!(
            mmu.write_byte(0x8000_0010, 1),
            Err(MemoryError::AccessDenied)
        );
    }

    #[test]
    fn paging_supports_all_levels() {
        let mut mmu = Mmu::new(4 << 20);
        mmu.set_paging(true, 0);
        let flags = PageFlags::PRESENT | PageFlags::WRITABLE;
        mmu.map_page(0x0000_1000, 0x0000_2000, flags).unwrap();
        assert_ne!(mmu.cr3(), 0, "CR3 must be allocated by map_page");

        mmu.write_byte(0x0000_1000, 0x5A).unwrap();
        assert_eq!(mmu.ram[0x0000_2000], 0x5A);
        // Virtual 0x2000 is not mapped, so writing through it must fault.
        assert_eq!(
            mmu.write_byte(0x0000_2000, 0x6B),
            Err(MemoryError::PageFault)
        );
        assert_eq!(mmu.ram[0x0000_2000], 0x5A, "physical frame must not alias");

        // Unmapped page faults.
        assert_eq!(mmu.read_byte(0x0000_3000), Err(MemoryError::PageFault));
        // Non-writable mapping rejects writes but allows reads.
        mmu.map_page(0x0000_4000, 0x0000_3000, PageFlags::PRESENT).unwrap();
        assert_eq!(
            mmu.write_byte(0x0000_4000, 0xFF),
            Err(MemoryError::AccessDenied)
        );
        assert_eq!(mmu.read_byte(0x0000_4000).unwrap(), 0);

        mmu.unmap_page(0x0000_1000).unwrap();
        assert_eq!(mmu.read_byte(0x0000_1000), Err(MemoryError::PageFault));
    }

    #[test]
    fn large_page_walk() {
        let mut mmu = Mmu::new(8 << 20);
        mmu.set_paging(true, 0);

        // Build a two-level page table by hand so we can install a 2 MiB
        // large page (PDE with the PS bit set).
        let pml4 = mmu.alloc_zeroed_frame().unwrap();
        let pdpt = mmu.alloc_zeroed_frame().unwrap();
        let pd = mmu.alloc_zeroed_frame().unwrap();

        // PML4[0] -> PDPT
        let pml4_idx = 0usize;
        mmu.ram[(pml4 as usize) + pml4_idx * 8..(pml4 as usize) + pml4_idx * 8 + 8]
            .copy_from_slice(&(pdpt | 0x007).to_le_bytes());
        // PDPT[0] -> PD
        let pdpt_idx = 0usize;
        mmu.ram[(pdpt as usize) + pdpt_idx * 8..(pdpt as usize) + pdpt_idx * 8 + 8]
            .copy_from_slice(&(pd | 0x007).to_le_bytes());
        // PD[0] -> 2 MiB large page at physical 0x200000 (PS bit 0x80 set).
        let pd_idx = 0usize;
        let large_phys = 0x2000_00u64;
        let large_flags = 0x007u64 | 0x080u64; // present | writable | large
        mmu.ram[(pd as usize) + pd_idx * 8..(pd as usize) + pd_idx * 8 + 8]
            .copy_from_slice(&(large_phys | large_flags).to_le_bytes());

        mmu.set_paging(true, pml4);
        assert_ne!(mmu.cr3(), 0);

        // Write through the large-page virtual alias.
        mmu.write_byte(0x0000_0000, 0xAB).unwrap();
        assert_eq!(mmu.ram[0x2000_00], 0xAB);

        // Offset within the large page.
        mmu.write_byte(0x0000_1000, 0xCD).unwrap();
        assert_eq!(mmu.ram[0x2010_00], 0xCD);
    }

    #[test]
    fn bounds_checks() {
        let mut mmu = Mmu::new(1 << 20);
        assert_eq!(
            mmu.write_byte((1 << 20) as u64, 1),
            Err(MemoryError::InvalidAddress)
        );
        assert_eq!(
            mmu.read_byte((1 << 20) as u64),
            Err(MemoryError::InvalidAddress)
        );
    }
}