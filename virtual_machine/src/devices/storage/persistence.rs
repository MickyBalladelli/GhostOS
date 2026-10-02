//! C-owned persistence port and SYNOPS01 disk-region encoding.

use super::{DiskImage, StorageError};
use crate::devices::{DeviceError, PortDevice};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

#[repr(C)]
struct CState {
    _private: [u8; 0],
}

#[repr(C)]
struct CIo {
    attached: bool,
    read_sector: unsafe extern "C" fn(*mut c_void, u64, *mut u8) -> bool,
    write_sector: unsafe extern "C" fn(*mut c_void, u64, *const u8) -> bool,
    sync: unsafe extern "C" fn(*mut c_void) -> bool,
    context: *mut c_void,
}

const _: () = {
    assert!(std::mem::size_of::<CIo>() == 40);
    assert!(std::mem::offset_of!(CIo, read_sector) == 8);
    assert!(std::mem::offset_of!(CIo, context) == 32);
};

unsafe extern "C" {
    fn ghostos_vm_persistence_new() -> *mut CState;
    fn ghostos_vm_persistence_free(state: *mut CState);
    fn ghostos_vm_persistence_attach(state: *mut CState, sectors: u64, io: *const CIo) -> u8;
    fn ghostos_vm_persistence_read(state: *mut CState, port: u16, size: u8, value: *mut u64) -> u8;
    fn ghostos_vm_persistence_write(state: *mut CState, port: u16, value: u64, size: u8,
        io: *const CIo) -> u8;
    fn ghostos_vm_persistence_reset(state: *mut CState, io: *const CIo);
}

struct DiskContext<'a> {
    image: Option<&'a mut DiskImage>,
    error: Option<StorageError>,
}

impl DiskContext<'_> {
    fn io(&mut self) -> CIo {
        CIo {
            attached: self.image.is_some(), read_sector, write_sector, sync: sync_image,
            context: (self as *mut Self).cast(),
        }
    }

    fn finish(&mut self, result: Result<(), StorageError>) -> bool {
        match result {
            Ok(()) => true,
            Err(error) => { self.error = Some(error); false }
        }
    }
}

// C invokes each callback synchronously with a live 512-byte sector and an
// exclusive disk context. No callback pointer or buffer is retained.
unsafe extern "C" fn read_sector(context: *mut c_void, sector: u64, bytes: *mut u8) -> bool {
    let context = unsafe { &mut *context.cast::<DiskContext<'_>>() };
    let Some(image) = context.image.as_mut() else { return false };
    let bytes = unsafe { &mut *bytes.cast::<[u8; 512]>() };
    let result = image.read_sector(sector, bytes);
    context.finish(result)
}

unsafe extern "C" fn write_sector(context: *mut c_void, sector: u64, bytes: *const u8) -> bool {
    let context = unsafe { &mut *context.cast::<DiskContext<'_>>() };
    let Some(image) = context.image.as_mut() else { return false };
    let bytes = unsafe { &*bytes.cast::<[u8; 512]>() };
    let result = image.write_sector(sector, bytes);
    context.finish(result)
}

unsafe extern "C" fn sync_image(context: *mut c_void) -> bool {
    let context = unsafe { &mut *context.cast::<DiskContext<'_>>() };
    let Some(image) = context.image.as_mut() else { return false };
    let result = image.sync();
    context.finish(result)
}

/// VM filesystem persistence stored in the tail of a disk image.
pub struct SynosPersistencePort {
    state: *mut CState,
    image: Option<DiskImage>,
}

impl SynosPersistencePort {
    pub fn new() -> Self {
        let state = unsafe { ghostos_vm_persistence_new() };
        assert!(!state.is_null(), "could not allocate persistence port");
        Self { state, image: None }
    }

    pub fn attach_image(&mut self, mut image: DiskImage) -> Result<(), StorageError> {
        let sectors = image.sector_count();
        {
            let mut context = DiskContext { image: Some(&mut image), error: None };
            let io = context.io();
            let code = unsafe { ghostos_vm_persistence_attach(self.state, sectors, &io) };
            match code {
                0 => {},
                1 => return Err(StorageError::InvalidImage(
                    "disk is too small for GhostOS persistence metadata".to_string(),
                )),
                _ => return Err(context.error.take().expect("C persistence I/O error")),
            }
        }
        self.image = Some(image);
        Ok(())
    }

    pub fn sync(&mut self) -> Result<(), StorageError> {
        if let Some(image) = self.image.as_mut() { image.sync() } else { Ok(()) }
    }

    pub fn has_image(&self) -> bool { self.image.is_some() }
}

impl Default for SynosPersistencePort {
    fn default() -> Self { Self::new() }
}

impl Drop for SynosPersistencePort {
    fn drop(&mut self) {
        unsafe { ghostos_vm_persistence_free(self.state) }
    }
}

fn port_result(code: u8) -> Result<(), DeviceError> {
    match code {
        0 => Ok(()),
        1 => Err(DeviceError::UnsupportedSize),
        2 => Err(DeviceError::InvalidAddress),
        3 => Err(DeviceError::NotReady),
        _ => panic!("persistence read cursor out of bounds"),
    }
}

impl PortDevice for SynosPersistencePort {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        let mut value = 0;
        port_result(unsafe { ghostos_vm_persistence_read(self.state, port, size, &mut value) })?;
        Ok(value)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        let mut context = DiskContext { image: self.image.as_mut(), error: None };
        let io = context.io();
        port_result(unsafe { ghostos_vm_persistence_write(self.state, port, value, size, &io) })
    }

    fn reset(&mut self) {
        let mut context = DiskContext { image: self.image.as_mut(), error: None };
        let io = context.io();
        unsafe { ghostos_vm_persistence_reset(self.state, &io) }
    }
}

impl PortDevice for Rc<RefCell<SynosPersistencePort>> {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        self.borrow_mut().read(port, size)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        self.borrow_mut().write(port, value, size)
    }

    fn reset(&mut self) {
        self.borrow_mut().reset();
    }
}
