use std::collections::HashMap;

pub struct Mmu {
    physical_memory: Vec<u8>,
    page_tables: PageTableManager,
    mappings: HashMap<u64, u64>,
    page_size: usize,
}

impl Mmu {
    pub fn new(size: usize) -> Self {
        let mut physical_memory = Vec::with_capacity(size);
        physical_memory.resize(size, 0);
        
        Self {
            physical_memory,
            page_tables: PageTableManager::new(),
            mappings: HashMap::new(),
            page_size: 4096,
        }
    }

    pub fn read_byte(&self, addr: u64) -> Result<u8, MemoryError> {
        if (addr as usize) < self.physical_memory.len() {
            Ok(self.physical_memory[addr as usize])
        } else {
            Err(MemoryError::InvalidAddress)
        }
    }

    pub fn write_byte(&mut self, addr: u64, value: u8) -> Result<(), MemoryError> {
        if (addr as usize) < self.physical_memory.len() {
            self.physical_memory[addr as usize] = value;
            Ok(())
        } else {
            Err(MemoryError::InvalidAddress)
        }
    }

    pub fn read_u64(&self, addr: u64) -> Result<u64, MemoryError> {
        let bytes = self.read_bytes(addr, 8)?;
        Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
    }

    pub fn write_u64(&mut self, addr: u64, value: u64) -> Result<(), MemoryError> {
        let bytes = value.to_le_bytes();
        self.write_bytes(addr, &bytes)
    }

    pub fn read_bytes(&self, addr: u64, len: usize) -> Result<Vec<u8>, MemoryError> {
        let end = addr.checked_add(len as u64).ok_or(MemoryError::InvalidAddress)?;
        if (end as usize) <= self.physical_memory.len() {
            Ok(self.physical_memory[addr as usize..end as usize].to_vec())
        } else {
            Err(MemoryError::InvalidAddress)
        }
    }

    pub fn write_bytes(&mut self, addr: u64, bytes: &[u8]) -> Result<(), MemoryError> {
        let len = bytes.len();
        let end = addr.checked_add(len as u64).ok_or(MemoryError::InvalidAddress)?;
        if (end as usize) <= self.physical_memory.len() {
            self.physical_memory[addr as usize..end as usize].copy_from_slice(bytes);
            Ok(())
        } else {
            Err(MemoryError::InvalidAddress)
        }
    }

    pub fn map_page(&mut self, virt: u64, phys: u64, flags: PageFlags) -> Result<(), MemoryError> {
        self.page_tables.map(virt, phys, flags);
        Ok(())
    }

    pub fn unmap_page(&mut self, virt: u64) -> Result<(), MemoryError> {
        self.page_tables.unmap(virt);
        Ok(())
    }

    pub fn reset(&mut self) {
        self.page_tables.reset();
        self.mappings.clear();
    }
}

impl Default for Mmu {
    fn default() -> Self {
        Self::new(128 * 1024 * 1024)
    }
}

#[derive(Debug)]
pub enum MemoryError {
    InvalidAddress,
    PageFault,
    AccessDenied,
    AlignmentError,
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

pub struct PageTableManager {
    pml4: [u64; 512],
    pdpt: [u64; 512],
    pd: [u64; 512],
    pt: [u64; 512],
}

impl PageTableManager {
    pub fn new() -> Self {
        Self {
            pml4: [0; 512],
            pdpt: [0; 512],
            pd: [0; 512],
            pt: [0; 512],
        }
    }

    pub fn map(&mut self, virt: u64, phys: u64, flags: PageFlags) {
        let pml4_idx = ((virt >> 39) & 0x1FF) as usize;
        let pdpt_idx = ((virt >> 30) & 0x1FF) as usize;
        let pd_idx = ((virt >> 21) & 0x1FF) as usize;
        let pt_idx = ((virt >> 12) & 0x1FF) as usize;

        if self.pml4[pml4_idx] == 0 {
            self.pml4[pml4_idx] = (self.pdpt.as_ptr() as u64) | 0xC00 | flags.bits();
        }

        if self.pdpt[pdpt_idx] == 0 {
            self.pdpt[pdpt_idx] = (self.pd.as_ptr() as u64) | 0xC00 | flags.bits();
        }

        if self.pd[pd_idx] == 0 {
            self.pd[pd_idx] = (self.pt.as_ptr() as u64) | 0xC00 | flags.bits();
        }

        self.pt[pt_idx] = phys | 0x800 | flags.bits();
    }

    pub fn unmap(&mut self, _virt: u64) {
    }

    pub fn reset(&mut self) {
        self.pml4 = [0; 512];
        self.pdpt = [0; 512];
        self.pd = [0; 512];
        self.pt = [0; 512];
    }
}