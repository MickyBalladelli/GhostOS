use core::sync::atomic::{AtomicUsize, Ordering};

use crate::address_space::{AddressSpaceError, AddressSpaceTable};
use crate::allocator::EarlyFrameAllocator;
use crate::capability::{CapabilityHandle, CapabilitySpace};
use crate::cow::{CowError, CowManager, CowPageCopier, CowWriteResult};
use crate::quota::{QuotaDecision, QuotaResource};
use crate::task::AddressSpaceId;
use synos_status::{IntoStatus, Status};

pub use synos_fabric::PageFault;

pub type PageFaultHandler = fn(PageFault) -> bool;

static HANDLER: AtomicUsize = AtomicUsize::new(0);

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
        || fault.access != synos_fabric::Access::Write
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
    HANDLER
        .compare_exchange(0, handler as usize, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| PageFaultHandlerError::AlreadyInstalled)
}

#[allow(dead_code)]
pub(crate) fn dispatch(fault: PageFault) -> bool {
    let raw = HANDLER.load(Ordering::Acquire);
    if raw == 0 {
        return false
    }
    // Safety: the only nonzero value stored in HANDLER comes from a value of
    // the exact PageFaultHandler function-pointer type above.
    let handler: PageFaultHandler = unsafe { core::mem::transmute(raw) };
    handler(fault)
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
    use synos_fabric::Access;

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
