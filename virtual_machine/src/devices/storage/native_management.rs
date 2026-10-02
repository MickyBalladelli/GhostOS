//! Borrowed host-path and owned Rust-object adapters for C disk management.

use super::{DiskController, DiskFormat, DiskInfo, DiskPersistence, DiskRole, DiskSpec, StorageError};
use std::ffi::{c_char, c_void, CStr};
use std::path::PathBuf;

#[repr(C)]
struct CSpec {
    id: *const u8,
    id_length: usize,
    controller: u32,
    role: u32,
    persistence: u32,
    bus: u8,
    slot: u8,
    read_only: bool,
    empty_id: bool,
}

#[repr(C)]
struct CPaths {
    resolve: unsafe extern "C" fn(*mut c_void, usize) -> bool,
    same_path: unsafe extern "C" fn(*mut c_void, usize, usize) -> bool,
    context: *mut c_void,
}

#[repr(C)]
struct CCloneIo {
    reserve: unsafe extern "C" fn(*mut c_void, u32) -> u32,
    copy: unsafe extern "C" fn(*mut c_void) -> bool,
    discard: unsafe extern "C" fn(*mut c_void),
    context: *mut c_void,
}

unsafe extern "C" {
    fn ghostos_vm_disk_clone_temporary(io: *const CCloneIo) -> u32;
    fn ghostos_vm_disk_validate_specs(specs: *const CSpec, count: usize,
        paths: *const CPaths, index: *mut usize) -> u32;
    fn ghostos_vm_disk_validate_image(format: u32, capacity: u64,
        has_format: bool, expected_format: u32, has_capacity: bool, expected_capacity: u64) -> u32;
    fn ghostos_vm_disk_needs_clone(read_only: bool, persistence: u32) -> bool;
    fn ghostos_vm_disk_controller_name(controller: u32) -> *const c_char;
    fn ghostos_vm_disk_guest_id(controller: u32, bus: u8, slot: u8,
        output: *mut u8, capacity: usize) -> usize;
    fn ghostos_vm_disk_manager_new() -> *mut c_void;
    fn ghostos_vm_disk_manager_free(manager: *mut c_void, destroy: unsafe extern "C" fn(*mut c_void));
    fn ghostos_vm_disk_manager_length(manager: *const c_void) -> usize;
    fn ghostos_vm_disk_manager_at(manager: *const c_void, index: usize) -> *mut c_void;
    fn ghostos_vm_disk_manager_get(manager: *const c_void, id: *const u8, length: usize) -> *mut c_void;
    fn ghostos_vm_disk_manager_insert(manager: *mut c_void, id: *const u8, length: usize, payload: *mut c_void) -> bool;
    fn ghostos_vm_disk_manager_remove(manager: *mut c_void, id: *const u8, length: usize) -> *mut c_void;
}

fn controller_code(controller: DiskController) -> u32 {
    match controller { DiskController::Ahci => 0, DiskController::Nvme => 1, DiskController::VirtioBlk => 2 }
}
fn persistence_code(persistence: DiskPersistence) -> u32 {
    match persistence { DiskPersistence::Persistent => 0, DiskPersistence::CopyOnWrite => 1, DiskPersistence::Disposable => 2 }
}
fn format_code(format: DiskFormat) -> u32 {
    match format { DiskFormat::Raw => 0, DiskFormat::Vhd => 1, DiskFormat::Qcow2 => 2 }
}

pub(super) fn controller_name(controller: DiskController) -> &'static str {
    // C returns one immutable static ASCII string for every supported enum.
    unsafe { CStr::from_ptr(ghostos_vm_disk_controller_name(controller_code(controller))) }
        .to_str().expect("C controller name is UTF-8")
}

pub(super) fn guest_id(spec: &DiskSpec) -> String {
    let mut output = [0; 32];
    let length = unsafe { ghostos_vm_disk_guest_id(controller_code(spec.controller),
        spec.bus, spec.slot, output.as_mut_ptr(), output.len()) };
    assert!(length != 0, "C disk guest ID fits its output buffer");
    String::from_utf8(output[..length].to_vec()).expect("C disk guest ID is ASCII")
}

