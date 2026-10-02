//! AHCI (Advanced Host Controller Interface) SATA HBA emulation.
//!
//! Single-port SATA HBA exposing the standard register file (ABAR 4 KiB),
//! command list + PRD DMA, H2D/D2H register FIS protocol, and IDENTIFY +
//! DMA read/write ATA commands backed by a [`DiskImage`].
//!
//! Commands are recorded when the guest issues them (CI write); the actual
//! DMA transfer is deferred to [`Ahci::poll_dma`], called by the machine
//! outside the CPU step so it can borrow guest physical memory safely.

use crate::devices::storage::{DiskImage, StorageError};
use super::native_io::{CIo, Context};
use crate::devices::{Device, DeviceError, LocalApic};
use crate::memory::Mmu;
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

pub const AHCI_ABAR_SIZE: u64 = 0x1000;
pub const AHCI_VENDOR_ID: u16 = 0x8086;
pub const AHCI_DEVICE_ID: u16 = 0x2922;
pub const AHCI_CLASS: u8 = 0x01;
pub const AHCI_SUBCLASS: u8 = 0x06;
pub const AHCI_PROG_IF: u8 = 0x01;

#[cfg(test)]
#[repr(C)]
struct CPrd { address: u64, length: usize }

unsafe extern "C" {
    fn ghostos_vm_ahci_new(checked: bool) -> *mut c_void;
    fn ghostos_vm_ahci_free(state: *mut c_void);
    fn ghostos_vm_ahci_reset(state: *mut c_void);
    fn ghostos_vm_ahci_pending(state: *const c_void) -> bool;
    fn ghostos_vm_ahci_read(state: *const c_void, address: u64, size: u8, value: *mut u64) -> u32;
    fn ghostos_vm_ahci_write(state: *mut c_void, address: u64, size: u8, value: u32) -> u32;
    fn ghostos_vm_ahci_poll(state: *mut c_void, io: *const CIo) -> u32;
    #[cfg(test)]
    fn ghostos_vm_ahci_identify(sectors: u64, output: *mut u8);
    #[cfg(test)]
    fn ghostos_vm_ahci_transfer(state: *mut c_void, io: *const CIo, lba: u64,
        sectors: usize, prds: *const CPrd, prd_count: usize, to_disk: bool) -> u32;
}

fn check_native(result: u32) {
    match result {
        0 => {}
        4 => panic!("AHCI sector slice out of bounds"),
        5 => panic!("attempt to add with overflow"),
        _ => panic!("unexpected AHCI native result"),
    }
}

/// Host resources wrap a C-owned single-port SATA controller.
pub struct Ahci {
    state: *mut c_void,
    disk: Option<DiskImage>,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

impl Ahci {
    pub fn new() -> Self {
        let state = unsafe { ghostos_vm_ahci_new(cfg!(debug_assertions)) };
        assert!(!state.is_null(), "AHCI native allocation failed");
        Self { state, disk: None, apic: None, irq_vector: 0 }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) { self.apic = Some(apic); }
    pub fn set_irq_vector(&mut self, vector: u8) { self.irq_vector = vector; }
    pub fn attach_disk(&mut self, disk: DiskImage) -> Option<DiskImage> { self.disk.replace(disk) }
    pub fn detach_disk(&mut self) -> Option<DiskImage> { self.disk.take() }

    pub fn flush_disk(&mut self) -> Result<(), StorageError> {
        self.disk.as_mut().ok_or_else(|| StorageError::InvalidImage("no disk".into()))?.flush()
    }
    pub fn sync_disk(&mut self) -> Result<(), StorageError> {
        self.disk.as_mut().ok_or_else(|| StorageError::InvalidImage("no disk".into()))?.sync()
    }
    pub fn sector_count(&self) -> Option<u64> { self.disk.as_ref().map(|disk| disk.sector_count()) }
    pub fn has_pending(&self) -> bool { unsafe { ghostos_vm_ahci_pending(self.state) } }

