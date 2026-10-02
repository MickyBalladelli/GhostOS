//! NVMe (Non-Volatile Memory Express) controller emulation.
//!
//! Single-namespace NVMe PCIe device exposing the standard controller
//! registers (BAR0, 8 KiB), the Admin SQ/CQ pair, I/O SQ/CQ pairs, and the
//! core admin commands (IDENTIFY, CREATE/DELETE I/O SQ/CQ, GET/SET
//! FEATURES) plus NVM read/write/flush with PRP DMA.
//!
//! Commands are recorded when the guest rings a submission-queue doorbell;
//! the DMA transfer is deferred to [`Nvme::poll_dma`], called from the
//! machine loop outside the CPU step so guest physical memory can be
//! borrowed without aliasing.

use crate::devices::storage::{DiskImage, StorageError};
use super::native_io::{CIo, Context};
use crate::devices::{Device, DeviceError, LocalApic};
use crate::memory::Mmu;
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

pub const NVME_BAR0_SIZE: u64 = 0x2000;

/// Intel PCH-style NVMe controller PCI IDs.
pub const NVME_VENDOR_ID: u16 = 0x8086;
pub const NVME_DEVICE_ID: u16 = 0x5845;
pub const NVME_CLASS: u8 = 0x01;
pub const NVME_SUBCLASS: u8 = 0x08;
pub const NVME_PROG_IF: u8 = 0x02;

#[cfg(test)]
const REG_CAP: u64 = 0;
#[cfg(test)]
const REG_VS: u64 = 8;
#[cfg(test)]
const CAP_CSS_NVM: u64 = 1 << 37;
#[cfg(test)]
const ADMIN_IDENTIFY: u8 = 6;
#[cfg(test)]
const NVM_WRITE: u8 = 1;
#[cfg(test)]
const NVM_READ: u8 = 2;
#[cfg(test)]
const STS_SUCCESS: u32 = 0;
#[cfg(test)]
const STS_INVALID_NS: u32 = 11;
#[cfg(test)]
const STS_LBA_OUT_OF_RANGE: u32 = 128;

unsafe extern "C" {
    fn ghostos_vm_nvme_new(checked: bool) -> *mut c_void;
    fn ghostos_vm_nvme_free(state: *mut c_void);
    fn ghostos_vm_nvme_reset(state: *mut c_void);
    fn ghostos_vm_nvme_pending(state: *const c_void) -> bool;
    fn ghostos_vm_nvme_read(state: *const c_void, address: u64, size: u8, value: *mut u64) -> u32;
    fn ghostos_vm_nvme_write(state: *mut c_void, address: u64, size: u8, value: u64) -> u32;
    fn ghostos_vm_nvme_poll(state: *mut c_void, io: *const CIo) -> u32;
    #[cfg(test)]
    fn ghostos_vm_nvme_admin(state: *mut c_void, io: *const CIo, command: *const u8, status: *mut u32) -> u32;
    #[cfg(test)]
    fn ghostos_vm_nvme_command(state: *mut c_void, io: *const CIo, command: *const u8, status: *mut u32) -> u32;
}

fn check_native(result: u32) {
    match result {
        0 => {}
        3 => panic!("attempt to add with overflow"),
        _ => panic!("NVMe native allocation failed"),
    }
}

/// Host resources wrap a C-owned NVMe controller.
pub struct Nvme {
    state: *mut c_void,
    ns1: Option<DiskImage>,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

impl Nvme {
    pub fn new() -> Self {
        let state = unsafe { ghostos_vm_nvme_new(cfg!(debug_assertions)) };
        assert!(!state.is_null(), "NVMe native allocation failed");
        Self { state, ns1: None, apic: None, irq_vector: 0 }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic);
    }

    pub fn set_irq_vector(&mut self, vector: u8) {
        self.irq_vector = vector;
    }

    /// Attach namespace 1, returning the previous image.
    pub fn attach_namespace(&mut self, image: DiskImage) -> Option<DiskImage> {
        self.ns1.replace(image)
    }

    pub fn detach_namespace(&mut self) -> Option<DiskImage> {
        self.ns1.take()
    }

    pub fn flush_namespace(&mut self) -> Result<(), StorageError> {
        self.ns1.as_mut().ok_or_else(|| StorageError::InvalidImage("no namespace".into()))?.flush()
    }

    pub fn sync_namespace(&mut self) -> Result<(), StorageError> {
        self.ns1.as_mut().ok_or_else(|| StorageError::InvalidImage("no namespace".into()))?.sync()
    }

    pub fn has_pending(&self) -> bool {
        unsafe { ghostos_vm_nvme_pending(self.state) }
    }

    /// Poll synchronously; C never retains the borrowed host resources.
    pub fn poll_dma(&mut self, mmu: &mut Mmu) {
        let mut context = Context { mmu, disk: &mut self.ns1, apic: &self.apic, vector: self.irq_vector };
        check_native(unsafe { ghostos_vm_nvme_poll(self.state, &context.io()) });
    }

