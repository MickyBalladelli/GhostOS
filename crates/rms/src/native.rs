pub(crate) fn lock_path(path: &[u8]) -> Result<(), ()> {
    if unsafe { ghostos_rms_lock_path(path.as_ptr(), path.len()) } == 0 { Ok(()) } else { Err(()) }
}
pub(crate) fn resource_id(path: &[u8]) -> u64 {
    unsafe { ghostos_rms_resource_id(path.as_ptr(), path.len()) }
}
pub(crate) fn namespace(name: &[u8]) -> Result<(), ()> {
    if unsafe { ghostos_rms_namespace(name.as_ptr(), name.len()) } == 0 { Ok(()) } else { Err(()) }
}
pub(crate) fn key_path(namespace: &[u8], key: &[u8], destination: &mut [u8]) -> Result<usize, i32> {
    let mut written = 0;
    let code = unsafe { ghostos_rms_key_path(namespace.as_ptr(), namespace.len(), key.as_ptr(),
        key.len(), destination.as_mut_ptr(), destination.len(), &mut written) };
    if code == 0 { Ok(written) } else { Err(code) }
}
pub(crate) fn reserve(len: usize, capacity: usize) -> Result<usize, ()> {
    let mut index = 0;
    if unsafe { ghostos_rms_reserve(len, capacity, &mut index) } == 0 { Ok(index) } else { Err(()) }
}
pub(crate) fn add_count(count: u32) -> Result<u32, ()> {
    let mut next = 0;
    if unsafe { ghostos_rms_add_count(count, &mut next) } { Ok(next) } else { Err(()) }
}
pub(crate) fn sub_count(count: u32) -> Result<u32, ()> {
    let mut next = 0;
    if unsafe { ghostos_rms_sub_count(count, &mut next) } { Ok(next) } else { Err(()) }
}
unsafe extern "C" {
    fn ghostos_rms_lock_path(path: *const u8, length: usize) -> i32;
    fn ghostos_rms_resource_id(path: *const u8, length: usize) -> u64;
    fn ghostos_rms_namespace(name: *const u8, length: usize) -> i32;
    fn ghostos_rms_key_path(namespace: *const u8, namespace_length: usize, key: *const u8,
        key_length: usize, destination: *mut u8, destination_length: usize, written: *mut usize) -> i32;
    fn ghostos_rms_reserve(len: usize, capacity: usize, index: *mut usize) -> i32;
    fn ghostos_rms_add_count(count: u32, next: *mut u32) -> bool;
    fn ghostos_rms_sub_count(count: u32, next: *mut u32) -> bool;
}
