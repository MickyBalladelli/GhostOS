use synos_boot_protocol::{MemoryKind, MemoryRegion};

use crate::capability::{CapabilityHandle, CapabilitySpace};
use crate::quota::{CapabilityQuota, QuotaDecision, QuotaResource};
use crate::task::AddressSpaceId;
use synos_status::{IntoStatus, Status};

pub const FRAME_SIZE: u64 = 4096;
const EARLY_ALLOCATION_FLOOR: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationError;

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
        match self.allocate() {
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
        match self.allocate() {
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