    #[cfg(test)]
    fn handle_admin(&mut self, mmu: &mut Mmu, command: &[u8; 64]) -> u32 {
        let mut context = Context { mmu, disk: &mut self.ns1, apic: &self.apic, vector: self.irq_vector };
        let mut status = 0;
        check_native(unsafe { ghostos_vm_nvme_admin(self.state, &context.io(), command.as_ptr(), &mut status) });
        status
    }

    #[cfg(test)]
    fn handle_io(&mut self, mmu: &mut Mmu, command: &[u8; 64]) -> u32 {
        let mut context = Context { mmu, disk: &mut self.ns1, apic: &self.apic, vector: self.irq_vector };
        let mut status = 0;
        check_native(unsafe { ghostos_vm_nvme_command(self.state, &context.io(), command.as_ptr(), &mut status) });
        status
    }
}

impl Drop for Nvme {
    fn drop(&mut self) { unsafe { ghostos_vm_nvme_free(self.state) }; }
}

impl Default for Nvme {
    fn default() -> Self { Self::new() }
}

impl Device for Nvme {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        let mut value = 0;
        if unsafe { ghostos_vm_nvme_read(self.state, addr, size, &mut value) } == 0 {
            Ok(value)
        } else { Err(DeviceError::UnsupportedSize) }
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        if unsafe { ghostos_vm_nvme_write(self.state, addr, size, value) } == 0 {
            Ok(())
        } else { Err(DeviceError::UnsupportedSize) }
    }

    fn reset(&mut self) { unsafe { ghostos_vm_nvme_reset(self.state) }; }
}

impl Device for Rc<RefCell<Nvme>> {
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

    fn disk(name: &str) -> DiskImage {
        let path = std::env::temp_dir().join(format!("ghostos-nvme-{name}-{}", std::process::id()));
        let mut file = File::create(path.clone()).unwrap();
        file.set_len(4096).unwrap();
        file.flush().unwrap();
        DiskImage::open(path).unwrap()
    }

    fn command(opcode: u8, buffer: u64, sector: u64, sectors: u16) -> [u8; 64] {
        let mut cmd = [0u8; 64];
        cmd[0] = opcode;
        cmd[4..8].copy_from_slice(&1u32.to_le_bytes());
        cmd[8..16].copy_from_slice(&buffer.to_le_bytes());
        cmd[40..44].copy_from_slice(&(sector as u32).to_le_bytes());
        cmd[44..48].copy_from_slice(&(((u32::from(sectors) - 1) << 16) | (sector >> 32) as u32).to_le_bytes());
        cmd
    }

    #[test]
    fn controller_registers_identify_and_io_round_trip() {
        let mut nvme = Nvme::new();
        nvme.attach_namespace(disk("round-trip"));
        assert_eq!(Device::read(&nvme, REG_VS, 4).unwrap(), 0x0001_0300);
        assert!(Device::read(&nvme, REG_CAP, 8).unwrap() & CAP_CSS_NVM != 0);

        let mut mmu = Mmu::new(0x20_000);
        let buffer = 0x2000;
        let payload = [0x5Au8; 512];
        mmu.write_phys(buffer, &payload).unwrap();
        assert_eq!(nvme.handle_io(&mut mmu, &command(NVM_WRITE, buffer, 1, 1)), STS_SUCCESS);
        mmu.write_phys(buffer, &[0; 512]).unwrap();
        assert_eq!(nvme.handle_io(&mut mmu, &command(NVM_READ, buffer, 1, 1)), STS_SUCCESS);
        assert_eq!(mmu.read_phys(buffer, 512).unwrap(), payload);

        let identify_buffer = 0x3000;
        let mut identify = command(ADMIN_IDENTIFY, identify_buffer, 0, 1);
        identify[40..44].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(nvme.handle_admin(&mut mmu, &identify), STS_SUCCESS);
        assert_eq!(u64::from_le_bytes(mmu.read_phys(identify_buffer, 8).unwrap().try_into().unwrap()), 8);
    }

    #[test]
    fn invalid_namespace_and_bounds_fail_cleanly() {
        let mut nvme = Nvme::new();
        let mut mmu = Mmu::new(0x10_000);
        let mut cmd = command(NVM_READ, 0x2000, 0, 1);
        cmd[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(nvme.handle_io(&mut mmu, &cmd), STS_INVALID_NS);
        nvme.attach_namespace(disk("bounds"));
        assert_eq!(nvme.handle_io(&mut mmu, &command(NVM_READ, 0x2000, 8, 1)), STS_LBA_OUT_OF_RANGE);
        assert_eq!(Device::read(&nvme, REG_VS, 2), Err(DeviceError::UnsupportedSize));
        nvme.reset();
        assert_eq!(nvme.ns1.as_ref().map(|image| image.sector_count()), Some(8));
    }
}
