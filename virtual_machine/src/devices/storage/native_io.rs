//! Synchronous memory, disk, and interrupt adapters for C storage controllers.

use super::DiskImage;
use crate::devices::{ApicTrigger, LocalApic};
use crate::memory::Mmu;
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

#[repr(C)]
pub(super) struct CIo {
    read_memory: unsafe extern "C" fn(*mut c_void, u64, *mut u8, usize) -> bool,
    write_memory: unsafe extern "C" fn(*mut c_void, u64, *const u8, usize) -> bool,
    validate_dma: unsafe extern "C" fn(*mut c_void, u64, usize, u64) -> bool,
    sector_count: unsafe extern "C" fn(*mut c_void, *mut u64) -> bool,
    read_sector: unsafe extern "C" fn(*mut c_void, u64, *mut u8) -> bool,
    write_sector: unsafe extern "C" fn(*mut c_void, u64, *const u8) -> bool,
    flush_disk: unsafe extern "C" fn(*mut c_void) -> bool,
    interrupt: unsafe extern "C" fn(*mut c_void),
    context: *mut c_void,
}

pub(super) struct Context<'a> {
    pub mmu: &'a mut Mmu,
    pub disk: &'a mut Option<DiskImage>,
    pub apic: &'a Option<Rc<RefCell<LocalApic>>>,
    pub vector: u8,
}

impl Context<'_> {
    pub fn io(&mut self) -> CIo {
        CIo { read_memory, write_memory, validate_dma, sector_count, read_sector,
            write_sector, flush_disk, interrupt, context: (self as *mut Context<'_>).cast() }
    }
}

unsafe extern "C" fn read_memory(raw: *mut c_void, address: u64, out: *mut u8, length: usize) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    match context.mmu.read_phys(address, length) {
        Ok(bytes) => {
            if length != 0 { unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, length) }; }
            true
        }
        Err(_) => false,
    }
}

unsafe extern "C" fn write_memory(raw: *mut c_void, address: u64, bytes: *const u8, length: usize) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    let bytes = if length == 0 { &[] } else { unsafe { std::slice::from_raw_parts(bytes, length) } };
    context.mmu.write_phys(address, bytes).is_ok()
}

unsafe extern "C" fn validate_dma(raw: *mut c_void, address: u64, length: usize, alignment: u64) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    context.mmu.validate_dma_range(address, length, alignment).is_ok()
}

unsafe extern "C" fn sector_count(raw: *mut c_void, out: *mut u64) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    let Some(disk) = context.disk.as_ref() else { return false };
    unsafe { *out = disk.sector_count() };
    true
}

unsafe extern "C" fn read_sector(raw: *mut c_void, sector: u64, out: *mut u8) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    let Some(disk) = context.disk.as_mut() else { return false };
    disk.read_sector(sector, unsafe { &mut *out.cast::<[u8; 512]>() }).is_ok()
}

unsafe extern "C" fn write_sector(raw: *mut c_void, sector: u64, bytes: *const u8) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    let Some(disk) = context.disk.as_mut() else { return false };
    disk.write_sector(sector, unsafe { &*bytes.cast::<[u8; 512]>() }).is_ok()
}

unsafe extern "C" fn flush_disk(raw: *mut c_void) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    context.disk.as_mut().is_some_and(|disk| disk.flush().is_ok())
}

unsafe extern "C" fn interrupt(raw: *mut c_void) {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    if context.vector != 0 {
        if let Some(apic) = context.apic {
            apic.borrow_mut().signal(context.vector, ApicTrigger::Edge);
        }
    }
}
