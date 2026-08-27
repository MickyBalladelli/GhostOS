//! Shared legacy (0.9) virtqueue layout used by virtio-blk, console, RNG, and
//! virtio-net. Ring math must stay identical so the devices cannot diverge.

use crate::memory::Mmu;

pub const QUEUE_SIZE: u16 = 128;
pub const DESC_SIZE: u64 = 16;
pub const MAX_CHAIN: usize = 64;
pub const DESC_NEXT: u16 = 1;
pub const DESC_WRITE: u16 = 2;
pub const DESC_INDIRECT: u16 = 4;

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
        desc_base(self.pfn)
    }

    pub fn avail_base(&self) -> u64 {
        avail_base(self.pfn)
    }

    pub fn used_base(&self) -> u64 {
        used_base(self.pfn)
    }

    pub fn next_available(&mut self, mmu: &Mmu) -> Option<u16> {
        if !self.enabled() {
            return None;
        }
        let avail_idx = read_u16(mmu, self.avail_base() + 2)?;
        if self.avail_last == avail_idx {
            return None;
        }
        if avail_idx.wrapping_sub(self.avail_last) > QUEUE_SIZE {
            // A producer cannot publish more than one queue's worth of
            // entries. Drop the stale window instead of replaying old heads.
            self.avail_last = avail_idx;
            return None;
        }
        let slot = self.avail_last as u64 & (QUEUE_SIZE as u64 - 1);
        let head = read_u16(mmu, self.avail_base() + 4 + slot * 2)?;
        self.avail_last = self.avail_last.wrapping_add(1);
        Some(head)
    }

    pub fn chain(&self, mmu: &Mmu, head: u16) -> Result<Vec<Descriptor>, ()> {
        if head >= QUEUE_SIZE {
            return Err(());
        }
        let mut descriptors = Vec::new();
        let mut index = head;
        for _ in 0..MAX_CHAIN {
            let addr = self
                .desc_base()
                .checked_add(index as u64 * DESC_SIZE)
                .ok_or(())?;
            let bytes = mmu.read_phys(addr, DESC_SIZE as usize).map_err(|_| ())?;
            let descriptor = Descriptor {
                addr: u64::from_le_bytes(bytes[0..8].try_into().map_err(|_| ())?),
                len: u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| ())?),
                flags: u16::from_le_bytes(bytes[12..14].try_into().map_err(|_| ())?),
                next: u16::from_le_bytes(bytes[14..16].try_into().map_err(|_| ())?),
            };
            if descriptor.flags & !(DESC_NEXT | DESC_WRITE | DESC_INDIRECT) != 0
                || descriptor.flags & DESC_INDIRECT != 0
            {
                return Err(());
            }
            descriptors.push(descriptor);
            if descriptor.flags & DESC_NEXT == 0 {
                return Ok(descriptors);
            }
            index = descriptor.next;
            if index >= QUEUE_SIZE {
                return Err(());
            }
        }
        Err(())
    }

    pub fn complete(&mut self, mmu: &mut Mmu, head: u16, len: u32) -> bool {
        let slot = self.used_idx as u64 & (QUEUE_SIZE as u64 - 1);
        let entry = self.used_base() + 4 + slot * 8;
        let mut bytes = [0u8; 8];
        bytes[0..2].copy_from_slice(&head.to_le_bytes());
        bytes[4..8].copy_from_slice(&len.to_le_bytes());
        if mmu.write_phys(entry, &bytes).is_err() {
            return false;
        }
        self.used_idx = self.used_idx.wrapping_add(1);
        mmu.write_phys(self.used_base() + 2, &self.used_idx.to_le_bytes())
            .is_ok()
    }
}

pub fn desc_base(pfn: u32) -> u64 {
    (pfn as u64) << 12
}

pub fn avail_base(pfn: u32) -> u64 {
    desc_base(pfn) + QUEUE_SIZE as u64 * DESC_SIZE
}

pub fn used_base(pfn: u32) -> u64 {
    let avail_end = avail_base(pfn) + 4 + QUEUE_SIZE as u64 * 2;
    (avail_end + 3) & !3
}

fn read_u16(mmu: &Mmu, addr: u64) -> Option<u16> {
    let bytes = mmu.read_phys(addr, 2).ok()?;
    Some(u16::from_le_bytes(bytes.try_into().ok()?))
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
