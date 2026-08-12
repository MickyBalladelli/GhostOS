use crate::{AddressSpaceId, CpuId, CpuMask};

/// Fixed number of shootdowns that may be waiting for acknowledgements.
pub const MAX_TLB_SHOOTDOWNS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct TlbShootdownId(u64);

impl TlbShootdownId {
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlbShootdownError {
    InvalidRange,
    MissingInitiator,
    Capacity,
    NotFound,
    InvalidTarget,
    AlreadyAcknowledged,
    NotComplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TlbShootdownRequest {
    id: TlbShootdownId,
    address_space: AddressSpaceId,
    start: u64,
    length: u64,
    targets: CpuMask,
    acknowledged: CpuMask,
}

impl TlbShootdownRequest {
    const EMPTY: Option<Self> = None;

    const fn pending(self) -> CpuMask {
        self.targets.difference(self.acknowledged)
    }

    const fn complete(self) -> bool {
        self.pending().is_empty()
    }
}

/// Tracks local invalidation plus acknowledgements from every CPU that may
/// have run the address space. The platform interrupt path can call
/// [`acknowledge`] when it receives the shootdown IPI.
pub struct TlbShootdownCoordinator<const CAPACITY: usize = MAX_TLB_SHOOTDOWNS> {
    next_id: u64,
    requests: [Option<TlbShootdownRequest>; CAPACITY],
}

impl<const CAPACITY: usize> TlbShootdownCoordinator<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            next_id: 1,
            requests: [TlbShootdownRequest::EMPTY; CAPACITY],
        }
    }

    /// Start a shootdown. The initiator invalidates its own TLB before the
    /// request is published; other CPUs remain pending until they acknowledge.
    pub fn begin(
        &mut self,
        address_space: AddressSpaceId,
        start: u64,
        length: u64,
        targets: CpuMask,
        initiator: CpuId,
    ) -> Result<TlbShootdownId, TlbShootdownError> {
        validate_range(start, length)?;
        if targets.is_empty() {
            return Err(TlbShootdownError::InvalidTarget)
        }
        if !targets.contains(initiator) {
            return Err(TlbShootdownError::MissingInitiator)
        }
        let slot = self
            .requests
            .iter()
            .position(Option::is_none)
            .ok_or(TlbShootdownError::Capacity)?;
        let id = TlbShootdownId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);

        crate::arch::invalidate_tlb_range(start, length);
        self.requests[slot] = Some(TlbShootdownRequest {
            id,
            address_space,
            start,
            length,
            targets,
            acknowledged: CpuMask::from_cpu(initiator),
        });
        Ok(id)
    }

    /// Invalidate locally and acknowledge a received shootdown request.
    pub fn acknowledge(
        &mut self,
        id: TlbShootdownId,
        cpu: CpuId,
    ) -> Result<bool, TlbShootdownError> {
        let request = self.request_mut(id)?;
        if !request.targets.contains(cpu) {
            return Err(TlbShootdownError::InvalidTarget)
        }
        if request.acknowledged.contains(cpu) {
            return Err(TlbShootdownError::AlreadyAcknowledged)
        }
        crate::arch::invalidate_tlb_range(request.start, request.length);
        request.acknowledged = request.acknowledged.union(CpuMask::from_cpu(cpu));
        Ok(request.complete())
    }

    pub fn address_space(
        &self,
        id: TlbShootdownId,
    ) -> Result<AddressSpaceId, TlbShootdownError> {
        Ok(self.request(id)?.address_space)
    }

    pub fn targets(&self, id: TlbShootdownId) -> Result<CpuMask, TlbShootdownError> {
        Ok(self.request(id)?.targets)
    }

    pub fn pending_targets(&self, id: TlbShootdownId) -> Result<CpuMask, TlbShootdownError> {
        Ok(self.request(id)?.pending())
    }

    pub fn is_complete(&self, id: TlbShootdownId) -> Result<bool, TlbShootdownError> {
        Ok(self.request(id)?.complete())
    }

    /// Retire a completed request and free its fixed-capacity slot.
    pub fn retire(&mut self, id: TlbShootdownId) -> Result<(), TlbShootdownError> {
        let slot = self.request_index(id)?;
        if !self.requests[slot].is_some_and(|request| request.complete()) {
            return Err(TlbShootdownError::NotComplete)
        }
        self.requests[slot] = None;
        Ok(())
    }

    fn request(&self, id: TlbShootdownId) -> Result<TlbShootdownRequest, TlbShootdownError> {
        self.request_index(id)
            .map(|slot| self.requests[slot].expect("TLB shootdown slot disappeared"))
    }

    fn request_mut(
        &mut self,
        id: TlbShootdownId,
    ) -> Result<&mut TlbShootdownRequest, TlbShootdownError> {
        let slot = self.request_index(id)?;
        Ok(self.requests[slot].as_mut().expect("TLB shootdown slot disappeared"))
    }

    fn request_index(&self, id: TlbShootdownId) -> Result<usize, TlbShootdownError> {
        self.requests
            .iter()
            .position(|request| request.is_some_and(|request| request.id == id))
            .ok_or(TlbShootdownError::NotFound)
    }
}

impl<const CAPACITY: usize> Default for TlbShootdownCoordinator<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

const fn validate_range(start: u64, length: u64) -> Result<(), TlbShootdownError> {
    if start % crate::FRAME_SIZE != 0
        || length == 0
        || length % crate::FRAME_SIZE != 0
        || start.checked_add(length).is_none()
    {
        Err(TlbShootdownError::InvalidRange)
    } else {
        Ok(())
    }
}
