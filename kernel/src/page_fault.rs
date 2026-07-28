use core::sync::atomic::{AtomicUsize, Ordering};

pub use synos_fabric::PageFault;

pub type PageFaultHandler = fn(PageFault) -> bool;

static HANDLER: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageFaultHandlerError {
    AlreadyInstalled,
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
