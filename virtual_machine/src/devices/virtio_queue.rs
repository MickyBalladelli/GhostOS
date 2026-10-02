//! Shared legacy (0.9) virtqueue layout used by virtio-blk, console, RNG, and
//! virtio-net. Ring math must stay identical so the devices cannot diverge.

use crate::memory::Mmu;

unsafe extern "C" {
    fn ghostos_vm_virtio_desc_base(pfn: u32) -> u64;
    fn ghostos_vm_virtio_avail_base(pfn: u32) -> u64;
    fn ghostos_vm_virtio_used_base(pfn: u32) -> u64;
    fn ghostos_vm_virtio_next_available(
        pfn: u32,
        avail_last: *mut u16,
        read: unsafe extern "C" fn(*mut core::ffi::c_void, u64, *mut u8, usize) -> bool,
        context: *mut core::ffi::c_void,
        head: *mut u16,
    ) -> bool;
    fn ghostos_vm_virtio_descriptor_chain(
        pfn: u32,
        head: u16,
        read: unsafe extern "C" fn(*mut core::ffi::c_void, u64, *mut u8, usize) -> bool,
        context: *mut core::ffi::c_void,
        output: *mut Descriptor,
        count: *mut usize,
    ) -> bool;
    fn ghostos_vm_virtio_complete(
        pfn: u32,
        used_idx: *mut u16,
        head: u16,
        length: u32,
        write: unsafe extern "C" fn(*mut core::ffi::c_void, u64, *const u8, usize) -> bool,
        context: *mut core::ffi::c_void,
    ) -> bool;
}

unsafe extern "C" fn read_mmu(
    context: *mut core::ffi::c_void,
    address: u64,
    output: *mut u8,
    length: usize,
) -> bool {
    if context.is_null() || (length != 0 && output.is_null()) {
        return false
    }
    let mmu = unsafe { &*(context.cast::<Mmu>()) };
    let Ok(bytes) = mmu.read_phys(address, length) else {
        return false
    };
    if length != 0 {
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), output, length) }
    }
    true
}

unsafe extern "C" fn write_mmu(
    context: *mut core::ffi::c_void,
    address: u64,
    input: *const u8,
    length: usize,
) -> bool {
    if context.is_null() || (length != 0 && input.is_null()) {
        return false
    }
    let mmu = unsafe { &mut *(context.cast::<Mmu>()) };
    let bytes = unsafe { core::slice::from_raw_parts(input, length) };
    mmu.write_phys(address, bytes).is_ok()
}

pub const QUEUE_SIZE: u16 = 128;
pub const DESC_SIZE: u64 = 16;
pub const MAX_CHAIN: usize = 64;
pub const DESC_NEXT: u16 = 1;
pub const DESC_WRITE: u16 = 2;
pub const DESC_INDIRECT: u16 = 4;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Descriptor {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

pub struct VirtioQueue {
    pub pfn: u32,
    avail_last: u16,
    used_idx: u16,
}

impl VirtioQueue {
    pub fn new() -> Self {
        Self {
            pfn: 0,
            avail_last: 0,
            used_idx: 0,
        }
    }

    pub fn set_pfn(&mut self, pfn: u32) {
        self.pfn = pfn;
        self.avail_last = 0;
        self.used_idx = 0;
    }

    pub fn reset(&mut self) {
        self.pfn = 0;
        self.avail_last = 0;
        self.used_idx = 0;
    }

    pub fn enabled(&self) -> bool {
        self.pfn != 0
    }

    pub fn desc_base(&self) -> u64 {
        unsafe { ghostos_vm_virtio_desc_base(self.pfn) }
    }

    pub fn avail_base(&self) -> u64 {
        unsafe { ghostos_vm_virtio_avail_base(self.pfn) }
    }

    pub fn used_base(&self) -> u64 {
        unsafe { ghostos_vm_virtio_used_base(self.pfn) }
    }

    pub fn next_available(&mut self, mmu: &Mmu) -> Option<u16> {
        let mut head = 0;
        let available = unsafe {
            ghostos_vm_virtio_next_available(
                self.pfn,
                &mut self.avail_last,
                read_mmu,
                (mmu as *const Mmu).cast_mut().cast(),
                &mut head,
            )
        };
        available.then_some(head)
    }

    pub fn chain(&self, mmu: &Mmu, head: u16) -> Result<Vec<Descriptor>, ()> {
        let empty = Descriptor { addr: 0, len: 0, flags: 0, next: 0 };
        let mut descriptors = [empty; MAX_CHAIN];
        let mut count = 0;
        let valid = unsafe {
            ghostos_vm_virtio_descriptor_chain(
                self.pfn,
                head,
                read_mmu,
                (mmu as *const Mmu).cast_mut().cast(),
                descriptors.as_mut_ptr(),
                &mut count,
            )
        };
        if valid { Ok(descriptors[..count].to_vec()) } else { Err(()) }
    }

    pub fn complete(&mut self, mmu: &mut Mmu, head: u16, len: u32) -> bool {
        unsafe {
            ghostos_vm_virtio_complete(
                self.pfn,
                &mut self.used_idx,
                head,
                len,
                write_mmu,
                (mmu as *mut Mmu).cast(),
            )
        }
    }
}

pub fn desc_base(pfn: u32) -> u64 {
    unsafe { ghostos_vm_virtio_desc_base(pfn) }
}

pub fn avail_base(pfn: u32) -> u64 {
    unsafe { ghostos_vm_virtio_avail_base(pfn) }
}

pub fn used_base(pfn: u32) -> u64 {
    unsafe { ghostos_vm_virtio_used_base(pfn) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Mmu;

    #[test]
    fn split_queue_layout_matches_legacy_0_9() {
        let pfn = 2;
        let desc = desc_base(pfn);
        assert_eq!(desc, 0x2000);
        assert_eq!(avail_base(pfn), desc + QUEUE_SIZE as u64 * DESC_SIZE);
        let avail_end = avail_base(pfn) + 4 + QUEUE_SIZE as u64 * 2;
        assert_eq!(used_base(pfn), (avail_end + 3) & !3);
        let mut queue = VirtioQueue::new();
        queue.pfn = pfn;
        assert_eq!(queue.desc_base(), desc_base(pfn));
        assert_eq!(queue.avail_base(), avail_base(pfn));
        assert_eq!(queue.used_base(), used_base(pfn));
    }

    #[test]
    fn malformed_descriptor_chain_is_rejected() {
        let mut mmu = Mmu::new(0x20_000);
        let mut queue = VirtioQueue::new();
        queue.pfn = 1;
        let descriptor = [0u8; 16];
        mmu.write_phys(queue.desc_base(), &descriptor).unwrap();
        let mut bad = descriptor;
        bad[12..14].copy_from_slice(&DESC_INDIRECT.to_le_bytes());
        mmu.write_phys(queue.desc_base(), &bad).unwrap();
        assert!(queue.chain(&mmu, 0).is_err());
        assert!(queue.chain(&mmu, QUEUE_SIZE).is_err());
    }
}
