//! Kernel CSPRNG seeded only by trusted processor entropy instructions.

use core::ffi::c_uchar;

pub const MAX_REQUEST_BYTES: usize = 4096;

unsafe extern "C" {
    fn ghostos_random_initialize();
    fn ghostos_random_ready() -> bool;
    fn ghostos_random_fill(output: *mut c_uchar, length: usize) -> bool;
}

pub fn initialize() {
    unsafe { ghostos_random_initialize() }
}

pub fn ready() -> bool {
    unsafe { ghostos_random_ready() }
}

pub fn fill(output: &mut [u8]) -> bool {
    if output.is_empty() || output.len() > MAX_REQUEST_BYTES || !ready() {
        return false
    }
    unsafe { ghostos_random_fill(output.as_mut_ptr(), output.len()) }
}

pub fn next_u64() -> Option<u64> {
    let mut bytes = [0; size_of::<u64>()];
    fill(&mut bytes).then(|| u64::from_le_bytes(bytes))
}