    /// C processes all issued commands before the borrowed context expires.
    pub fn poll_dma(&mut self, mmu: &mut Mmu) {
        let mut context = Context { mmu, disk: &mut self.disk, apic: &self.apic, vector: self.irq_vector };
        check_native(unsafe { ghostos_vm_ahci_poll(self.state, &context.io()) });
    }

    #[cfg(test)]
    fn build_identify(&self) -> [u8; 512] {
        let mut output = [0; 512];
        unsafe { ghostos_vm_ahci_identify(self.sector_count().unwrap_or(0), output.as_mut_ptr()) };
        output
    }

    #[cfg(test)]
    fn disk_to_prds(&mut self, mmu: &mut Mmu, lba: u64, count: usize,
        prds: &[(u64, usize)], to_disk: bool) -> Result<(), StorageError> {
        let prds: Vec<CPrd> = prds.iter().map(|&(address, length)| CPrd { address, length }).collect();
        let mut context = Context { mmu, disk: &mut self.disk, apic: &self.apic, vector: self.irq_vector };
        let result = unsafe { ghostos_vm_ahci_transfer(self.state, &context.io(), lba, count, prds.as_ptr(), prds.len(), to_disk) };
        match result {
            0 => Ok(()),
            2 => Err(StorageError::Dma("native PRD transfer".into())),
            3 => Err(StorageError::InvalidImage("native disk transfer".into())),
            _ => { check_native(result); unreachable!() }
        }
    }
}

impl Drop for Ahci {
    fn drop(&mut self) { unsafe { ghostos_vm_ahci_free(self.state) }; }
}
impl Default for Ahci {
    fn default() -> Self { Self::new() }
}
impl Device for Ahci {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        let mut value = 0;
        if unsafe { ghostos_vm_ahci_read(self.state, addr, size, &mut value) } == 0 {
            Ok(value)
        } else { Err(DeviceError::UnsupportedSize) }
    }
    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        if unsafe { ghostos_vm_ahci_write(self.state, addr, size, value as u32) } == 0 {
            Ok(())
        } else { Err(DeviceError::UnsupportedSize) }
    }
    fn reset(&mut self) { unsafe { ghostos_vm_ahci_reset(self.state) }; }
}
impl Device for Rc<RefCell<Ahci>> {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        Device::read(&*self.borrow(), addr, size)
    }
    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        Device::write(&mut *self.borrow_mut(), addr, value, size)
    }
    fn reset(&mut self) { self.borrow_mut().reset(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn image_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ghostos-ahci-{name}-{}", std::process::id()))
    }

    fn disk(name: &str) -> DiskImage {
        let path = image_path(name);
        let mut file = File::create(path.clone()).unwrap();
        file.set_len(4096).unwrap();
        file.flush().unwrap();
        DiskImage::open(path).unwrap()
    }

    #[test]
    fn identification_and_dma_round_trip() {
        let mut ahci = Ahci::new();
        ahci.attach_disk(disk("round-trip"));
        assert_eq!(ahci.sector_count(), Some(8));
        let identify = ahci.build_identify();
        assert_eq!(&identify[46..66], b"GhostOS Virtual Disk");

        let mut mmu = Mmu::new(0x20_000);
        let source = 0x1000;
        let target = 0x2000;
        let sector = [0xA5u8; 512];
        mmu.write_phys(source, &sector).unwrap();
        ahci.disk_to_prds(&mut mmu, 2, 1, &[(source, 512)], true).unwrap();
        ahci.disk_to_prds(&mut mmu, 2, 1, &[(target, 512)], false).unwrap();
        assert_eq!(mmu.read_phys(target, 512).unwrap(), sector);
    }

    #[test]
    fn register_access_rejects_bad_size_and_reset_keeps_disk() {
        let mut ahci = Ahci::new();
        ahci.attach_disk(disk("reset"));
        assert_eq!(Device::read(&ahci, 0, 2), Err(DeviceError::UnsupportedSize));
        assert_eq!(Device::read(&ahci, 0, 4).unwrap(), 0x8002_0301);
        ahci.reset();
        assert_eq!(ahci.sector_count(), Some(8));
        assert!(!ahci.has_pending());
    }
}
