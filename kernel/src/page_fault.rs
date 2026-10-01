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

/// Grow a stack only when the fault hits the current guard page. The mapper
/// must allocate and install the physical page before metadata is committed.
pub fn resolve_stack_fault<const ADDRESS_SPACE_CAPACITY: usize>(
    address_spaces: &mut AddressSpaceTable<ADDRESS_SPACE_CAPACITY>,
    mapper: &mut impl StackPageMapper,
    address_space: AddressSpaceId,
    fault: PageFault,
) -> Result<StackGrowth, StackFaultError> {
    if !fault.user || fault.present || fault.reserved_bit {
        return Err(StackFaultError::InvalidFault)
    }
    let page = fault.page_address();
    let candidate = address_spaces
        .get(address_space)
        .map_err(StackFaultError::AddressSpace)?
        .stack_growth_page(page)
        .map_err(StackFaultError::Growth)?;
    if !mapper.map_stack_page(address_space, candidate) {
        return Err(StackFaultError::MappingFailed)
    }
    address_spaces
        .get_mut(address_space)
        .map_err(StackFaultError::AddressSpace)?
        .grow_stack(page)
        .map_err(StackFaultError::Growth)
}

/// Resolve a present user write fault against a COW mapping. The copier owns
/// the architecture-specific physical-memory access, while the kernel owns
/// frame allocation, refcounts, permissions, and rollback on copy failure.
pub fn resolve_cow_fault<
    const ADDRESS_SPACE_CAPACITY: usize,
    const COW_CAPACITY: usize,
>(
    address_spaces: &mut AddressSpaceTable<ADDRESS_SPACE_CAPACITY>,
    cow: &mut CowManager<COW_CAPACITY>,
    allocator: &mut EarlyFrameAllocator,
    copier: &mut impl CowPageCopier,
    address_space: AddressSpaceId,
    fault: PageFault,
) -> Result<CowFaultResult, CowFaultError> {
    if !fault.user
        || !fault.present
        || fault.reserved_bit
        || fault.access != ghostos_fabric::Access::Write
    {
        return Err(CowFaultError::InvalidFault)
    }
    let page = fault.page_address();
    let (_, frame) = address_spaces
        .get(address_space)
        .map_err(CowFaultError::AddressSpace)?
        .cow_mapping(page)
        .map_err(CowFaultError::AddressSpace)?;
    address_spaces
        .get(address_space)
        .map_err(CowFaultError::AddressSpace)?
        .can_replace_cow_page(page)
        .map_err(CowFaultError::AddressSpace)?;
    match cow
        .write_fault(address_space, frame, allocator, copier)
        .map_err(CowFaultError::Cow)?
    {
        CowWriteResult::Exclusive { frame } => {
            address_spaces
                .get_mut(address_space)
                .map_err(CowFaultError::AddressSpace)?
                .replace_cow_page(page, frame)
                .map_err(CowFaultError::AddressSpace)?;
            Ok(CowFaultResult::MadeWritable { frame })
        }
        CowWriteResult::Copied {
            old_frame,
            new_frame,
        } => {
            address_spaces
                .get_mut(address_space)
                .map_err(CowFaultError::AddressSpace)?
                .replace_cow_page(page, new_frame)
                .map_err(CowFaultError::AddressSpace)?;
            Ok(CowFaultResult::Copied {
                old_frame,
                new_frame,
            })
        }
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
    let decision = capabilities
        .consume_quota(
            caller,
            authority,
            QuotaResource::PageFaults,
            now_us,
            1,
        )
        .map_err(|_| PageFaultDispatchError::InvalidCapability)?;
    match decision {
        QuotaDecision::Allowed => {}
        QuotaDecision::Throttled { retry_after_us } => {
            return Err(PageFaultDispatchError::RateLimited { retry_after_us })
        }
        QuotaDecision::Rejected => {
            return Err(PageFaultDispatchError::RateLimited {
                retry_after_us: u64::MAX,
            })
        }
    }
    let handled = dispatch(fault);
    if !handled {
        let _ = capabilities.refund_quota(
            caller,
            authority,
            QuotaResource::PageFaults,
            1,
        );
    }
    Ok(handled)
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
