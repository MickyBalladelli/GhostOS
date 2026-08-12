//! Kernel entry for the native SynOS request ABI.

use core::sync::atomic::{AtomicUsize, Ordering};

use synos_runtime::{Request, Response};
use synos_status::Status;

use crate::task::AddressSpaceId;

/// x86 user processes enter the kernel through this DPL 3 interrupt gate.
pub const CALL_GATE_VECTOR: u8 = 0x80;

/// A registered runtime dispatcher. The caller is taken from the currently
/// running scheduler thread, never from user memory.
pub type DispatchHandler = fn(AddressSpaceId, Request) -> Response;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallError {
    AlreadyInstalled,
}

static DISPATCH_HANDLER: AtomicUsize = AtomicUsize::new(0);

/// Connect the architecture entry point to the kernel runtime dispatcher.
///
/// Installation is one-shot. Service startup owns the dispatcher and can
/// register a small adapter around [`crate::runtime::Dispatcher`].
pub fn install_dispatcher(handler: DispatchHandler) -> Result<(), InstallError> {
    DISPATCH_HANDLER
        .compare_exchange(
            0,
            handler as usize,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map(|_| ())
        .map_err(|_| InstallError::AlreadyInstalled)
}

fn registered_dispatcher() -> Option<DispatchHandler> {
    let raw = DISPATCH_HANDLER.load(Ordering::Acquire);
    if raw == 0 {
        None
    } else {
        // Function pointers are stored as an atomic word so the entry path
        // does not need a lock or allocation.
        Some(unsafe { core::mem::transmute(raw) })
    }
}

pub fn dispatch(caller: AddressSpaceId, request: Request) -> Response {
    registered_dispatcher()
        .map(|handler| handler(caller, request))
        .unwrap_or_else(|| error(Status::BUSY))
}

fn error(status: Status) -> Response {
    Response {
        status: status.raw(),
        flags: 0,
        values: [0; 4],
    }
}

fn valid_user_range(address: usize, length: usize, alignment: usize) -> bool {
    if address == 0 || address % alignment != 0 {
        return false
    }
    let Ok(address) = u64::try_from(address) else {
        return false
    };
    let Ok(length) = u64::try_from(length) else {
        return false
    };
    crate::is_user_range(address, length)
}

/// Common raw-pointer boundary used by the architecture entry stubs.
///
/// The fixed ABI objects must be aligned and live in the caller's user
/// address range. Page ownership and permissions remain enforced by the
/// active address-space page tables.
#[unsafe(no_mangle)]
pub extern "C" fn synos_call_gate_dispatch(
    request: *const Request,
    response: *mut Response,
) {
    if !valid_user_range(
        response as usize,
        core::mem::size_of::<Response>(),
        core::mem::align_of::<Response>(),
    ) {
        return
    }

    if !valid_user_range(
        request as usize,
        core::mem::size_of::<Request>(),
        core::mem::align_of::<Request>(),
    ) {
        unsafe { response.write(error(Status::INVALID_ARGUMENT)) };
        return
    }

    let Some(caller) = crate::current_address_space() else {
        unsafe { response.write(error(Status::BUSY)) };
        return
    };
    let request = unsafe { request.read() };
    let result = dispatch(caller, request);
    unsafe { response.write(result) };
}
