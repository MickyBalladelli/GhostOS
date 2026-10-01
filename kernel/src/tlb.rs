use core::ffi::c_void;

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

#[repr(C)]
#[derive(Clone, Copy)]
struct CTlbRequest {
    used: u32,
    address_space: u32,
    id: u64,
    start: u64,
    length: u64,
    targets: [u64; 2],
    acknowledged: [u64; 2],
}

impl CTlbRequest {
    const EMPTY: Self = Self {
        used: 0,
        address_space: 0,
        id: 0,
        start: 0,
        length: 0,
        targets: [0; 2],
        acknowledged: [0; 2],
    };
}

#[repr(C)]
struct CTlbState {
    capacity: usize,
    next_id: u64,
    requests: [CTlbRequest; MAX_TLB_SHOOTDOWNS],
}

unsafe extern "C" {
    fn ghostos_tlb_init(state: *mut CTlbState, capacity: usize);
    fn ghostos_tlb_begin(
        state: *mut CTlbState,
        address_space: u32,
        start: u64,
        length: u64,
        targets: *const u64,
        initiator: u8,
        invalidate: extern "C" fn(*mut c_void, u64, u64),
        send_ipi: extern "C" fn(*mut c_void, u8),
        context: *mut c_void,
        id: *mut u64,
    ) -> u32;
    fn ghostos_tlb_acknowledge(
        state: *mut CTlbState,
        id: u64,
        cpu: u8,
        invalidate: extern "C" fn(*mut c_void, u64, u64),
        context: *mut c_void,
        complete: *mut bool,
    ) -> u32;
    fn ghostos_tlb_request_info(
        state: *const CTlbState,
        id: u64,
        address_space: *mut u32,
        start: *mut u64,
        length: *mut u64,
        targets: *mut u64,
    ) -> u32;
    fn ghostos_tlb_pending_targets(state: *const CTlbState, id: u64, pending: *mut u64) -> bool;
    fn ghostos_tlb_is_complete(state: *const CTlbState, id: u64) -> bool;
    fn ghostos_tlb_retire(state: *mut CTlbState, id: u64) -> u32;
}

extern "C" fn invalidate_range(_: *mut c_void, start: u64, length: u64) {
    crate::arch::invalidate_tlb_range(start, length)
}

extern "C" fn send_shootdown_ipi(_: *mut c_void, raw_cpu: u8) {
    if let Some(cpu) = CpuId::new(raw_cpu) {
        let _ = crate::arch::interrupts::send_ipi(cpu, crate::arch::TLB_SHOOTDOWN_IPI_VECTOR);
    }
}

fn map_result(result: u32) -> Result<(), TlbShootdownError> {
    match result {
        0 => Ok(()),
        1 => Err(TlbShootdownError::InvalidRange),
        2 => Err(TlbShootdownError::MissingInitiator),
        3 => Err(TlbShootdownError::Capacity),
        4 => Err(TlbShootdownError::NotFound),
        5 => Err(TlbShootdownError::InvalidTarget),
        6 => Err(TlbShootdownError::AlreadyAcknowledged),
        7 => Err(TlbShootdownError::NotComplete),
        _ => Err(TlbShootdownError::NotFound),
    }
}

/// Tracks local invalidation plus acknowledgements from every CPU that may
/// have run the address space. C owns the bounded request table and transitions.
pub struct TlbShootdownCoordinator<const CAPACITY: usize = MAX_TLB_SHOOTDOWNS> {
    state: CTlbState,
}

impl<const CAPACITY: usize> TlbShootdownCoordinator<CAPACITY> {
    pub fn new() -> Self {
        let mut state = CTlbState {
            capacity: 0,
            next_id: 0,
            requests: [CTlbRequest::EMPTY; MAX_TLB_SHOOTDOWNS],
        };
        unsafe { ghostos_tlb_init(&mut state, CAPACITY) };
        Self { state }
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
        let words = targets.raw_words();
        let mut id = 0;
        map_result(unsafe {
            ghostos_tlb_begin(
                &mut self.state,
                address_space.raw(),
                start,
                length,
                words.as_ptr(),
                initiator.raw(),
                invalidate_range,
                send_shootdown_ipi,
                core::ptr::null_mut(),
                &mut id,
            )
        })?;
        Ok(TlbShootdownId(id))
    }

    /// Invalidate locally and acknowledge a received shootdown request.
    pub fn acknowledge(
        &mut self,
        id: TlbShootdownId,
        cpu: CpuId,
    ) -> Result<bool, TlbShootdownError> {
        let mut complete = false;
        map_result(unsafe {
            ghostos_tlb_acknowledge(
                &mut self.state,
                id.raw(),
                cpu.raw(),
                invalidate_range,
                core::ptr::null_mut(),
                &mut complete,
            )
        })?;
        Ok(complete)
    }

    pub fn address_space(&self, id: TlbShootdownId) -> Result<AddressSpaceId, TlbShootdownError> {
        let mut raw = 0;
        self.request_info(id, &mut raw, &mut [0; 2])?;
        if raw == 0 {
            Ok(AddressSpaceId::KERNEL)
        } else {
            AddressSpaceId::new(raw).ok_or(TlbShootdownError::NotFound)
        }
    }

    pub fn targets(&self, id: TlbShootdownId) -> Result<CpuMask, TlbShootdownError> {
        let mut words = [0; 2];
        self.request_info(id, &mut 0, &mut words)?;
        Ok(CpuMask::from_words(words[0], words[1]))
    }

    pub fn pending_targets(&self, id: TlbShootdownId) -> Result<CpuMask, TlbShootdownError> {
        let mut pending = [0; 2];
        if !unsafe { ghostos_tlb_pending_targets(&self.state, id.raw(), pending.as_mut_ptr()) } {
            return Err(TlbShootdownError::NotFound)
        }
        Ok(CpuMask::from_words(pending[0], pending[1]))
    }

    pub fn is_complete(&self, id: TlbShootdownId) -> Result<bool, TlbShootdownError> {
        self.request_info(id, &mut 0, &mut [0; 2])?;
        Ok(unsafe { ghostos_tlb_is_complete(&self.state, id.raw()) })
    }

    /// Retire a completed request and free its fixed-capacity slot.
    pub fn retire(&mut self, id: TlbShootdownId) -> Result<(), TlbShootdownError> {
        map_result(unsafe { ghostos_tlb_retire(&mut self.state, id.raw()) })
    }

    fn request_info(
        &self,
        id: TlbShootdownId,
        address_space: &mut u32,
        targets: &mut [u64; 2],
    ) -> Result<(), TlbShootdownError> {
        let mut start = 0;
        let mut length = 0;
        map_result(unsafe {
            ghostos_tlb_request_info(
                &self.state,
                id.raw(),
                address_space,
                &mut start,
                &mut length,
                targets.as_mut_ptr(),
            )
        })
    }
}

impl<const CAPACITY: usize> Default for TlbShootdownCoordinator<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
