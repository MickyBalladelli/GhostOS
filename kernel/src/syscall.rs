//! Kernel entry for the native GhostOS request ABI.

use ghostos_runtime::{Request, Response};
use ghostos_status::Status;

use crate::address_space::{AddressSpaceError, AddressSpaceTable, PAGE_SIZE};
use crate::capability::{CapabilityHandle, CapabilitySpace, Rights};
use crate::runtime::{RuntimeDispatchError, RuntimeOperationService};
use crate::task::AddressSpaceId;

unsafe extern "C" {
    fn ghostos_syscall_valid_user_range(address: u64, length: u64, alignment: u64) -> bool;
    fn ghostos_syscall_validate_request_shape(
        operation: u16,
        abi_version: u16,
        reserved: u16,
    ) -> bool;
    fn ghostos_syscall_sleep_hint(
        operation: u16,
        status: u32,
        deadline: u64,
        now: u64,
    ) -> u64;
    fn ghostos_syscall_install_dispatcher(handler: usize) -> bool;
    fn ghostos_syscall_dispatcher() -> usize;
    fn ghostos_syscall_validate_memory_map(
        flags: u16,
        reserved: u16,
        arguments: *const u64,
    ) -> bool;
    fn ghostos_syscall_validate_memory_unmap(
        flags: u16,
        reserved: u16,
        arguments: *const u64,
    ) -> bool;
}

/// x86 user processes enter the kernel through this DPL 3 interrupt gate.
pub const CALL_GATE_VECTOR: u8 = 0x80;
pub const SLEEP_EXPIRED: u64 = u64::MAX - 1;

/// A registered runtime dispatcher. The caller is taken from the currently
/// running scheduler thread, never from user memory.
pub type DispatchHandler = fn(AddressSpaceId, Request) -> Response;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallError {
    AlreadyInstalled,
}

/// Connect the architecture entry point to the kernel runtime dispatcher.
///
/// Installation is one-shot. Service startup owns the dispatcher and can
/// register a small adapter around [`crate::runtime::Dispatcher`].
pub fn install_dispatcher(handler: DispatchHandler) -> Result<(), InstallError> {
    if unsafe { ghostos_syscall_install_dispatcher(handler as usize) } {
        Ok(())
    } else {
        Err(InstallError::AlreadyInstalled)
    }
}

