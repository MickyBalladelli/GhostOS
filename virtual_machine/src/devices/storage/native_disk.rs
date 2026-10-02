//! Borrowed host-file adapters for the C image parser and sector engine.

use super::StorageError;
use std::ffi::{c_char, c_void, CStr};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

#[repr(C)]
pub(super) struct CIo {
    length: unsafe extern "C" fn(*mut c_void, *mut u64) -> bool,
    seek: unsafe extern "C" fn(*mut c_void, u32, u64, *mut u64) -> bool,
    read: unsafe extern "C" fn(*mut c_void, *mut u8, usize) -> bool,
    write: unsafe extern "C" fn(*mut c_void, *const u8, usize) -> bool,
    resize: unsafe extern "C" fn(*mut c_void, u64) -> bool,
    flush: unsafe extern "C" fn(*mut c_void) -> bool,
    sync: unsafe extern "C" fn(*mut c_void) -> bool,
    context: *mut c_void,
}
#[repr(C)]
#[derive(Default)]
pub(super) struct CError { code: u32, format: u32, value: u64, other: u64 }

unsafe extern "C" {
    pub(super) fn ghostos_vm_disk_image_open(io: *const CIo, writable: bool, checked: bool, error: *mut CError) -> *mut c_void;
    pub(super) fn ghostos_vm_disk_image_free(image: *mut c_void);
    pub(super) fn ghostos_vm_disk_image_format(image: *const c_void) -> u32;
    pub(super) fn ghostos_vm_disk_image_size(image: *const c_void) -> u64;
    pub(super) fn ghostos_vm_disk_image_sectors(image: *const c_void) -> u64;
    pub(super) fn ghostos_vm_disk_image_writable(image: *const c_void) -> bool;
    pub(super) fn ghostos_vm_disk_image_read(image: *mut c_void, io: *const CIo, lba: u64, out: *mut u8, error: *mut CError) -> bool;
    pub(super) fn ghostos_vm_disk_image_write(image: *mut c_void, io: *const CIo, lba: u64, bytes: *const u8, error: *mut CError) -> bool;
    pub(super) fn ghostos_vm_disk_image_flush(io: *const CIo, durable: bool, error: *mut CError) -> bool;
    fn ghostos_vm_disk_error_message(code: u32) -> *const c_char;
}

pub(super) struct Context<'a> { file: &'a mut File, error: Option<std::io::Error> }
impl<'a> Context<'a> {
    pub fn new(file: &'a mut File) -> Self { Self { file, error: None } }
    pub fn io(&mut self) -> CIo {
        CIo { length, seek, read, write, resize, flush, sync, context: (self as *mut Context<'_>).cast() }
    }
    fn result<T>(&mut self, result: std::io::Result<T>) -> Option<T> {
        match result { Ok(value) => Some(value), Err(error) => { self.error = Some(error); None } }
    }
    pub fn error(&mut self, error: CError) -> StorageError {
        match error.code {
            1 => return StorageError::Io(self.error.take().expect("native file callback retained its error")),
            2 => return StorageError::ReadOnly,
            3 => return StorageError::OutOfRange,
            4 => panic!("disk image native allocation failed"),
            5 => panic!("attempt to add with overflow"),
            _ => {}
        }
        let message = match error.code {
            12 => format!("VHD disk type {} (only fixed is supported)", error.value),
            14 => {
                let format = match error.format { 0 => "RAW", 1 => "VHD", 2 => "QCOW2", _ => unreachable!() };
                format!("{format} capacity {} is not a non-zero sector multiple", error.value)
            }
            15 => format!("VHD file is {} bytes, footer declares {} bytes of disk data", error.value, error.other),
            18 => format!("QCOW version {} (only 2 and 3 are supported)", error.value),
            21 => format!("invalid QCOW2 cluster_bits {}", error.value),
            22 => format!("QCOW2 incompatible features 0x{:x} (dirty/corrupt images unsupported)", error.value),
            code => unsafe { CStr::from_ptr(ghostos_vm_disk_error_message(code)) }.to_str().expect("C image errors are UTF-8").to_string(),
        };
        if matches!(error.code, 12 | 18 | 19 | 20 | 22 | 32) { StorageError::Unsupported(message) }
        else { StorageError::InvalidImage(message) }
    }
}

unsafe extern "C" fn length(raw: *mut c_void, out: *mut u64) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    let result = context.file.metadata();
    match context.result(result) { Some(metadata) => { unsafe { *out = metadata.len() }; true }, None => false }
}
unsafe extern "C" fn seek(raw: *mut c_void, origin: u32, offset: u64, out: *mut u64) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    let position = match origin { 0 => SeekFrom::Start(offset), 1 => SeekFrom::End(offset as i64),
        2 => SeekFrom::Current(offset as i64), _ => unreachable!() };
    let result = context.file.seek(position);
    match context.result(result) { Some(position) => { unsafe { *out = position }; true }, None => false }
}
unsafe extern "C" fn read(raw: *mut c_void, bytes: *mut u8, length: usize) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    let bytes = if length == 0 { &mut [] } else { unsafe { std::slice::from_raw_parts_mut(bytes, length) } };
    let result = context.file.read_exact(bytes); context.result(result).is_some()
}
unsafe extern "C" fn write(raw: *mut c_void, bytes: *const u8, length: usize) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    let bytes = if length == 0 { &[] } else { unsafe { std::slice::from_raw_parts(bytes, length) } };
    let result = context.file.write_all(bytes); context.result(result).is_some()
}
unsafe extern "C" fn resize(raw: *mut c_void, length: u64) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    let result = context.file.set_len(length); context.result(result).is_some()
}
unsafe extern "C" fn flush(raw: *mut c_void) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    let result = context.file.flush(); context.result(result).is_some()
}
unsafe extern "C" fn sync(raw: *mut c_void) -> bool {
    let context = unsafe { &mut *raw.cast::<Context<'_>>() };
    let result = context.file.sync_all(); context.result(result).is_some()
}
