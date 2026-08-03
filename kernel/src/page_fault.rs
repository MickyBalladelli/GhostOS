use core::sync::atomic::{AtomicUsize, Ordering};

use crate::capability::{CapabilityHandle, CapabilitySpace};
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
