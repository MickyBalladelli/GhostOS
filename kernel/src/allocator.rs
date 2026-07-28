use synos_boot_protocol::{MemoryKind, MemoryRegion};

pub const FRAME_SIZE: u64 = 4096;
const EARLY_ALLOCATION_FLOOR: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationError;

pub struct EarlyFrameAllocator<'a> {
    regions: &'a [MemoryRegion],
    region_index: usize,
    next_frame: u64,
}

impl<'a> EarlyFrameAllocator<'a> {
    pub fn new(regions: &'a [MemoryRegion]) -> Self {
        let mut allocator = Self {
            regions,
            region_index: 0,
            next_frame: 0,
        };
        allocator.advance_to_usable_region();
        allocator
    }

    pub fn allocate(&mut self) -> Result<u64, AllocationError> {
        loop {
            let region = self.regions.get(self.region_index).ok_or(AllocationError)?;
            let frame = align_up(self.next_frame, FRAME_SIZE);

            if frame.saturating_add(FRAME_SIZE) <= region.end() {
                self.next_frame = frame + FRAME_SIZE;
                return Ok(frame)
            }

            self.region_index += 1;
            self.advance_to_usable_region();
        }
    }

    fn advance_to_usable_region(&mut self) {
        while let Some(region) = self.regions.get(self.region_index) {
            if region.kind == MemoryKind::Usable && region.length >= FRAME_SIZE {
                self.next_frame = align_up(region.start.max(EARLY_ALLOCATION_FLOOR), FRAME_SIZE);
                return
            }
            self.region_index += 1;
        }
    }
}

const fn align_up(value: u64, alignment: u64) -> u64 {
    value.saturating_add(alignment - 1) & !(alignment - 1)
}