fn registered_dispatcher() -> Option<DispatchHandler> {
    let raw = unsafe { ghostos_syscall_dispatcher() };
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

pub struct MemorySyscallService<
    'a,
    const ADDRESS_SPACE_CAPACITY: usize = { crate::DEFAULT_KERNEL_PROCESS_CAPACITY },
    const CAPABILITY_CAPACITY: usize = { crate::MAX_CAPABILITIES },
> {
    capabilities: &'a CapabilitySpace<CAPABILITY_CAPACITY>,
    address_spaces: &'a mut AddressSpaceTable<ADDRESS_SPACE_CAPACITY>,
}

impl<
        'a,
        const ADDRESS_SPACE_CAPACITY: usize,
        const CAPABILITY_CAPACITY: usize,
    > MemorySyscallService<'a, ADDRESS_SPACE_CAPACITY, CAPABILITY_CAPACITY>
{
    pub fn new(
        capabilities: &'a CapabilitySpace<CAPABILITY_CAPACITY>,
        address_spaces: &'a mut AddressSpaceTable<ADDRESS_SPACE_CAPACITY>,
    ) -> Self {
        Self {
            capabilities,
            address_spaces,
        }
    }

    fn map(
        &mut self,
        caller: AddressSpaceId,
        request: Request,
    ) -> Result<Response, RuntimeDispatchError> {
        if !unsafe {
            ghostos_syscall_validate_memory_map(
                request.flags,
                request.reserved,
                request.arguments.as_ptr(),
            )
        } {
            return Err(RuntimeDispatchError::InvalidRequest)
        }
        let authority = capability(request.capability)?;
        let writable = request.arguments[2] != 0;
        let backing = self.backing(caller, authority, writable)?;
        let offset = request.arguments[0];
        let length = request.arguments[1];
        if offset % PAGE_SIZE != 0 || length == 0 || length % PAGE_SIZE != 0 {
            return Err(RuntimeDispatchError::InvalidRequest)
        }
        let physical_start = backing
            .start
            .checked_add(offset)
            .ok_or(RuntimeDispatchError::InvalidRequest)?;
        let physical = crate::PhysicalRange::new(physical_start, length)
            .filter(|range| backing.contains(*range))
            .ok_or(RuntimeDispatchError::InvalidRequest)?;
        let mapping = self
            .address_spaces
            .get_mut(caller)
            .map_err(|_| RuntimeDispatchError::ProcessNotRegistered)?
            .map_backing(authority, physical, writable)
            .map_err(map_error)?;
        Ok(Response {
            status: Status::NORMAL.raw(),
            flags: 0,
            values: [mapping.base, 0, 0, 0],
        })
    }

    fn unmap(
        &mut self,
        caller: AddressSpaceId,
        request: Request,
    ) -> Result<Response, RuntimeDispatchError> {
        if !unsafe {
            ghostos_syscall_validate_memory_unmap(
                request.flags,
                request.reserved,
                request.arguments.as_ptr(),
            )
        } {
            return Err(RuntimeDispatchError::InvalidRequest)
        }
        let authority = capability(request.capability)?;
        let info = self
            .capabilities
            .inspect(caller, authority)
            .map_err(|_| RuntimeDispatchError::InvalidCapability)?;
        if !info.rights.contains(Rights::MAP) {
            return Err(RuntimeDispatchError::InvalidCapability)
        }
        self.address_spaces
            .get_mut(caller)
            .map_err(|_| RuntimeDispatchError::ProcessNotRegistered)?
            .unmap_backing(authority, request.arguments[0], request.arguments[1])
            .map_err(map_error)?;
        Ok(Response {
            status: Status::NORMAL.raw(),
            flags: 0,
            values: [0; 4],
        })
    }

    fn backing(
        &self,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        writable: bool,
    ) -> Result<crate::PhysicalRange, RuntimeDispatchError> {
        let info = self
            .capabilities
            .inspect(caller, authority)
            .map_err(|_| RuntimeDispatchError::InvalidCapability)?;
        let mut required = Rights::MAP.union(Rights::READ);
        if writable {
            required = required.union(Rights::WRITE)
        }
        if !info.rights.contains(required) {
            return Err(RuntimeDispatchError::InvalidCapability)
        }
        let backing = info.backing.ok_or(RuntimeDispatchError::InvalidCapability)?;
        if backing.start % PAGE_SIZE != 0
            || backing.length == 0
            || backing.length % PAGE_SIZE != 0
        {
            return Err(RuntimeDispatchError::InvalidCapability)
        }
        Ok(backing)
    }
}

impl<
        'a,
        const ADDRESS_SPACE_CAPACITY: usize,
        const CAPABILITY_CAPACITY: usize,
    > RuntimeOperationService
    for MemorySyscallService<'a, ADDRESS_SPACE_CAPACITY, CAPABILITY_CAPACITY>
{
    fn dispatch(
        &mut self,
        caller: AddressSpaceId,
        operation: ghostos_runtime::Operation,
        request: Request,
    ) -> Result<Response, RuntimeDispatchError> {
        if request.abi_version != ghostos_abi::ABI_SCHEMA_VERSION {
            return Err(RuntimeDispatchError::AbiMismatch)
        }
        match operation {
            ghostos_runtime::Operation::MemoryMap => self.map(caller, request),
            ghostos_runtime::Operation::MemoryUnmap => self.unmap(caller, request),
            _ => Err(RuntimeDispatchError::InvalidRequest),
        }
    }
}

fn capability(raw: u64) -> Result<CapabilityHandle, RuntimeDispatchError> {
    CapabilityHandle::from_raw(raw).ok_or(RuntimeDispatchError::InvalidCapability)
}

fn map_error(error: AddressSpaceError) -> RuntimeDispatchError {
    match error {
        AddressSpaceError::Capacity => RuntimeDispatchError::Capacity,
        AddressSpaceError::NotFound => RuntimeDispatchError::InvalidRequest,
        _ => RuntimeDispatchError::InvalidRequest,
    }
}

fn valid_user_range(address: usize, length: usize, alignment: usize) -> bool {
    let (Ok(address), Ok(length), Ok(alignment)) = (
        u64::try_from(address),
        u64::try_from(length),
        u64::try_from(alignment),
    ) else {
        return false
    };
    unsafe { ghostos_syscall_valid_user_range(address, length, alignment) }
}

pub fn validate_request_shape(request: &Request) -> bool {
    unsafe {
        ghostos_syscall_validate_request_shape(
            request.operation,
            request.abi_version,
            request.reserved,
        )
    }
}

/// Common raw-pointer boundary used by the architecture entry stubs.
///
/// The fixed ABI objects must be aligned and live in the caller's user
/// address range. Page ownership and permissions remain enforced by the
/// active address-space page tables.
#[unsafe(no_mangle)]
pub extern "C" fn ghostos_call_gate_dispatch(
    request: *const Request,
    response: *mut Response,
) -> u64 {
    if !valid_user_range(
        response as usize,
        core::mem::size_of::<Response>(),
        core::mem::align_of::<Response>(),
    ) {
        return 0
    }

    if !valid_user_range(
        request as usize,
        core::mem::size_of::<Request>(),
        core::mem::align_of::<Request>(),
    ) {
        unsafe { crate::arch::write_user(response, error(Status::INVALID_ARGUMENT)) };
        return 0
    }

    let Some(caller) = crate::current_address_space() else {
        unsafe { crate::arch::write_user(response, error(Status::BUSY)) };
        return 0
    };
    let request = unsafe { crate::arch::read_user(request) };
    if request.abi_version != ghostos_abi::ABI_SCHEMA_VERSION {
        unsafe { crate::arch::write_user(response, error(Status::PROTOCOL_MISMATCH)) };
        return 0
    }
    if !validate_request_shape(&request) {
        unsafe { crate::arch::write_user(response, error(Status::INVALID_ARGUMENT)) };
        return 0
    }
    let result = dispatch(caller, request);
    if (1..=14).contains(&caller.raw()) {
        crate::watchdog::service_activity(caller.raw() as usize, crate::time::monotonic_now_us());
    }
    let sleep_us = unsafe {
        ghostos_syscall_sleep_hint(
            request.operation,
            result.status,
            request.arguments[0],
            crate::time::monotonic_now_us(),
        )
    };
    let wire_result = encode_user_response(caller, result);
    unsafe { crate::arch::write_user(response, wire_result) };
    sleep_us
}

/// Map a kernel `Response` onto the Ring 3 wire. Success stays `Status::NORMAL`
/// for every caller, including the boot shell (address space 9).
pub(crate) fn encode_user_response(_caller: AddressSpaceId, result: Response) -> Response {
    result
}
