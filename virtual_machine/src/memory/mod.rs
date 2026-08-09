//! Memory management: frame allocator, four-level paging, MMIO routing, and
//! access validation.

use crate::devices::{Device, DeviceError, MmioRegion};
use crate::replay::{ReplayDmaWrite, ReplayMode, SharedReplay};
use std::collections::{HashMap, HashSet};

pub const PAGE_SIZE: usize = 4096;
pub const PAGE_SHIFT: u64 = 12;
pub const LARGE_PAGE_2M: usize = 2 * 1024 * 1024;
pub const LARGE_PAGE_1G: usize = 1024 * 1024 * 1024;

/// Hardware page sizes supported by the page-table builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LargePageSize {
    TwoMiB,
    OneGiB,
}

impl LargePageSize {
    pub const fn bytes(self) -> usize {
        match self {
            Self::TwoMiB => LARGE_PAGE_2M,
            Self::OneGiB => LARGE_PAGE_1G,
        }
    }
}

/// A compact view of the MMU's reclaimable and lazily committed memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryStats {
    pub total_frames: usize,
    pub free_frames: usize,
    pub ballooned_frames: usize,
    pub cow_pages: usize,
    pub overcommitted_pages: usize,
    pub overcommit_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    InvalidAddress,
    PageFault,
    AccessDenied,
    AlignmentError,
    OutOfMemory,
    MmioError,
    ReplayDivergence,
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

    fn add_memory(&mut self, old_size: usize, new_size: usize) {
        let old_frames = old_size / PAGE_SIZE;
        let new_frames = new_size / PAGE_SIZE;
        if new_frames <= old_frames {
            return
        }
        self.free.extend(old_frames..new_frames);
        self.total_frames = new_frames;
    }

    pub fn alloc(&mut self) -> Option<u64> {
        self.free.pop().map(|f| f as u64 * PAGE_SIZE as u64)
    }

    fn alloc_excluding(&mut self, blocked: &HashSet<u64>) -> Option<u64> {
        let mut skipped = Vec::new();
        let result = loop {
            let Some(frame) = self.free.pop() else {
                self.free.extend(skipped);
                return None;
            };
            let addr = frame as u64 * PAGE_SIZE as u64;
            if blocked.contains(&addr) {
                skipped.push(frame);
            } else {
                break Some(addr);
            }
        };
        self.free.extend(skipped);
        result
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

    fn take(&mut self, addr: u64) -> bool {
        if addr % PAGE_SIZE as u64 != 0 {
            return false;
        }
        let frame = (addr / PAGE_SIZE as u64) as usize;
        if let Some(index) = self.free.iter().position(|candidate| *candidate == frame) {
            self.free.swap_remove(index);
            true
        } else {
            false
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

    fn large(&self) -> bool {
        self.raw & 0x80 != 0
    }

    fn phys(&self) -> u64 {
        self.raw & 0x000F_FFFF_FFFF_F000
    }
}

#[derive(Clone, Copy, Debug)]
struct CowMapping {
    phys: u64,
    flags: PageFlags,
    overcommitted: bool,
}

/// Serializable MMU state. MMIO registrations are owned by the VM topology
/// and are not replaced during restore; their guest-visible RAM and mapping
/// state is restored here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MmuState {
    pub ram: Vec<u8>,
    pub free_frames: Vec<usize>,
    pub code_version: u64,
    pub identity: Vec<(u64, u64)>,
    pub cow_pages: Vec<(u64, u64, u64, bool)>,
    pub mapped_frames: Vec<(u64, usize)>,
    pub mapped_pages: Vec<(u64, u64)>,
    pub ballooned_frames: Vec<u64>,
    pub zero_page: Option<u64>,
    pub overcommitted_pages: usize,
    pub overcommit_limit: usize,
    pub paging_enabled: bool,
    pub cr3: u64,
    pub privilege: bool,
}

/// Resolves `virt` through a four-level page table whose root (PML4) is at
/// physical address `cr3`, reading tables from guest RAM.
fn walk_page_table(mem: &[u8], cr3: u64, virt: u64) -> Option<(u64, PageFlags)> {
    let read_ptr = |addr: u64| -> Option<u64> {
        let addr = addr as usize;
        if addr.saturating_add(8) > mem.len() {
            return None;
        }
        Some(u64::from_le_bytes(mem[addr..addr + 8].try_into().unwrap()))
    };

    let pml4_idx = ((virt >> 39) & 0x1FF) as usize;
    let pdpt_idx = ((virt >> 30) & 0x1FF) as usize;
    let pd_idx = ((virt >> 21) & 0x1FF) as usize;
    let pt_idx = ((virt >> 12) & 0x1FF) as usize;

    let pml4e = read_ptr(cr3.checked_add((pml4_idx as u64) * 8)?)?;
    let pml4e = Pte { raw: pml4e };
    if !pml4e.present() {
        return None;
    }

    let pdpte = read_ptr(pml4e.phys().checked_add((pdpt_idx as u64) * 8)?)?;
    let pdpte = Pte { raw: pdpte };
    if !pdpte.present() {
        return None;
    }

    let effective_flags = |leaf: PageFlags| {
        let mut flags = leaf;
        if pml4e.raw & 0x2 == 0 || pdpte.raw & 0x2 == 0 {
            flags.remove(PageFlags::WRITABLE);
        }
        if pml4e.raw & 0x4 == 0 || pdpte.raw & 0x4 == 0 {
            flags.remove(PageFlags::USER);
        }
        if pml4e.raw & PageFlags::NX.bits() != 0 || pdpte.raw & PageFlags::NX.bits() != 0 {
            flags.insert(PageFlags::NX);
        }
        flags
    };

    // 1 GiB page
    if pdpte.large() {
        let mask = 0x3FFF_FFFFu64;
        return Some((
            pdpte.phys().checked_add(virt & mask)?,
            effective_flags(PageFlags::from_bits_truncate(pdpte.raw)),
        ));
    }

    let pde = read_ptr(pdpte.phys().checked_add((pd_idx as u64) * 8)?)?;
    let pde = Pte { raw: pde };
    if !pde.present() {
        return None;
    }

    // 2 MiB page
    if pde.large() {
        let mask = 0x1F_FFFFu64;
        return Some((
            pde.phys().checked_add(virt & mask)?,
            effective_flags(PageFlags::from_bits_truncate(pde.raw)),
        ));
    }

    let pte = read_ptr(pde.phys().checked_add((pt_idx as u64) * 8)?)?;
    let pte = Pte { raw: pte };
    if !pte.present() {
        return None;
    }

    let offset = virt & (PAGE_SIZE as u64 - 1);
    let mut flags = effective_flags(PageFlags::from_bits_truncate(pte.raw));
    if pde.raw & 0x2 == 0 {
        flags.remove(PageFlags::WRITABLE);
    }
    if pde.raw & 0x4 == 0 {
        flags.remove(PageFlags::USER);
    }
    if pde.raw & PageFlags::NX.bits() != 0 {
        flags.insert(PageFlags::NX);
    }
    Some((
        pte.phys().checked_add(offset)?,
        flags,
    ))
}

/// Return every guest-physical page currently used by the active page-table
/// tree. Guest writes to one of these pages can change the meaning or
/// permissions of a translated virtual address.
fn collect_page_table_pages(mem: &[u8], cr3: u64) -> HashSet<u64> {
    fn visit(mem: &[u8], page: u64, level: u8, pages: &mut HashSet<u64>) {
        let page = page & !(PAGE_SIZE as u64 - 1);
        if (page as usize) >= mem.len() || !pages.insert(page) {
            return
        }
        if level == 1 {
            return
        }
        let start = page as usize;
        for index in 0..512usize {
            let offset = index * 8;
            let Some(end) = start.checked_add(offset + 8) else {
                return
            };
            if end > mem.len() {
                return
            }
            let raw = u64::from_le_bytes(mem[start + offset..end].try_into().unwrap());
            let entry = Pte { raw };
            if !entry.present() || (level <= 3 && entry.large()) {
                continue
            }
            visit(mem, entry.phys(), level - 1, pages);
        }
    }

    let mut pages = HashSet::new();
    if cr3 != 0 {
        visit(mem, cr3, 4, &mut pages);
    }
    pages
}

/// Central MMU: physical RAM, frame allocation, page tables, MMIO routing,
/// and permission checks.
pub struct Mmu {
    ram: Vec<u8>,
    allocator: FrameAllocator,
    mmio: Vec<MmioRegion>,
    /// Monotonically increasing version for writes to pages containing
    /// translated guest code.
    code_version: u64,
    translated_code_pages: HashSet<u64>,
    /// Monotonically increasing version for changes to address translation or
    /// access permissions. This is separate from code bytes because a page
    /// can keep the same bytes while its virtual mapping changes.
    translation_version: u64,
    page_table_pages: HashSet<u64>,
    // Identity/physical mappings the CPU sets up for bootstrap. Key: physical
    // frame address, value: PageFlags.
    identity: HashMap<u64, PageFlags>,
    // Virtual pages participating in copy-on-write. The guest page tables are
    // still authoritative; this side table supplies the write-fault action.
    cow_pages: HashMap<u64, CowMapping>,
    // Number of virtual mappings that refer to each 4 KiB physical frame.
    mapped_frames: HashMap<u64, usize>,
    mapped_pages: HashMap<u64, u64>,
    ballooned_frames: HashSet<u64>,
    zero_page: Option<u64>,
    overcommitted_pages: usize,
    overcommit_limit: usize,
    paging_enabled: bool,
    cr3: u64,
    privilege: bool, // false = kernel (ring 0), true = user (ring 3)
    replay: Option<SharedReplay>,
    instruction_ip: Option<u64>,
    dma_capture: bool,
    dma_writes: Vec<ReplayDmaWrite>,
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
        let total_frames = size / PAGE_SIZE;
        let mut ram = Vec::with_capacity(size);
        ram.resize(size, 0);
        Self {
            allocator: FrameAllocator::new(size),
            ram,
            mmio: Vec::new(),
            code_version: 0,
            translated_code_pages: HashSet::new(),
            translation_version: 0,
            page_table_pages: HashSet::new(),
            identity: HashMap::new(),
            cow_pages: HashMap::new(),
            mapped_frames: HashMap::new(),
            mapped_pages: HashMap::new(),
            ballooned_frames: HashSet::new(),
            zero_page: None,
            overcommitted_pages: 0,
            overcommit_limit: total_frames.saturating_mul(4),
            paging_enabled: false,
            cr3: 0,
            privilege: false,
            replay: None,
            instruction_ip: None,
            dma_capture: false,
            dma_writes: Vec::new(),
        }
    }

    pub fn ram_size(&self) -> usize {
        self.ram.len()
    }

    /// Add page-aligned physical RAM at the end of the current RAM range.
    /// Existing guest mappings and contents remain untouched.
    pub fn hotplug_memory(&mut self, size: usize) -> Result<u64, MemoryError> {
        if size == 0 || size % PAGE_SIZE != 0 {
            return Err(MemoryError::AlignmentError)
        }
        let old_size = self.ram.len();
        let new_size = old_size
            .checked_add(size)
            .ok_or(MemoryError::InvalidAddress)?;
        self.ram.resize(new_size, 0);
        self.allocator.add_memory(old_size, new_size);
        self.overcommit_limit = self
            .overcommit_limit
            .saturating_add((size / PAGE_SIZE).saturating_mul(4));
        Ok(old_size as u64)
    }

    pub(crate) fn snapshot_state(&self) -> MmuState {
        MmuState {
            ram: self.ram.clone(),
            free_frames: self.allocator.free.clone(),
            code_version: self.code_version,
            identity: self
                .identity
                .iter()
                .map(|(&address, &flags)| (address, flags.bits()))
                .collect(),
            cow_pages: self
                .cow_pages
                .iter()
                .map(|(&virt, mapping)| {
                    (virt, mapping.phys, mapping.flags.bits(), mapping.overcommitted)
                })
                .collect(),
            mapped_frames: self
                .mapped_frames
                .iter()
                .map(|(&frame, &count)| (frame, count))
                .collect(),
            mapped_pages: self
                .mapped_pages
                .iter()
                .map(|(&page, &frame)| (page, frame))
                .collect(),
            ballooned_frames: self.ballooned_frames.iter().copied().collect(),
            zero_page: self.zero_page,
            overcommitted_pages: self.overcommitted_pages,
            overcommit_limit: self.overcommit_limit,
            paging_enabled: self.paging_enabled,
            cr3: self.cr3,
            privilege: self.privilege,
        }
    }

    pub(crate) fn restore_state(&mut self, state: &MmuState) -> Result<(), MemoryError> {
        if state.ram.len() != self.ram.len()
            || state.free_frames.iter().any(|&frame| frame >= self.allocator.total_frames)
        {
            return Err(MemoryError::InvalidAddress);
        }

        self.ram.clone_from(&state.ram);
        self.allocator.free = state.free_frames.clone();
        self.code_version = state.code_version;
        self.translated_code_pages.clear();
        self.identity = state
            .identity
            .iter()
            .map(|&(address, flags)| (address, PageFlags::from_bits_truncate(flags)))
            .collect();
        self.cow_pages = state
            .cow_pages
            .iter()
            .map(|&(virt, phys, flags, overcommitted)| {
                (
                    virt,
                    CowMapping {
                        phys,
                        flags: PageFlags::from_bits_truncate(flags),
                        overcommitted,
                    },
                )
            })
            .collect();
        self.mapped_frames = state.mapped_frames.iter().copied().collect();
        self.mapped_pages = state.mapped_pages.iter().copied().collect();
        self.ballooned_frames = state.ballooned_frames.iter().copied().collect();
        self.zero_page = state.zero_page;
        self.overcommitted_pages = state.overcommitted_pages;
        self.overcommit_limit = state.overcommit_limit;
        self.paging_enabled = state.paging_enabled;
        self.cr3 = state.cr3;
        self.privilege = state.privilege;
        self.translation_version = self.translation_version.wrapping_add(1);
        self.page_table_pages = collect_page_table_pages(&self.ram, self.cr3);
        Ok(())
    }

    /// Version of guest RAM used by the CPU translation cache.
    pub fn code_version(&self) -> u64 {
        self.code_version
    }

    /// Version of the address-translation and permission state used by the
    /// CPU translation cache.
    pub fn translation_version(&self) -> u64 {
        self.translation_version
    }

    /// Refresh the set of guest pages that form the active page-table tree.
    /// The execution engine calls this at dispatch boundaries so direct guest
    /// edits to newly linked tables are tracked on the next block.
    pub(crate) fn refresh_page_table_pages(&mut self) {
        self.page_table_pages = collect_page_table_pages(&self.ram, self.cr3);
    }

    pub(crate) fn mark_code_range(&mut self, virt: u64, len: usize) {
        if len == 0 {
            return
        }
        let end = virt.saturating_add(len.saturating_sub(1) as u64);
        let mut page = virt & !(PAGE_SIZE as u64 - 1);
        let last_page = end & !(PAGE_SIZE as u64 - 1);
        loop {
            if let Ok(phys) = self.physical_address(page, AccessKind::Read, 1) {
                if self.find_mmio(phys, 1).is_none() && (phys as usize) < self.ram.len() {
                    self.translated_code_pages
                        .insert(phys & !(PAGE_SIZE as u64 - 1));
                }
            }
            if page == last_page {
                break
            }
            page = page.saturating_add(PAGE_SIZE as u64);
        }
    }

    fn record_ram_write(&mut self, phys: u64, size: usize) {
        if size == 0 {
            return
        }
        let Some(end) = phys.checked_add(size.saturating_sub(1) as u64) else {
            return
        };
        let mut page = phys & !(PAGE_SIZE as u64 - 1);
        let last_page = end & !(PAGE_SIZE as u64 - 1);
        let mut page_table_changed = false;
        loop {
            if self.page_table_pages.contains(&page) {
                page_table_changed = true;
            }
            if self.translated_code_pages.contains(&page) {
                self.code_version = self.code_version.wrapping_add(1);
            }
            if page == last_page {
                break
            }
            page = page.saturating_add(PAGE_SIZE as u64);
        }
        if page_table_changed {
            self.translation_version = self.translation_version.wrapping_add(1);
        }
    }

    pub fn allocator(&self) -> &FrameAllocator {
        &self.allocator
    }

    pub fn allocator_mut(&mut self) -> &mut FrameAllocator {
        &mut self.allocator
    }

    pub fn memory_stats(&self) -> MemoryStats {
        MemoryStats {
            total_frames: self.allocator.total_frames(),
            free_frames: self
                .allocator
                .free_frames()
                .saturating_sub(self.ballooned_frames.len()),
            ballooned_frames: self.ballooned_frames.len(),
            cow_pages: self.cow_pages.len(),
            overcommitted_pages: self.overcommitted_pages,
            overcommit_limit: self.overcommit_limit,
        }
    }

    /// Set the maximum number of virtual pages that may use lazy
    /// overcommit. Existing mappings are not evicted when the limit shrinks.
    pub fn set_overcommit_limit(&mut self, pages: usize) {
        self.overcommit_limit = pages;
    }

    pub fn overcommit_limit(&self) -> usize {
        self.overcommit_limit
    }

    pub fn overcommitted_pages(&self) -> usize {
        self.overcommitted_pages
    }

    pub fn is_cow_page(&self, virt: u64) -> bool {
        self.cow_pages
            .contains_key(&(virt & !(PAGE_SIZE as u64 - 1)))
    }

    /// Return physical pages to the host through a balloon. Pages must not be
    /// mapped or be the shared zero page.
    pub fn balloon_inflate(&mut self, frames: &[u64]) -> Result<(), MemoryError> {
        let ram_frame_end = self.allocator.total_frames() as u64 * PAGE_SIZE as u64;
        for &frame in frames {
            if frame % PAGE_SIZE as u64 != 0
                || frame >= ram_frame_end
                || self.zero_page == Some(frame)
                || self.mapped_frames.get(&frame).copied().unwrap_or(0) != 0
            {
                return Err(MemoryError::AccessDenied);
            }
        }
        for &frame in frames {
            self.allocator.free(frame);
            self.ballooned_frames.insert(frame);
        }
        Ok(())
    }

    /// Give ballooned pages back to the guest allocator.
    pub fn balloon_deflate(&mut self, frames: &[u64]) -> Result<(), MemoryError> {
        for &frame in frames {
            if !self.ballooned_frames.contains(&frame) {
                return Err(MemoryError::InvalidAddress);
            }
            if !self.allocator.take(frame) {
                return Err(MemoryError::OutOfMemory);
            }
        }
        for &frame in frames {
            self.ballooned_frames.remove(&frame);
        }
        Ok(())
    }

    pub fn ballooned_frames(&self) -> usize {
        self.ballooned_frames.len()
    }

    pub fn inflate_balloon(&mut self, frames: &[u64]) -> Result<(), MemoryError> {
        self.balloon_inflate(frames)
    }

    pub fn deflate_balloon(&mut self, frames: &[u64]) -> Result<(), MemoryError> {
        self.balloon_deflate(frames)
    }

    pub fn set_paging(&mut self, enabled: bool, cr3: u64) {
        if self.paging_enabled != enabled || self.cr3 != cr3 {
            self.translation_version = self.translation_version.wrapping_add(1);
        }
        self.paging_enabled = enabled;
        self.cr3 = cr3;
        self.refresh_page_table_pages();
    }

    pub fn paging_enabled(&self) -> bool {
        self.paging_enabled
    }

    pub fn cr3(&self) -> u64 {
        self.cr3
    }

    pub fn set_privilege(&mut self, user: bool) {
        if self.privilege != user {
            self.translation_version = self.translation_version.wrapping_add(1);
        }
        self.privilege = user;
    }

    pub fn attach_mmio(&mut self, base: u64, size: u64, device: Box<dyn Device>) {
        self.mmio.push(MmioRegion::new(base, size, device));
    }

    pub fn attach_replay(&mut self, replay: SharedReplay) {
        self.replay = Some(replay)
    }

    pub fn set_replay_instruction_ip(&mut self, instruction_ip: Option<u64>) {
        self.instruction_ip = instruction_ip
    }

    pub(crate) fn begin_dma_capture(&mut self) {
        self.dma_capture = true;
        self.dma_writes.clear();
    }

    pub(crate) fn take_dma_writes(&mut self) -> Vec<ReplayDmaWrite> {
        self.dma_capture = false;
        std::mem::take(&mut self.dma_writes)
    }

    pub(crate) fn apply_dma_writes(
        &mut self,
        writes: &[ReplayDmaWrite],
    ) -> Result<(), MemoryError> {
        for write in writes {
            self.write_phys(write.address, &write.bytes)?;
        }
        Ok(())
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

    fn track_mapping(&mut self, virt: u64, phys: u64) {
        let page = virt & !(PAGE_SIZE as u64 - 1);
        let frame = phys & !(PAGE_SIZE as u64 - 1);
        self.mapped_pages.insert(page, frame);
        *self.mapped_frames.entry(frame).or_insert(0) += 1;
        self.cow_pages.remove(&page);
    }

    fn untrack_mapping(&mut self, virt: u64, phys: u64) {
        let page = virt & !(PAGE_SIZE as u64 - 1);
        let frame = phys & !(PAGE_SIZE as u64 - 1);
        self.mapped_pages.remove(&page);
        if let Some(count) = self.mapped_frames.get_mut(&frame) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.mapped_frames.remove(&frame);
            }
        }
        if let Some(mapping) = self.cow_pages.remove(&page) {
            if mapping.overcommitted {
                self.overcommitted_pages = self.overcommitted_pages.saturating_sub(1);
            }
        }
    }

    fn read_pte(&self, addr: u64) -> Result<Pte, MemoryError> {
        let start = addr as usize;
        let end = start.checked_add(8).ok_or(MemoryError::InvalidAddress)?;
        if end > self.ram.len() {
            return Err(MemoryError::InvalidAddress);
        }
        Ok(Pte {
            raw: u64::from_le_bytes(self.ram[start..end].try_into().unwrap()),
        })
    }

    fn write_pte(&mut self, addr: u64, pte: Pte) -> Result<(), MemoryError> {
        let start = addr as usize;
        let end = start.checked_add(8).ok_or(MemoryError::InvalidAddress)?;
        if end > self.ram.len() {
            return Err(MemoryError::InvalidAddress);
        }
        self.ram[start..end].copy_from_slice(&pte.raw.to_le_bytes());
        self.page_table_pages
            .insert(addr & !(PAGE_SIZE as u64 - 1));
        self.translation_version = self.translation_version.wrapping_add(1);
        Ok(())
    }

    fn update_page_mapping(
        &mut self,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), MemoryError> {
        let pml4e = self.read_pte(self.cr3 + ((virt >> 39) & 0x1FF) * 8)?;
        if !pml4e.present() {
            return Err(MemoryError::PageFault);
        }
        let pdpte_addr = pml4e.phys() + ((virt >> 30) & 0x1FF) * 8;
        let pdpte = self.read_pte(pdpte_addr)?;
        if !pdpte.present() || pdpte.large() {
            return Err(MemoryError::PageFault);
        }
        let pde_addr = pdpte.phys() + ((virt >> 21) & 0x1FF) * 8;
        let pde = self.read_pte(pde_addr)?;
        if !pde.present() || pde.large() {
            return Err(MemoryError::PageFault);
        }
        let pte_addr = pde.phys() + ((virt >> 12) & 0x1FF) * 8;
        self.write_pte(
            pte_addr,
            Pte {
                raw: phys | 0x001 | flags.bits(),
            },
        )
    }

    fn resolve_cow_page(&mut self, virt_page: u64) -> Result<(), MemoryError> {
        let Some(mapping) = self.cow_pages.get(&virt_page).copied() else {
            return Ok(());
        };
        let new_phys = self.alloc_zeroed_frame()?;
        let old_start = mapping.phys as usize;
        let new_start = new_phys as usize;
        let old_end = old_start
            .checked_add(PAGE_SIZE)
            .ok_or(MemoryError::InvalidAddress)?;
        let new_end = new_start
            .checked_add(PAGE_SIZE)
            .ok_or(MemoryError::InvalidAddress)?;
        if old_end > self.ram.len() || new_end > self.ram.len() {
            self.free_frame(new_phys);
            return Err(MemoryError::InvalidAddress);
        }
        let page = self.ram[old_start..old_end].to_vec();
        self.ram[new_start..new_end].copy_from_slice(&page);
        self.update_page_mapping(virt_page, new_phys, mapping.flags | PageFlags::WRITABLE)?;
        self.untrack_mapping(virt_page, mapping.phys);
        self.track_mapping(virt_page, new_phys);
        self.cow_pages.remove(&virt_page);
        Ok(())
    }

    fn prepare_write(&mut self, addr: u64, size: u64) -> Result<(), MemoryError> {
        if size == 0 {
            return Ok(());
        }
        let end = addr.checked_add(size - 1).ok_or(MemoryError::PageFault)?;
        let mut page = addr & !(PAGE_SIZE as u64 - 1);
        let last_page = end & !(PAGE_SIZE as u64 - 1);
        loop {
            self.resolve_cow_page(page)?;
            if page == last_page {
                break;
            }
            page = page
                .checked_add(PAGE_SIZE as u64)
                .ok_or(MemoryError::PageFault)?;
        }
        Ok(())
    }

    fn physical_address(
        &self,
        virt: u64,
        access: AccessKind,
        size: u64,
    ) -> Result<u64, MemoryError> {
        if size == 0 {
            return Err(MemoryError::InvalidAddress);
        }
        if self.paging_enabled {
            let upper = virt >> 48;
            if upper != 0 && upper != 0xFFFF {
                return Err(MemoryError::PageFault);
            }
            let max_addr = u64::MAX
                .checked_sub(size - 1)
                .ok_or(MemoryError::InvalidAddress)?;
            if virt > max_addr {
                return Err(MemoryError::PageFault);
            }
            let (phys, flags) =
                walk_page_table(&self.ram, self.cr3, virt).ok_or(MemoryError::PageFault)?;
            validate_flags(flags, access, self.privilege)?;
            phys.checked_add(size - 1).ok_or(MemoryError::PageFault)?;
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
        if size > 8 {
            return Err(MemoryError::InvalidAddress);
        }
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
        if size > 8 {
            return Err(MemoryError::InvalidAddress);
        }
        let start = phys as usize;
        let end = start.checked_add(size).ok_or(MemoryError::InvalidAddress)?;
        if end > self.ram.len() {
            return Err(MemoryError::InvalidAddress);
        }
        self.ram[start..end].copy_from_slice(&value.to_le_bytes()[..size]);
        self.record_ram_write(phys, size);
        Ok(())
    }

    /// Read `size` bytes from a (possibly translated) address. If the address
    /// falls inside a registered MMIO region, the whole access is routed to
    /// the device in one call; otherwise it reads from RAM. This is the
    /// canonical read used by the CPU executor's memory operands.
    pub fn read_from_addr(&self, addr: u64, size: u8) -> Result<u64, MemoryError> {
        if size == 0 || size > 8 {
            return Err(MemoryError::InvalidAddress);
        }
        if (addr & (PAGE_SIZE as u64 - 1)) + size as u64 > PAGE_SIZE as u64 {
            let mut value = 0u64;
            for offset in 0..size as u64 {
                value |= (self.read_from_addr(
                    addr.checked_add(offset).ok_or(MemoryError::InvalidAddress)?,
                    1,
                )? as u8 as u64)
                    << (offset * 8);
            }
            return Ok(value);
        }
        let phys = self.physical_address(addr, AccessKind::Read, size as u64)?;
        if let Some(region) = self.find_mmio(phys, size as u64) {
            let result = region.read(addr, size);
            let Some(replay) = &self.replay else {
                return result.map_err(|_| MemoryError::MmioError)
            };
            let mode = replay.borrow().mode();
            let actual = match mode {
                ReplayMode::Replaying => result.unwrap_or(0),
                ReplayMode::Disabled | ReplayMode::Recording => {
                    result.map_err(|_| MemoryError::MmioError)?
                }
            };
            return replay
                .borrow_mut()
                .instruction_input(
                    self.instruction_ip.unwrap_or(0),
                    phys,
                    size,
                    actual,
                )
                .map_err(|_| MemoryError::ReplayDivergence);
        }
        self.ram_read(phys, size as usize)
    }

    /// Write `value` (low `size` bytes) to a (possibly translated) address.
    /// MMIO regions receive the write; RAM gets a plain store.
    pub fn write_to_addr(&mut self, addr: u64, value: u64, size: u8) -> Result<(), MemoryError> {
        if size == 0 || size > 8 {
            return Err(MemoryError::InvalidAddress);
        }
        if (addr & (PAGE_SIZE as u64 - 1)) + size as u64 > PAGE_SIZE as u64 {
            self.prepare_write(addr, size as u64)?;
            for offset in 0..size as u64 {
                self.write_to_addr(
                    addr.checked_add(offset).ok_or(MemoryError::InvalidAddress)?,
                    (value >> (offset * 8)) & 0xFF,
                    1,
                )?;
            }
            return Ok(())
        }
        self.prepare_write(addr, size as u64)?;
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

    /// Compare guest memory without allocating a temporary buffer.
    pub fn bytes_equal(&self, virt: u64, expected: &[u8]) -> bool {
        let mut offset = 0usize;
        while offset < expected.len() {
            let addr = virt.wrapping_add(offset as u64);
            let remaining = expected.len() - offset;
            let chunk = remaining.min(PAGE_SIZE - (addr as usize & (PAGE_SIZE - 1)));
            let Ok(phys) = self.physical_address(addr, AccessKind::Read, chunk as u64) else {
                return false
            };
            if let Some(region) = self.find_mmio(phys, chunk as u64) {
                for index in 0..chunk {
                    if region.read(addr + index as u64, 1).ok().map(|value| value as u8)
                        != Some(expected[offset + index])
                    {
                        return false
                    }
                }
            } else {
                let start = phys as usize;
                let Some(end) = start.checked_add(chunk) else {
                    return false
                };
                if end > self.ram.len() || self.ram[start..end] != expected[offset..offset + chunk] {
                    return false
                }
            }
            offset += chunk;
        }
        true
    }

    pub fn write_bytes(&mut self, virt: u64, bytes: &[u8]) -> Result<(), MemoryError> {
        let len = bytes.len();
        let mut offset = 0usize;
        while offset < len {
            let addr = virt + offset as u64;
            let remaining = len - offset;
            let chunk = remaining.min(PAGE_SIZE - (addr as usize & (PAGE_SIZE - 1)));
            self.prepare_write(addr, chunk as u64)?;
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
            self.record_ram_write(phys, chunk);
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
            return self.ram_read(phys, 1).map(|value| value as u8);
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

    /// Fetch one instruction byte with execute permission checks.
    pub fn read_instruction_byte(&self, addr: u64) -> Result<u8, MemoryError> {
        let phys = self.physical_address(addr, AccessKind::Execute, 1)?;
        if let Some(region) = self.find_mmio(phys, 1) {
            return region.read(addr, 1).map(|value| value as u8).map_err(Into::into);
        }
        self.ram_read(phys, 1).map(|value| value as u8)
    }

    pub fn write_byte(&mut self, addr: u64, value: u8) -> Result<(), MemoryError> {
        self.prepare_write(addr, 1)?;
        if self.paging_enabled || !self.identity.is_empty() {
            let phys = self.physical_address(addr, AccessKind::Write, 1)?;
            if self.find_mmio(phys, 1).is_some() {
                return Err(MemoryError::AccessDenied);
            }
            if (phys as usize) < self.ram.len() {
                self.ram[phys as usize] = value;
                self.record_ram_write(phys, 1);
                return Ok(());
            }
            return Err(MemoryError::InvalidAddress);
        }
        if self.find_mmio(addr, 1).is_some() {
            return Err(MemoryError::AccessDenied);
        }
        if (addr as usize) < self.ram.len() {
            self.ram[addr as usize] = value;
            self.record_ram_write(addr, 1);
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
        self.allocator
            .alloc_excluding(&self.ballooned_frames)
            .ok_or(MemoryError::OutOfMemory)
    }

    /// Allocate a zeroed physical frame from the allocator.
    pub fn alloc_zeroed_frame(&mut self) -> Result<u64, MemoryError> {
        let frame = self.alloc_frame()?;
        let start = frame as usize;
        let end = start + PAGE_SIZE;
        if end > self.ram.len() {
            self.allocator.free(frame);
            return Err(MemoryError::InvalidAddress);
        }
        self.ram[start..end].fill(0);
        Ok(frame)
    }

    /// Free a physical frame back to the allocator.
    pub fn free_frame(&mut self, addr: u64) {
        if self.zero_page == Some(addr) {
            return;
        }
        self.ballooned_frames.remove(&addr);
        self.allocator.free(addr);
    }

    /// Direct physical (non-translated) write. Used by the loader to place
    /// the kernel image at a physical address regardless of paging state.
    pub fn write_phys(&mut self, phys: u64, bytes: &[u8]) -> Result<(), MemoryError> {
        self.validate_dma_range(phys, bytes.len(), 1)?;
        let start = usize::try_from(phys).map_err(|_| MemoryError::InvalidAddress)?;
        let end = start.checked_add(bytes.len()).ok_or(MemoryError::InvalidAddress)?;
        if end > self.ram.len() {
            return Err(MemoryError::InvalidAddress);
        }
        self.ram[start..end].copy_from_slice(bytes);
        self.record_ram_write(phys, bytes.len());
        if self.dma_capture {
            self.dma_writes.push(ReplayDmaWrite {
                address: phys,
                bytes: bytes.to_vec(),
            });
        }
        Ok(())
    }

    /// Direct physical (non-translated) read. Used by the BIOS and loader to
    /// inspect guest memory without a mapping.
    pub fn read_phys(&self, phys: u64, len: usize) -> Result<Vec<u8>, MemoryError> {
        self.validate_dma_range(phys, len, 1)?;
        let start = usize::try_from(phys).map_err(|_| MemoryError::InvalidAddress)?;
        let end = start.checked_add(len).ok_or(MemoryError::InvalidAddress)?;
        if end > self.ram.len() {
            return Err(MemoryError::InvalidAddress);
        }
        Ok(self.ram[start..end].to_vec())
    }

    /// Validate a physical DMA range before a device touches guest RAM.
    /// Device models use this same gate, so overflow, MMIO, ballooned pages,
    /// and out-of-RAM accesses fail identically.
    pub fn validate_dma_range(
        &self,
        phys: u64,
        len: usize,
        alignment: u64,
    ) -> Result<(), MemoryError> {
        if len == 0 || (alignment != 0 && phys % alignment != 0) {
            return Err(MemoryError::AlignmentError);
        }
        let last = phys
            .checked_add(len as u64 - 1)
            .ok_or(MemoryError::InvalidAddress)?;
        let end = last.checked_add(1).ok_or(MemoryError::InvalidAddress)?;
        let ram_end = self.ram.len() as u64;
        if end > ram_end {
            return Err(MemoryError::InvalidAddress);
        }
        let first_page = phys & !(PAGE_SIZE as u64 - 1);
        let last_page = last & !(PAGE_SIZE as u64 - 1);
        let mut page = first_page;
        loop {
            if self.ballooned_frames.contains(&page) || self.find_mmio(page, 1).is_some() {
                return Err(MemoryError::AccessDenied);
            }
            if page == last_page {
                break;
            }
            page = page
                .checked_add(PAGE_SIZE as u64)
                .ok_or(MemoryError::InvalidAddress)?;
        }
        Ok(())
    }

    /// Install an identity mapping for a physical range in the bootstrap
    /// identity map (used while paging is disabled or in early boot).
    pub fn identity_map_range(
        &mut self,
        start: u64,
        size: u64,
        flags: PageFlags,
    ) -> Result<(), MemoryError> {
        self.translation_version = self.translation_version.wrapping_add(1);
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
        if !self.identity.is_empty() {
            self.translation_version = self.translation_version.wrapping_add(1);
        }
        self.identity.clear();
    }

    /// Map one virtual page to one physical frame using guest-visible page
    /// tables stored in RAM. Returns the physical frame allocated for the
    /// page-table entries if new tables were needed.
    pub fn map_page(&mut self, virt: u64, phys: u64, flags: PageFlags) -> Result<(), MemoryError> {
        if virt & (PAGE_SIZE as u64 - 1) != 0 || phys & (PAGE_SIZE as u64 - 1) != 0 {
            return Err(MemoryError::AlignmentError);
        }
        if let Some(old_phys) = self.mapped_pages.get(&virt).copied() {
            self.untrack_mapping(virt, old_phys);
        }
        self.page_tables_map(virt, phys, flags)
            .map(|()| self.track_mapping(virt, phys))
    }

    /// Map a 2 MiB or 1 GiB hardware large page.
    pub fn map_large_page(
        &mut self,
        virt: u64,
        phys: u64,
        size: LargePageSize,
        flags: PageFlags,
    ) -> Result<(), MemoryError> {
        let page_size = size.bytes() as u64;
        if virt % page_size != 0 || phys % page_size != 0 {
            return Err(MemoryError::AlignmentError);
        }
        let end = phys
            .checked_add(page_size)
            .ok_or(MemoryError::InvalidAddress)?;
        if end > self.ram.len() as u64 {
            return Err(MemoryError::InvalidAddress);
        }
        self.page_tables_map_large(virt, phys, size, flags)
    }

    pub fn map_2mb_page(
        &mut self,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), MemoryError> {
        self.map_large_page(virt, phys, LargePageSize::TwoMiB, flags)
    }

    pub fn map_1gb_page(
        &mut self,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), MemoryError> {
        self.map_large_page(virt, phys, LargePageSize::OneGiB, flags)
    }

    /// Map a read-only demand-zero page. The page consumes no private frame
    /// until the guest writes to it, making sparse address spaces cheap.
    pub fn map_overcommit_page(&mut self, virt: u64, flags: PageFlags) -> Result<(), MemoryError> {
        if virt & (PAGE_SIZE as u64 - 1) != 0 {
            return Err(MemoryError::AlignmentError);
        }
        let replacing_overcommit = self
            .cow_pages
            .get(&virt)
            .map(|mapping| mapping.overcommitted)
            .unwrap_or(false);
        if self.overcommitted_pages >= self.overcommit_limit && !replacing_overcommit {
            return Err(MemoryError::OutOfMemory);
        }
        let zero_page = match self.zero_page {
            Some(page) => page,
            None => {
                let page = self.alloc_zeroed_frame()?;
                self.zero_page = Some(page);
                page
            }
        };
        if let Some(old_phys) = self.mapped_pages.get(&virt).copied() {
            self.untrack_mapping(virt, old_phys);
        }
        let cow_flags = (flags | PageFlags::PRESENT) & !PageFlags::WRITABLE;
        self.page_tables_map(virt, zero_page, cow_flags)?;
        self.track_mapping(virt, zero_page);
        self.cow_pages.insert(
            virt,
            CowMapping {
                phys: zero_page,
                flags: cow_flags,
                overcommitted: true,
            },
        );
        self.overcommitted_pages += 1;
        Ok(())
    }

    pub fn map_lazy_page(&mut self, virt: u64, flags: PageFlags) -> Result<(), MemoryError> {
        self.map_overcommit_page(virt, flags)
    }

    /// Share a 4 KiB mapping between two virtual addresses. The source and
    /// destination become read-only and the first write to either gets a
    /// private copy.
    pub fn clone_cow_page(&mut self, source: u64, destination: u64) -> Result<(), MemoryError> {
        if source & (PAGE_SIZE as u64 - 1) != 0 || destination & (PAGE_SIZE as u64 - 1) != 0 {
            return Err(MemoryError::AlignmentError);
        }
        let source_phys = self
            .mapped_pages
            .get(&source)
            .copied()
            .or_else(|| {
                walk_page_table(&self.ram, self.cr3, source)
                    .map(|(phys, _)| phys & !(PAGE_SIZE as u64 - 1))
            })
            .ok_or(MemoryError::PageFault)?;
        if source_phys as usize > self.ram.len().saturating_sub(PAGE_SIZE) {
            return Err(MemoryError::AccessDenied);
        }
        let source_overcommitted = self
            .cow_pages
            .get(&source)
            .map(|mapping| mapping.overcommitted)
            .unwrap_or(false);
        let source_flags = self
            .cow_pages
            .get(&source)
            .map(|mapping| mapping.flags)
            .or_else(|| walk_page_table(&self.ram, self.cr3, source).map(|(_, flags)| flags))
            .ok_or(MemoryError::PageFault)?;
        let source_was_tracked = self.mapped_pages.contains_key(&source);
        if let Some(old_phys) = self.mapped_pages.get(&source).copied() {
            self.update_page_mapping(source, old_phys, source_flags & !PageFlags::WRITABLE)?;
        }
        if let Some(old_phys) = self.mapped_pages.get(&destination).copied() {
            self.untrack_mapping(destination, old_phys);
        }
        let cow_flags = (source_flags | PageFlags::PRESENT) & !PageFlags::WRITABLE;
        self.page_tables_map(destination, source_phys, cow_flags)?;
        if !source_was_tracked {
            self.track_mapping(source, source_phys);
        }
        self.track_mapping(destination, source_phys);
        self.cow_pages.insert(
            source,
            CowMapping {
                phys: source_phys,
                flags: cow_flags,
                overcommitted: source_overcommitted,
            },
        );
        self.cow_pages.insert(
            destination,
            CowMapping {
                phys: source_phys,
                flags: cow_flags,
                overcommitted: source_overcommitted,
            },
        );
        if source_overcommitted {
            self.overcommitted_pages += 1;
        }
        Ok(())
    }

    pub fn map_cow_page(
        &mut self,
        virt: u64,
        phys: u64,
        flags: PageFlags,
    ) -> Result<(), MemoryError> {
        if virt & (PAGE_SIZE as u64 - 1) != 0 || phys & (PAGE_SIZE as u64 - 1) != 0 {
            return Err(MemoryError::AlignmentError);
        }
        if phys as usize > self.ram.len().saturating_sub(PAGE_SIZE) {
            return Err(MemoryError::InvalidAddress);
        }
        if let Some(old_phys) = self.mapped_pages.get(&virt).copied() {
            self.untrack_mapping(virt, old_phys);
        }
        let cow_flags = (flags | PageFlags::PRESENT) & !PageFlags::WRITABLE;
        self.page_tables_map(virt, phys, cow_flags)?;
        self.track_mapping(virt, phys);
        self.cow_pages.insert(
            virt,
            CowMapping {
                phys,
                flags: cow_flags,
                overcommitted: false,
            },
        );
        Ok(())
    }

    pub fn unmap_page(&mut self, virt: u64) -> Result<(), MemoryError> {
        if virt & (PAGE_SIZE as u64 - 1) != 0 {
            return Err(MemoryError::AlignmentError);
        }
        self.page_tables_unmap(virt)?;
        if let Some(old_phys) = self.mapped_pages.get(&virt).copied() {
            self.untrack_mapping(virt, old_phys);
        }
        Ok(())
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
        let pml4e = self.read_pte(pml4e_addr)?;
        let pdpt_addr = self.ensure_table(pml4e_addr, pml4e)?;

        let pdpte_addr = pdpt_addr + pdpt_idx * 8;
        let pdpte = self.read_pte(pdpte_addr)?;
        if pdpte.present() && pdpte.large() {
            return Err(MemoryError::AccessDenied);
        }
        let pd_addr = self.ensure_table(pdpte_addr, pdpte)?;

        let pde_addr = pd_addr + pd_idx * 8;
        let pde = self.read_pte(pde_addr)?;
        if pde.present() && pde.large() {
            return Err(MemoryError::AccessDenied);
        }
        let pt_addr = self.ensure_table(pde_addr, pde)?;

        let pte_addr = pt_addr + pt_idx * 8;
        // Force PRESENT so the mapping is usable, but respect the caller's
        // writable/user/NX bits for permission enforcement.
        self.write_pte(
            pte_addr,
            Pte {
                raw: phys | 0x001 | flags.bits(),
            },
        )
    }

    fn ensure_table(&mut self, entry_addr: u64, entry: Pte) -> Result<u64, MemoryError> {
        if entry.present() {
            return Ok(entry.phys());
        }
        let frame = self.alloc_zeroed_frame()?;
        self.write_pte(entry_addr, Pte { raw: frame | 0x007 })?;
        Ok(frame)
    }

    fn page_tables_map_large(
        &mut self,
        virt: u64,
        phys: u64,
        size: LargePageSize,
        flags: PageFlags,
    ) -> Result<(), MemoryError> {
        let mut cr3 = self.cr3;
        if cr3 == 0 {
            cr3 = self.alloc_zeroed_frame()?;
            self.cr3 = cr3;
        }
        let pml4e_addr = cr3 + ((virt >> 39) & 0x1FF) * 8;
        let pml4e = self.read_pte(pml4e_addr)?;
        let pdpt = self.ensure_table(pml4e_addr, pml4e)?;
        let pdpte_addr = pdpt + ((virt >> 30) & 0x1FF) * 8;

        match size {
            LargePageSize::OneGiB => self.write_pte(
                pdpte_addr,
                Pte {
                    raw: phys | 0x081 | flags.bits(),
                },
            ),
            LargePageSize::TwoMiB => {
                let pdpte = self.read_pte(pdpte_addr)?;
                if pdpte.present() && pdpte.large() {
                    return Err(MemoryError::AccessDenied);
                }
                let pd = self.ensure_table(pdpte_addr, pdpte)?;
                let pde_addr = pd + ((virt >> 21) & 0x1FF) * 8;
                self.write_pte(
                    pde_addr,
                    Pte {
                        raw: phys | 0x081 | flags.bits(),
                    },
                )
            }
        }
    }

    fn page_tables_unmap(&mut self, virt: u64) -> Result<(), MemoryError> {
        if self.cr3 == 0 {
            return Ok(());
        }
        let pml4_idx = ((virt >> 39) & 0x1FF) as usize;
        let pdpt_idx = ((virt >> 30) & 0x1FF) as usize;
        let pd_idx = ((virt >> 21) & 0x1FF) as usize;
        let pt_idx = ((virt >> 12) & 0x1FF) as usize;

        let pml4e = self.read_pte(self.cr3 + (pml4_idx as u64) * 8)?;
        if !pml4e.present() {
            return Ok(());
        }
        let pdpte_addr = pml4e.phys() + (pdpt_idx as u64) * 8;
        let pdpte = self.read_pte(pdpte_addr)?;
        if !pdpte.present() {
            return Ok(());
        }
        if pdpte.large() {
            self.write_pte(pdpte_addr, Pte { raw: 0 })?;
            return Ok(());
        }
        let pde_addr = pdpte.phys() + (pd_idx as u64) * 8;
        let pde = self.read_pte(pde_addr)?;
        if !pde.present() {
            return Ok(());
        }
        if pde.large() {
            self.write_pte(pde_addr, Pte { raw: 0 })?;
            return Ok(());
        }
        let pte_addr = pde.phys() + (pt_idx as u64) * 8;
        self.write_pte(pte_addr, Pte { raw: 0 })?;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.translation_version = self.translation_version.wrapping_add(1);
        self.allocator.reset();
        self.ram.fill(0);
        self.code_version = 0;
        self.translated_code_pages.clear();
        self.page_table_pages.clear();
        self.identity.clear();
        self.cow_pages.clear();
        self.mapped_frames.clear();
        self.mapped_pages.clear();
        self.ballooned_frames.clear();
        self.zero_page = None;
        self.overcommitted_pages = 0;
        self.paging_enabled = false;
        self.cr3 = 0;
        self.privilege = false;
        self.instruction_ip = None;
        self.dma_capture = false;
        self.dma_writes.clear();
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
        fn write(&mut self, _addr: u64, value: u64, _size: u8) -> Result<(), DeviceError> {
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
        assert_eq!(
            mmu.read_phys(0x1000, 4).unwrap(),
            vec![0xDE, 0xAD, 0xBE, 0xEF]
        );
    }

    #[test]
    fn identity_map_permissions() {
        let mut mmu = Mmu::new(1 << 20);
        mmu.identity_map_range(
            0,
            PAGE_SIZE as u64,
            PageFlags::PRESENT | PageFlags::WRITABLE,
        )
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
        mmu.map_page(0x0000_4000, 0x0000_3000, PageFlags::PRESENT)
            .unwrap();
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
