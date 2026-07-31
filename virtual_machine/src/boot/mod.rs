use crate::memory::Mmu;
use std::path::Path;

pub struct Loader {
    pub kernel: Option<Vec<u8>>,
    pub initrd: Option<Vec<u8>>,
    pub entry_point: u64,
    pub kernel_size: usize,
    pub initrd_size: usize,
}

impl Loader {
    pub fn new() -> Self {
        Self {
            kernel: None,
            initrd: None,
            entry_point: 0,
            kernel_size: 0,
            initrd_size: 0,
        }
    }

    pub fn load_kernel<P: AsRef<Path>>(&mut self, path: P) -> Result<(), LoaderError> {
        let path = path.as_ref();
        self.kernel = Some(std::fs::read(path).map_err(|_| LoaderError::FileNotFound)?);
        self.kernel_size = self.kernel.as_ref().map(|k| k.len()).unwrap_or(0);
        Ok(())
    }

    pub fn load_initrd<P: AsRef<Path>>(&mut self, path: P) -> Result<(), LoaderError> {
        let path = path.as_ref();
        self.initrd = Some(std::fs::read(path).map_err(|_| LoaderError::FileNotFound)?);
        self.initrd_size = self.initrd.as_ref().map(|i| i.len()).unwrap_or(0);
        Ok(())
    }

    pub fn load_to_memory(&self, mmu: &mut Mmu, load_addr: u64) -> Result<(), LoaderError> {
        if let Some(ref kernel) = self.kernel {
            mmu.write_bytes(load_addr, kernel)
                .map_err(|_| LoaderError::LoadFailed)?;
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
}

impl Default for Loader {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum LoaderError {
    FileNotFound,
    InvalidFormat,
    LoadFailed,
    OutOfMemory,
    NotSupported,
}

pub fn multiboot_header(kernel: &[u8]) -> Option<u32> {
    for i in 0..kernel.len().saturating_sub(4) {
        let addr = i as u32;
        if addr + 3 < kernel.len() as u32 {
            let header = u32::from_le_bytes([
                kernel[i], kernel[i+1], kernel[i+2], kernel[i+3]
            ]);
            if header == 0x1BADB002 {
                return Some(addr);
            }
        }
    }
    None
}

pub fn parse_multiboot(header: &[u8]) -> Option<MultibootInfo> {
    None
}

pub struct MultibootInfo {
    pub flags: u32,
    pub mem_lower: u32,
    pub mem_upper: u32,
    pub boot_device: u32,
    pub cmdline: u32,
    pub mods_count: u32,
    pub mods_addr: u32,
}