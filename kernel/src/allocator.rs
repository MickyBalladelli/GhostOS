use ghostos_boot_protocol::{MemoryKind, MemoryRegion};

use crate::capability::{CapabilityHandle, CapabilitySpace, PhysicalRange};
use crate::quota::{CapabilityQuota, QuotaDecision, QuotaResource};
use crate::task::AddressSpaceId;
use ghostos_status::{IntoStatus, Status};

pub const FRAME_SIZE: u64 = 4096;
pub const MAX_OWNED_FRAME_RANGES: usize = 256;
const EARLY_ALLOCATION_FLOOR: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReclaimError {
    InvalidRange,
    NotOwned,
    Capacity,
    AccessDenied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaAllocationError {
    InvalidCapability,
    Exhausted,
    Throttled { retry_after_us: u64 },
    Rejected,
}

impl IntoStatus for QuotaAllocationError {
    fn status(self) -> Status {
        match self {
            Self::InvalidCapability => Status::ACCESS_DENIED,
            Self::Exhausted | Self::Rejected => Status::NO_SPACE,
            Self::Throttled { .. } => Status::BUSY,
        }
    }
}

pub struct EarlyFrameAllocator {
    free: [Option<FrameRange>; ghostos_boot_protocol::MAX_MEMORY_REGIONS],
    free_count: usize,
    owned: [Option<OwnedFrameRange>; MAX_OWNED_FRAME_RANGES],
    owned_count: usize,
}

#[derive(Clone, Copy)]
struct FrameRange {
    start: u64,
    length: u64,
}

impl FrameRange {
    const fn end(self) -> Option<u64> {
        self.start.checked_add(self.length)
    }

    const fn contains(self, other: Self) -> bool {
        other.start >= self.start
            && match (self.end(), other.end()) {
                (Some(end), Some(other_end)) => other_end <= end,
                _ => false,
            }
    }
}

#[derive(Clone, Copy)]
struct OwnedFrameRange {
    range: FrameRange,
    owner: AddressSpaceId,
}

impl EarlyFrameAllocator {
    pub fn new(regions: &[MemoryRegion]) -> Self {
        let mut allocator = Self {
            free: [None; ghostos_boot_protocol::MAX_MEMORY_REGIONS],
            free_count: 0,
            owned: [None; MAX_OWNED_FRAME_RANGES],
            owned_count: 0,
        };
        for region in regions {
            if region.kind != MemoryKind::Usable || region.length < FRAME_SIZE {
                continue
            }
            let Some(end) = region.start.checked_add(region.length) else {
                continue
            };
            let preferred = align_up(region.start.max(EARLY_ALLOCATION_FLOOR), FRAME_SIZE);
            let fallback = align_up(region.start, FRAME_SIZE);
            let start = if preferred.checked_add(FRAME_SIZE).is_some_and(|frame_end| frame_end <= end) {
                preferred
            } else {
                fallback
            };
            if start.checked_add(FRAME_SIZE).is_some_and(|frame_end| frame_end <= end) {
                allocator.insert_free_initial(FrameRange {
                    start,
                    length: end - start,
                });
            }
        }
        allocator
    }

    pub fn allocate(&mut self) -> Result<u64, AllocationError> {
        self.allocate_for_owner(AddressSpaceId::KERNEL)
    }

    pub fn allocate_for_owner(
        &mut self,
        owner: AddressSpaceId,
    ) -> Result<u64, AllocationError> {
        self.allocate_range(owner, 1).map(|range| range.start)
    }

    pub fn allocate_range(
        &mut self,
        owner: AddressSpaceId,
        frame_count: usize,
    ) -> Result<PhysicalRange, AllocationError> {
        if frame_count == 0 {
            return Err(AllocationError)
        }
        let length = u64::try_from(frame_count)
            .ok()
            .and_then(|count| count.checked_mul(FRAME_SIZE))
            .ok_or(AllocationError)?;
        let index = self
            .free
            .iter()
            .position(|range| range.is_some_and(|range| range.length >= length))
            .ok_or(AllocationError)?;
        let range = self.free[index].ok_or(AllocationError)?;
        let allocation = FrameRange {
            start: range.start,
            length,
        };
        if !self.can_insert_owned(allocation, owner) {
            return Err(AllocationError)
        }
        if range.length == length {
            self.free[index] = None;
            self.free_count -= 1;
        } else {
            self.free[index] = Some(FrameRange {
                start: range.start + length,
                length: range.length - length,
            });
        }
        self.insert_owned(allocation, owner);
        PhysicalRange::new(allocation.start, allocation.length).ok_or(AllocationError)
    }

    /// Allocate one frame while charging the tenant's memory-byte quota.
    pub fn allocate_for(
        &mut self,
        quota: &CapabilityQuota,
        now_us: u64,
    ) -> Result<u64, QuotaAllocationError> {
        match quota.consume(QuotaResource::MemoryBytes, now_us, FRAME_SIZE) {
            QuotaDecision::Allowed => {}
            QuotaDecision::Throttled { retry_after_us } => {
                return Err(QuotaAllocationError::Throttled { retry_after_us })
            }
            QuotaDecision::Rejected => return Err(QuotaAllocationError::Rejected),
        }
        match self.allocate_for_owner(AddressSpaceId::KERNEL) {
            Ok(frame) => Ok(frame),
            Err(_) => {
                quota.refund(QuotaResource::MemoryBytes, FRAME_SIZE);
                Err(QuotaAllocationError::Exhausted)
            }
        }
    }

    pub fn allocate_for_capability<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        now_us: u64,
    ) -> Result<u64, QuotaAllocationError> {
        match capabilities
            .consume_quota(
                caller,
                authority,
                QuotaResource::MemoryBytes,
                now_us,
                FRAME_SIZE,
            )
            .map_err(|_| QuotaAllocationError::InvalidCapability)?
        {
            QuotaDecision::Allowed => {}
            QuotaDecision::Throttled { retry_after_us } => {
                return Err(QuotaAllocationError::Throttled { retry_after_us })
            }
            QuotaDecision::Rejected => return Err(QuotaAllocationError::Rejected),
        }
        match self.allocate_for_owner(caller) {
            Ok(frame) => Ok(frame),
            Err(_) => {
                let _ = capabilities.refund_quota(
                    caller,
                    authority,
                    QuotaResource::MemoryBytes,
                    FRAME_SIZE,
                );
                Err(QuotaAllocationError::Exhausted)
            }
        }
    }

    pub fn reclaim(
        &mut self,
        owner: AddressSpaceId,
        frame: u64,
    ) -> Result<(), ReclaimError> {
        let range = PhysicalRange::new(frame, FRAME_SIZE).ok_or(ReclaimError::InvalidRange)?;
        self.reclaim_range(owner, range)
    }

    pub fn reclaim_range(
        &mut self,
        owner: AddressSpaceId,
        range: PhysicalRange,
    ) -> Result<(), ReclaimError> {
        if range.start % FRAME_SIZE != 0 || range.length % FRAME_SIZE != 0 {
            return Err(ReclaimError::InvalidRange)
        }
        if range.length == 0 {
            return Err(ReclaimError::InvalidRange)
        }
        let reclaimed = FrameRange {
            start: range.start,
            length: range.length,
        };
        let index = self
            .owned
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.owner == owner && entry.range.contains(reclaimed)))
            .ok_or(ReclaimError::NotOwned)?;
        let current = self.owned[index].ok_or(ReclaimError::NotOwned)?.range;
        let reclaimed_end = reclaimed.end().ok_or(ReclaimError::InvalidRange)?;
        let current_end = current.end().ok_or(ReclaimError::InvalidRange)?;
        let split = reclaimed.start > current.start && reclaimed_end < current_end;
        if !self.can_insert_free(reclaimed)
            || split && !self.owned.iter().any(|entry| entry.is_none())
        {
            return Err(ReclaimError::Capacity)
        }
        self.remove_owned(index, current, reclaimed, owner, split);
        self.insert_free(reclaimed);
        Ok(())
    }

    pub fn reclaim_for_quota(
        &mut self,
        owner: AddressSpaceId,
        range: PhysicalRange,
        quota: &CapabilityQuota,
    ) -> Result<(), ReclaimError> {
        self.reclaim_range(owner, range)?;
        quota.release_memory(range.length);
        Ok(())
    }

    pub fn reclaim_for_capability<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        range: PhysicalRange,
    ) -> Result<(), ReclaimError> {
        capabilities
            .inspect(caller, authority)
            .map_err(|_| ReclaimError::AccessDenied)?;
        self.reclaim_range(caller, range)?;
        capabilities
            .release_memory(caller, authority, range.length)
            .map_err(|_| ReclaimError::AccessDenied)
    }

    pub fn owner_of(&self, frame: u64) -> Option<AddressSpaceId> {
        if frame % FRAME_SIZE != 0 {
            return None
        }
        let range = FrameRange {
            start: frame,
            length: FRAME_SIZE,
        };
        self.owned
            .iter()
            .flatten()
            .find(|entry| entry.range.contains(range))
            .map(|entry| entry.owner)
    }

    pub fn available_frames(&self) -> u64 {
        self.free
            .iter()
            .flatten()
            .map(|range| range.length / FRAME_SIZE)
            .sum()
    }

    pub fn owned_frames(&self, owner: AddressSpaceId) -> u64 {
        self.owned
            .iter()
            .flatten()
            .filter(|entry| entry.owner == owner)
            .map(|entry| entry.range.length / FRAME_SIZE)
            .sum()
    }

    fn insert_free_initial(&mut self, range: FrameRange) {
        if let Some(slot) = self.free.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(range);
            self.free_count += 1;
        }
    }

    fn can_insert_free(&self, range: FrameRange) -> bool {
        self.free.iter().any(|entry| {
            entry.is_some_and(|entry| {
                entry.end() == Some(range.start) || range.end() == Some(entry.start)
            })
        }) || self.free.iter().any(|entry| entry.is_none())
    }

    fn insert_free(&mut self, range: FrameRange) {
        let left = self
            .free
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.end() == Some(range.start)));
        let right = self
            .free
            .iter()
            .position(|entry| entry.is_some_and(|entry| range.end() == Some(entry.start)));
        match (left, right) {
            (Some(left), Some(right)) if left != right => {
                let end = self.free[right].and_then(FrameRange::end).unwrap();
                let start = self.free[left].unwrap().start;
                self.free[left] = Some(FrameRange {
                    start,
                    length: end - start,
                });
                self.free[right] = None;
                self.free_count -= 1;
            }
            (Some(left), _) => {
                let start = self.free[left].unwrap().start;
                self.free[left] = Some(FrameRange {
                    start,
                    length: range.end().unwrap() - start,
                });
            }
            (None, Some(right)) => {
                let end = self.free[right].and_then(FrameRange::end).unwrap();
                self.free[right] = Some(FrameRange {
                    start: range.start,
                    length: end - range.start,
                });
            }
            (None, None) => self.insert_free_initial(range),
        }
    }

    fn can_insert_owned(&self, range: FrameRange, owner: AddressSpaceId) -> bool {
        self.owned.iter().any(|entry| {
            entry.is_some_and(|entry| {
                entry.owner == owner
                    && (entry.range.end() == Some(range.start)
                        || range.end() == Some(entry.range.start))
            })
        }) || self.owned.iter().any(|entry| entry.is_none())
    }

    fn insert_owned(&mut self, range: FrameRange, owner: AddressSpaceId) {
        let left = self.owned.iter().position(|entry| {
            entry.is_some_and(|entry| {
                entry.owner == owner && entry.range.end() == Some(range.start)
            })
        });
        let right = self.owned.iter().position(|entry| {
            entry.is_some_and(|entry| {
                entry.owner == owner && range.end() == Some(entry.range.start)
            })
        });
        match (left, right) {
            (Some(left), Some(right)) if left != right => {
                let end = self.owned[right].and_then(|entry| entry.range.end()).unwrap();
                let start = self.owned[left].unwrap().range.start;
                self.owned[left] = Some(OwnedFrameRange {
                    range: FrameRange {
                        start,
                        length: end - start,
                    },
                    owner,
                });
                self.owned[right] = None;
                self.owned_count -= 1;
            }
            (Some(left), _) => {
                let start = self.owned[left].unwrap().range.start;
                self.owned[left] = Some(OwnedFrameRange {
                    range: FrameRange {
                        start,
                        length: range.end().unwrap() - start,
                    },
                    owner,
                });
            }
            (None, Some(right)) => {
                let end = self.owned[right].unwrap().range.end().unwrap();
                self.owned[right] = Some(OwnedFrameRange {
                    range: FrameRange {
                        start: range.start,
                        length: end - range.start,
                    },
                    owner,
                });
            }
            (None, None) => {
                let slot = self.owned.iter_mut().find(|entry| entry.is_none()).unwrap();
                *slot = Some(OwnedFrameRange { range, owner });
                self.owned_count += 1;
            }
        }
    }

    fn remove_owned(
        &mut self,
        index: usize,
        current: FrameRange,
        reclaimed: FrameRange,
        owner: AddressSpaceId,
        split: bool,
    ) {
        let current_end = current.end().unwrap();
        let reclaimed_end = reclaimed.end().unwrap();
        if reclaimed.start == current.start && reclaimed_end == current_end {
            self.owned[index] = None;
            self.owned_count -= 1;
        } else if reclaimed.start == current.start {
            self.owned[index] = Some(OwnedFrameRange {
                range: FrameRange {
                    start: reclaimed_end,
                    length: current_end - reclaimed_end,
                },
                owner,
            });
        } else if reclaimed_end == current_end {
            self.owned[index] = Some(OwnedFrameRange {
                range: FrameRange {
                    start: current.start,
                    length: reclaimed.start - current.start,
                },
                owner,
            });
        } else if split {
            self.owned[index] = Some(OwnedFrameRange {
                range: FrameRange {
                    start: current.start,
                    length: reclaimed.start - current.start,
                },
                owner,
            });
            let slot = self.owned.iter_mut().find(|entry| entry.is_none()).unwrap();
            *slot = Some(OwnedFrameRange {
                range: FrameRange {
                    start: reclaimed_end,
                    length: current_end - reclaimed_end,
                },
                owner,
            });
            self.owned_count += 1;
        }
    }
}

const fn align_up(value: u64, alignment: u64) -> u64 {
    value.saturating_add(alignment - 1) & !(alignment - 1)
}