pub(super) fn needs_clone(spec: &DiskSpec) -> bool {
    unsafe { ghostos_vm_disk_needs_clone(spec.read_only, persistence_code(spec.persistence)) }
}

pub(super) fn validate_image(spec: &DiskSpec, format: DiskFormat, capacity: u64) -> Result<(), StorageError> {
    let code = unsafe { ghostos_vm_disk_validate_image(format_code(format), capacity,
        spec.format.is_some(), spec.format.map(format_code).unwrap_or(0),
        spec.capacity.is_some(), spec.capacity.unwrap_or(0)) };
    let message = match code {
        0 => return Ok(()),
        1 => format!("disk `{}` has format {:?}, expected {:?}", spec.id, format, spec.format.unwrap()),
        2 => format!("disk `{}` capacity {} is not a non-zero sector multiple", spec.id, capacity),
        3 => format!("disk `{}` capacity is {}, expected {}", spec.id, capacity, spec.capacity.unwrap()),
        _ => unreachable!("unknown C disk validation result"),
    };
    Err(StorageError::InvalidImage(message))
}

struct PathContext<'a> {
    specs: &'a [DiskSpec],
    paths: Vec<Option<PathBuf>>,
    error: Option<StorageError>,
}

unsafe extern "C" fn resolve(raw: *mut c_void, index: usize) -> bool {
    let context = unsafe { &mut *raw.cast::<PathContext<'_>>() };
    let path = &context.specs[index].image_path;
    match std::fs::canonicalize(path) {
        Ok(path) => { context.paths[index] = Some(path); true },
        Err(error) => {
            context.error = Some(StorageError::InvalidImage(format!(
                "cannot resolve disk `{}`: {error}", path.display())));
            false
        }
    }
}

unsafe extern "C" fn same_path(raw: *mut c_void, left: usize, right: usize) -> bool {
    let context = unsafe { &*raw.cast::<PathContext<'_>>() };
    context.paths[left].as_ref().expect("earlier path resolved") ==
        context.paths[right].as_ref().expect("current path resolved")
}

pub(super) fn validate_specs(specs: &[DiskSpec]) -> Result<(), StorageError> {
    let native: Vec<_> = specs.iter().map(|spec| CSpec {
        id: spec.id.as_ptr(), id_length: spec.id.len(), controller: controller_code(spec.controller),
        role: if spec.role == DiskRole::System { 0 } else { 1 },
        persistence: persistence_code(spec.persistence), bus: spec.bus, slot: spec.slot,
        read_only: spec.read_only,
        // Keep Rust's exact Unicode whitespace definition at the string boundary.
        empty_id: spec.id.trim().is_empty(),
    }).collect();
    let mut context = PathContext { specs, paths: vec![None; specs.len()], error: None };
    let paths = CPaths { resolve, same_path, context: (&mut context as *mut PathContext<'_>).cast() };
    let mut index = 0;
    // C borrows all slices and invokes host callbacks only during this call.
    let code = unsafe { ghostos_vm_disk_validate_specs(native.as_ptr(), native.len(), &paths, &mut index) };
    if code == 0 { return Ok(()); }
    let spec = &specs[index];
    let message = match code {
        1 => "disk ID must not be empty".to_string(),
        2 => format!("duplicate disk ID `{}`", spec.id),
        3 => "only one system disk may be attached".to_string(),
        4 => return Err(StorageError::Unsupported(format!("{} supports only bus 0, slot 0", controller_name(spec.controller)))),
        5 => format!("duplicate {} location bus {}, slot {}", controller_name(spec.controller), spec.bus, spec.slot),
        6 => return Err(context.error.take().expect("path callback retains its error")),
        7 => format!("backing image `{}` is assigned to multiple writable disks", spec.image_path.display()),
        _ => unreachable!("unknown C disk specification result"),
    };
    Err(StorageError::InvalidImage(message))
}

pub(super) struct NativeManager { raw: *mut c_void }

// C owns only immutable DiskInfo boxes and bookkeeping. Mutation requires
// &mut self; shared queries do not mutate the C manager or its payloads.
unsafe impl Send for NativeManager {}
unsafe impl Sync for NativeManager {}

impl NativeManager {
    pub fn new() -> Self {
        let raw = unsafe { ghostos_vm_disk_manager_new() };
        if raw.is_null() { std::alloc::handle_alloc_error(std::alloc::Layout::new::<DiskInfo>()); }
        Self { raw }
    }
    pub fn list(&self) -> Vec<DiskInfo> {
        let length = unsafe { ghostos_vm_disk_manager_length(self.raw) };
        (0..length).map(|index| unsafe { &*ghostos_vm_disk_manager_at(self.raw, index).cast::<DiskInfo>() }.clone()).collect()
    }
    pub fn get(&self, id: &str) -> Option<&DiskInfo> {
        // The box remains alive until mutation or manager drop; the returned
        // reference is tied to this shared borrow and prevents either operation.
        unsafe { ghostos_vm_disk_manager_get(self.raw, id.as_ptr(), id.len()).cast::<DiskInfo>().as_ref() }
    }
    pub fn insert(&mut self, info: DiskInfo) {
        let info = Box::new(info);
        let id = info.id.as_ptr();
        let length = info.id.len();
        let raw = Box::into_raw(info);
        if !unsafe { ghostos_vm_disk_manager_insert(self.raw, id, length, raw.cast()) } {
            unsafe { drop(Box::from_raw(raw)); }
            std::alloc::handle_alloc_error(std::alloc::Layout::new::<DiskInfo>());
        }
    }
    pub fn remove(&mut self, id: &str) -> Option<DiskInfo> {
        let raw = unsafe { ghostos_vm_disk_manager_remove(self.raw, id.as_ptr(), id.len()).cast::<DiskInfo>() };
        if raw.is_null() { None } else { Some(*unsafe { Box::from_raw(raw) }) }
    }
}

unsafe extern "C" fn destroy(raw: *mut c_void) {
    unsafe { drop(Box::from_raw(raw.cast::<DiskInfo>())); }
}

impl Drop for NativeManager {
    fn drop(&mut self) { unsafe { ghostos_vm_disk_manager_free(self.raw, destroy); } }
}

struct CloneContext<'a> {
    source: &'a std::path::Path,
    extension: String,
    timestamp: u128,
    path: PathBuf,
    error: Option<std::io::Error>,
}

unsafe extern "C" fn reserve_clone(raw: *mut c_void, attempt: u32) -> u32 {
    let context = unsafe { &mut *raw.cast::<CloneContext<'_>>() };
    context.path = std::env::temp_dir().join(format!(
        "ghostos-vm-disk-{}-{}-{attempt}{}", std::process::id(), context.timestamp, context.extension));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&context.path) {
        Ok(file) => { drop(file); 0 },
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => 1,
        Err(error) => { context.error = Some(error); 2 },
    }
}

unsafe extern "C" fn copy_clone(raw: *mut c_void) -> bool {
    let context = unsafe { &mut *raw.cast::<CloneContext<'_>>() };
    match std::fs::copy(context.source, &context.path) {
        Ok(_) => true,
        Err(error) => { context.error = Some(error); false },
    }
}

unsafe extern "C" fn discard_clone(raw: *mut c_void) {
    let context = unsafe { &*raw.cast::<CloneContext<'_>>() };
    let _ = std::fs::remove_file(&context.path);
}

pub(super) fn clone_to_temporary(source: &std::path::Path) -> Result<PathBuf, StorageError> {
    let extension = source.extension().and_then(|value| value.to_str())
        .map(|value| format!(".{value}")).unwrap_or_default();
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| StorageError::InvalidImage(format!("system clock error: {error}")))?.as_nanos();
    let mut context = CloneContext { source, extension, timestamp, path: PathBuf::new(), error: None };
    let io = CCloneIo { reserve: reserve_clone, copy: copy_clone, discard: discard_clone,
        context: (&mut context as *mut CloneContext<'_>).cast() };
    match unsafe { ghostos_vm_disk_clone_temporary(&io) } {
        0 => Ok(context.path),
        1 => Err(StorageError::Io(context.error.take().expect("clone callback retains host error"))),
        2 => Err(StorageError::InvalidImage("could not allocate a unique temporary disk clone".to_string())),
        _ => unreachable!("unknown C clone result"),
    }
}
