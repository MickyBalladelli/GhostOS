pub use synos_abi::{Capability, Operation, Request, Response};

pub use synos_abi::{ABI_REVISION, ABI_SCHEMA_VERSION};

pub type GateFn = unsafe extern "C" fn(*const Request, *mut Response);

#[derive(Clone, Copy)]
pub struct NativeGate {
    gate: GateFn,
}

impl NativeGate {
    /// Creates a call gate supplied by the SynOS process loader.
    ///
    /// # Safety
    ///
    /// The gate must obey the generated SynOS system-call ABI for the life
    /// of this value.
    pub const unsafe fn new(gate: GateFn) -> Self {
        Self { gate }
    }
}

pub trait SystemCall {
    fn call(&self, request: Request) -> Response;
}

impl SystemCall for NativeGate {
    fn call(&self, request: Request) -> Response {
        let mut response = Response::EMPTY;
        unsafe { (self.gate)(&request, &mut response) }
        response
    }
}
