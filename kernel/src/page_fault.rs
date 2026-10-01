use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::address_space::{
    AddressSpaceError, AddressSpaceTable, StackGrowth, StackGrowthError,
};
use crate::allocator::EarlyFrameAllocator;
use crate::capability::{CapabilityHandle, CapabilitySpace};
use crate::cow::{CowError, CowManager, CowPageCopier, CowWriteResult};
use crate::quota::{QuotaDecision, QuotaResource};
use crate::task::AddressSpaceId;
use ghostos_status::{IntoStatus, Status};

pub use ghostos_fabric::PageFault;

pub type PageFaultHandler = fn(PageFault) -> bool;

static HANDLER: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" {
    fn ghostos_page_fault_from_x86_error(virtual_address: u64, error: u64) -> PageFault;
    fn ghostos_page_fault_install_handler(
        handler: extern "C" fn(*const PageFault, *mut c_void) -> bool,
        context: *mut c_void,
    ) -> u32;
    fn ghostos_page_fault_dispatch(fault: PageFault) -> bool;
    fn ghostos_page_fault_dispatch_for(
        caller: u32,
        authority: u64,
        now_us: u64,
        fault: PageFault,
        consume: extern "C" fn(*mut c_void, u32, u64, u64, *mut u64) -> u32,
        refund: extern "C" fn(*mut c_void, u32, u64),
        quota_context: *mut c_void,
        retry_after_us: *mut u64,
        handled: *mut bool,
    ) -> u32;
    fn ghostos_page_fault_resolve_stack_with_ops(
        address_space: u32,
        fault: PageFault,
        inspect: extern "C" fn(*mut c_void, u32, u64, *mut u64) -> u32,
        map_page: extern "C" fn(*mut c_void, u32, u64) -> bool,
        commit: extern "C" fn(*mut c_void, u32, u64, *mut CStackGrowth) -> u32,
        context: *mut c_void,
        growth: *mut CStackGrowth,
    ) -> u32;
    fn ghostos_page_fault_resolve_cow_with_ops(
        address_space: u32,
        fault: PageFault,
        lookup: extern "C" fn(*mut c_void, u32, u64, *mut u64) -> u32,
        write: extern "C" fn(*mut c_void, u32, u64, *mut CCowFaultResult) -> u32,
        replace: extern "C" fn(*mut c_void, u32, u64, u64) -> u32,
        context: *mut c_void,
        result: *mut CCowFaultResult,
    ) -> u32;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CStackGrowth {
    page: u64,
    new_stack_base: u64,
    new_guard_base: u64,
    has_new_guard_base: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CCowFaultResult {
    copied: bool,
    old_frame: u64,
    new_frame: u64,
}

#[allow(unsafe_code)]
pub(crate) fn from_x86_error(virtual_address: u64, error: u64) -> PageFault {
    unsafe { ghostos_page_fault_from_x86_error(virtual_address, error) }
}

extern "C" fn dispatch_bridge(fault: *const PageFault, _context: *mut c_void) -> bool {
    if fault.is_null() {
        return false
    }
    let raw = HANDLER.load(Ordering::Acquire);
    if raw == 0 {
        return false
    }
    // SAFETY: HANDLER contains only the registered PageFaultHandler pointer.
    let handler: PageFaultHandler = unsafe { core::mem::transmute(raw) };
    // SAFETY: the C dispatcher passes a pointer to its live fault value.
    handler(unsafe { *fault })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageFaultHandlerError {
    AlreadyInstalled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageFaultDispatchError {
    InvalidCapability,
    RateLimited { retry_after_us: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CowFaultError {
    InvalidFault,
    AddressSpace(AddressSpaceError),
    Cow(CowError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CowFaultResult {
    MadeWritable { frame: u64 },
    Copied { old_frame: u64, new_frame: u64 },
}

struct CowAdapter<'a, const ADDRESS_SPACES: usize, const COW_PAGES: usize, P: CowPageCopier> {
    address_spaces: &'a mut AddressSpaceTable<ADDRESS_SPACES>,
    cow: &'a mut CowManager<COW_PAGES>,
    allocator: &'a mut EarlyFrameAllocator,
    copier: &'a mut P,
    address_space: AddressSpaceId,
    address_space_error: Option<AddressSpaceError>,
    cow_error: Option<CowError>,
}

extern "C" fn cow_lookup<const A: usize, const C: usize, P: CowPageCopier>(
    context: *mut c_void,
    _address_space: u32,
    page: u64,
    frame: *mut u64,
) -> u32 {
    // SAFETY: resolve_cow_fault passes a live CowAdapter for this call.
    let adapter = unsafe { &mut *(context.cast::<CowAdapter<'_, A, C, P>>()) };
    let space = match adapter.address_spaces.get(adapter.address_space) {
        Ok(space) => space,
        Err(error) => {
            adapter.address_space_error = Some(error);
            return 2
        }
    };
    match space.cow_mapping(page) {
        Ok((_, mapped_frame)) => {
            if let Err(error) = space.can_replace_cow_page(page) {
                adapter.address_space_error = Some(error);
                return 2
            }
            // SAFETY: the C resolver passes a valid output pointer.
            unsafe { *frame = mapped_frame };
            0
        }
        Err(error) => {
            adapter.address_space_error = Some(error);
            2
        }
    }
}

extern "C" fn cow_write<const A: usize, const C: usize, P: CowPageCopier>(
    context: *mut c_void,
    _address_space: u32,
    frame: u64,
    result: *mut CCowFaultResult,
) -> u32 {
    // SAFETY: resolve_cow_fault passes a live CowAdapter for this call.
    let adapter = unsafe { &mut *(context.cast::<CowAdapter<'_, A, C, P>>()) };
    let write = match adapter.cow.write_fault(
        adapter.address_space,
        frame,
        adapter.allocator,
        adapter.copier,
    ) {
        Ok(write) => write,
        Err(error) => {
            adapter.cow_error = Some(error);
            return 3
        }
    };
    let c_result = match write {
        CowWriteResult::Exclusive { frame } => CCowFaultResult {
            copied: false,
            old_frame: 0,
            new_frame: frame,
        },
        CowWriteResult::Copied { old_frame, new_frame } => CCowFaultResult {
            copied: true,
            old_frame,
            new_frame,
        },
    };
    // SAFETY: the C resolver passes a valid output pointer.
    unsafe { *result = c_result };
    0
}

extern "C" fn cow_replace<const A: usize, const C: usize, P: CowPageCopier>(
    context: *mut c_void,
    _address_space: u32,
    page: u64,
    frame: u64,
) -> u32 {
    // SAFETY: resolve_cow_fault passes a live CowAdapter for this call.
    let adapter = unsafe { &mut *(context.cast::<CowAdapter<'_, A, C, P>>()) };
    match adapter.address_spaces.get_mut(adapter.address_space) {
        Ok(space) => match space.replace_cow_page(page, frame) {
            Ok(_) => 0,
            Err(error) => {
                adapter.address_space_error = Some(error);
                2
            }
        },
        Err(error) => {
            adapter.address_space_error = Some(error);
            2
        }
    }
}

pub trait StackPageMapper {
    fn map_stack_page(&mut self, address_space: AddressSpaceId, page: u64) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StackFaultError {
    InvalidFault,
    AddressSpace(AddressSpaceError),
    Growth(StackGrowthError),
    MappingFailed,
}

struct StackAdapter<'a, const CAPACITY: usize, M: StackPageMapper> {
    address_spaces: &'a mut AddressSpaceTable<CAPACITY>,
    mapper: &'a mut M,
    address_space: AddressSpaceId,
    address_space_error: Option<AddressSpaceError>,
    growth_error: Option<StackGrowthError>,
}

extern "C" fn stack_inspect<const CAPACITY: usize, M: StackPageMapper>(
    context: *mut c_void,
    _address_space: u32,
    page: u64,
    candidate: *mut u64,
) -> u32 {
    // SAFETY: resolve_stack_fault passes a live StackAdapter for this call.
    let adapter = unsafe { &mut *(context.cast::<StackAdapter<'_, CAPACITY, M>>()) };
    let space = match adapter.address_spaces.get(adapter.address_space) {
        Ok(space) => space,
        Err(error) => {
            adapter.address_space_error = Some(error);
            return 2
        }
    };
    match space.stack_growth_page(page) {
        Ok(page) => {
            // SAFETY: the C resolver passes a valid output pointer.
            unsafe { *candidate = page };
            0
        }
        Err(error) => {
            adapter.growth_error = Some(error);
            3
        }
    }
}

extern "C" fn stack_map<const CAPACITY: usize, M: StackPageMapper>(
    context: *mut c_void,
    _address_space: u32,
    page: u64,
) -> bool {
    // SAFETY: resolve_stack_fault passes a live StackAdapter for this call.
    let adapter = unsafe { &mut *(context.cast::<StackAdapter<'_, CAPACITY, M>>()) };
    adapter.mapper.map_stack_page(adapter.address_space, page)
}

extern "C" fn stack_commit<const CAPACITY: usize, M: StackPageMapper>(
    context: *mut c_void,
    _address_space: u32,
    page: u64,
    growth: *mut CStackGrowth,
) -> u32 {
    // SAFETY: resolve_stack_fault passes a live StackAdapter for this call.
    let adapter = unsafe { &mut *(context.cast::<StackAdapter<'_, CAPACITY, M>>()) };
    match adapter.address_spaces.get_mut(adapter.address_space) {
        Ok(space) => match space.grow_stack(page) {
            Ok(grown) => {
                // SAFETY: the C resolver passes a valid output pointer.
                unsafe {
                    *growth = CStackGrowth {
                        page: grown.page,
                        new_stack_base: grown.new_stack_base,
                        new_guard_base: grown.new_guard_base.unwrap_or(0),
                        has_new_guard_base: grown.new_guard_base.is_some(),
                    }
                };
                0
            }
            Err(error) => {
                adapter.growth_error = Some(error);
                3
            }
        },
        Err(error) => {
            adapter.address_space_error = Some(error);
            2
        }
    }
}

/// Grow a stack only when the fault hits the current guard page. The mapper
/// must allocate and install the physical page before metadata is committed.
pub fn resolve_stack_fault<const ADDRESS_SPACE_CAPACITY: usize, M: StackPageMapper>(
    address_spaces: &mut AddressSpaceTable<ADDRESS_SPACE_CAPACITY>,
    mapper: &mut M,
    address_space: AddressSpaceId,
    fault: PageFault,
) -> Result<StackGrowth, StackFaultError> {
    let mut adapter = StackAdapter {
        address_spaces,
        mapper,
        address_space,
        address_space_error: None,
        growth_error: None,
    };
    let mut growth = CStackGrowth::default();
    let context = (&mut adapter as *mut StackAdapter<'_, ADDRESS_SPACE_CAPACITY, _>).cast();
    let status = unsafe {
        ghostos_page_fault_resolve_stack_with_ops(
            address_space.raw(),
            fault,
            stack_inspect::<ADDRESS_SPACE_CAPACITY, M>,
            stack_map::<ADDRESS_SPACE_CAPACITY, M>,
            stack_commit::<ADDRESS_SPACE_CAPACITY, M>,
            context,
            &mut growth,
        )
    };
    match status {
        0 => Ok(StackGrowth {
            page: growth.page,
            new_stack_base: growth.new_stack_base,
            new_guard_base: growth.has_new_guard_base.then_some(growth.new_guard_base),
        }),
        1 => Err(StackFaultError::InvalidFault),
        2 => Err(StackFaultError::AddressSpace(
            adapter.address_space_error.unwrap_or(AddressSpaceError::NotFound),
        )),
        3 => Err(StackFaultError::Growth(
            adapter.growth_error.unwrap_or(StackGrowthError::NotGuardPage),
        )),
        _ => Err(StackFaultError::MappingFailed),
    }
}

/// Resolve a present user write fault against a COW mapping. The copier owns
/// the architecture-specific physical-memory access, while the kernel owns
/// frame allocation, refcounts, permissions, and rollback on copy failure.
pub fn resolve_cow_fault<
    const ADDRESS_SPACE_CAPACITY: usize,
    const COW_CAPACITY: usize,
    P: CowPageCopier,
>(
    address_spaces: &mut AddressSpaceTable<ADDRESS_SPACE_CAPACITY>,
    cow: &mut CowManager<COW_CAPACITY>,
    allocator: &mut EarlyFrameAllocator,
    copier: &mut P,
    address_space: AddressSpaceId,
    fault: PageFault,
) -> Result<CowFaultResult, CowFaultError> {
    let mut adapter = CowAdapter {
        address_spaces,
        cow,
        allocator,
        copier,
        address_space,
        address_space_error: None,
        cow_error: None,
    };
    let mut result = CCowFaultResult::default();
    let context = (&mut adapter as *mut CowAdapter<'_, ADDRESS_SPACE_CAPACITY, COW_CAPACITY, _>).cast();
    let status = unsafe {
        ghostos_page_fault_resolve_cow_with_ops(
            address_space.raw(),
            fault,
            cow_lookup::<ADDRESS_SPACE_CAPACITY, COW_CAPACITY, P>,
            cow_write::<ADDRESS_SPACE_CAPACITY, COW_CAPACITY, P>,
            cow_replace::<ADDRESS_SPACE_CAPACITY, COW_CAPACITY, P>,
            context,
            &mut result,
        )
    };
    match status {
        0 if result.copied => Ok(CowFaultResult::Copied {
            old_frame: result.old_frame,
            new_frame: result.new_frame,
        }),
        0 => Ok(CowFaultResult::MadeWritable { frame: result.new_frame }),
        1 => Err(CowFaultError::InvalidFault),
        2 => Err(CowFaultError::AddressSpace(
            adapter.address_space_error.unwrap_or(AddressSpaceError::NotFound),
        )),
        _ => Err(CowFaultError::Cow(adapter.cow_error.unwrap_or(CowError::NotTracked))),
    }
}

impl IntoStatus for PageFaultDispatchError {
    fn status(self) -> Status {
        match self {
            Self::InvalidCapability => Status::ACCESS_DENIED,
            Self::RateLimited { .. } => Status::BUSY,
        }
    }
}

struct QuotaAdapter<'a, const CAPACITY: usize> {
    capabilities: &'a CapabilitySpace<CAPACITY>,
}

extern "C" fn consume_page_fault_quota<const CAPACITY: usize>(
    context: *mut c_void,
    caller: u32,
    authority: u64,
    now_us: u64,
    retry_after_us: *mut u64,
) -> u32 {
    // SAFETY: dispatch_for passes a live QuotaAdapter for the duration of this call.
    let adapter = unsafe { &mut *(context.cast::<QuotaAdapter<'_, CAPACITY>>()) };
    let caller = AddressSpaceId::new(caller).unwrap_or(AddressSpaceId::KERNEL);
    let Some(authority) = CapabilityHandle::from_raw(authority) else { return 3 };
    match adapter.capabilities.consume_quota(caller, authority, QuotaResource::PageFaults, now_us, 1) {
        Ok(QuotaDecision::Allowed) => 0,
        Ok(QuotaDecision::Throttled { retry_after_us: retry }) => {
            // SAFETY: the C dispatcher passes a valid output pointer.
            unsafe { *retry_after_us = retry };
            1
        }
        Ok(QuotaDecision::Rejected) => 2,
        Err(_) => 3,
    }
}

extern "C" fn refund_page_fault_quota<const CAPACITY: usize>(
    context: *mut c_void,
    caller: u32,
    authority: u64,
) {
    // SAFETY: dispatch_for passes a live QuotaAdapter for the duration of this call.
    let adapter = unsafe { &mut *(context.cast::<QuotaAdapter<'_, CAPACITY>>()) };
    let caller = AddressSpaceId::new(caller).unwrap_or(AddressSpaceId::KERNEL);
    if let Some(authority) = CapabilityHandle::from_raw(authority) {
        let _ = adapter.capabilities.refund_quota(caller, authority, QuotaResource::PageFaults, 1);
    }
}

/// Installs the fabric pager callback before user address spaces are started.
///
/// The callback must synchronously install a valid mapping and invalidate the
/// affected TLB entry before returning true. Legacy DSM pagers normally block
/// the faulting thread, fetch the page through their Ring 3 Ethernet service,
/// then finish the mapping through a capability-checked kernel IPC operation.
pub fn install_page_fault_handler(
    handler: PageFaultHandler,
) -> Result<(), PageFaultHandlerError> {
    if HANDLER
        .compare_exchange(0, handler as usize, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(PageFaultHandlerError::AlreadyInstalled)
    }
    let installed = unsafe { ghostos_page_fault_install_handler(dispatch_bridge, core::ptr::null_mut()) };
    if installed == 0 { Ok(()) } else { Err(PageFaultHandlerError::AlreadyInstalled) }
}

#[allow(dead_code)]
pub(crate) fn dispatch(fault: PageFault) -> bool {
    unsafe { ghostos_page_fault_dispatch(fault) }
}

/// Dispatch a fault on behalf of the capability that owns the pager lease.
/// The architecture trap path can use this entry point when it has the
/// current address space and scheduler clock available.
pub fn dispatch_for<const MAX_CAPABILITIES: usize>(
    capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
    caller: AddressSpaceId,
    authority: CapabilityHandle,
    now_us: u64,
    fault: PageFault,
) -> Result<bool, PageFaultDispatchError> {
    let mut adapter = QuotaAdapter { capabilities };
    let mut retry_after_us = 0;
    let mut handled = false;
    let result = unsafe {
        ghostos_page_fault_dispatch_for(
            caller.raw(),
            authority.raw(),
            now_us,
            fault,
            consume_page_fault_quota::<MAX_CAPABILITIES>,
            refund_page_fault_quota::<MAX_CAPABILITIES>,
            (&mut adapter as *mut QuotaAdapter<'_, MAX_CAPABILITIES>).cast(),
            &mut retry_after_us,
            &mut handled,
        )
    };
    match result {
        0 => Ok(handled),
        1 => Err(PageFaultDispatchError::InvalidCapability),
        2 => Err(PageFaultDispatchError::RateLimited { retry_after_us }),
        _ => Err(PageFaultDispatchError::InvalidCapability),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghostos_fabric::Access;

    #[test]
    fn x86_fault_bits_preserve_access_and_protection_state() {
        let fault = PageFault::from_x86_error(0x4000, 1 | (1 << 1) | (1 << 2) | (1 << 4));
        assert_eq!(fault.virtual_address, 0x4000);
        assert_eq!(fault.access, Access::Write);
        assert!(fault.user);
        assert!(fault.present);
        assert!(fault.instruction_fetch);
        assert!(!fault.reserved_bit);
    }
}
