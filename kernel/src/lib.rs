#![no_std]
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

mod allocator;
#[allow(unsafe_code)]
mod boot_services;
pub mod address_space;
pub mod hot_allocator;
#[allow(unsafe_code)]
mod arch;
pub mod capability;
#[allow(unsafe_code)]
mod console;
pub mod contention;
pub mod crash;
pub mod dlm;
pub mod ipc;
pub mod invariants;
pub mod litmus;
pub mod micro_silo;
pub mod quota;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))
]
#[allow(unsafe_code)]
mod keyboard;
#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
#[path = "keyboard_stub.rs"]
mod keyboard;
#[allow(unsafe_code)]
pub mod page_fault;
pub mod persona;
#[allow(unsafe_code)]
mod power;
#[allow(unsafe_code)]
mod persistence;
pub mod partition;
pub mod process;
pub mod scheduler;
pub mod runtime;
pub mod saturation;
#[allow(unsafe_code)]
pub mod syscall;
#[allow(unsafe_code)]
#[allow(dead_code)]
mod shell;
pub mod task;
pub mod monitor;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))
]
#[allow(unsafe_code)]
mod usb_keyboard;
#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
#[path = "usb_keyboard_stub.rs"]
mod usb_keyboard;

use core::panic::PanicInfo;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};
use synos_boot_protocol::BootInfo;
use synos_observability::{
    EventField, EventKind, ProfileDomain, ProfileSample, field, info, record_profile_sample,
};
use synos_status::Status;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
use synos_runtime::{Operation, Request, Response};

pub use allocator::{
    AllocationError, EarlyFrameAllocator, QuotaAllocationError, FRAME_SIZE,
};
pub use address_space::{
    AddressSpace, AddressSpaceError, AddressSpaceTable, PageTableRoot,
    MAX_ADDRESS_SPACE_REGIONS, PAGE_SIZE as ADDRESS_SPACE_PAGE_SIZE, USER_SPACE_END,
    USER_SPACE_START,
};
pub use hot_allocator::{
    HotAllocation, HotAllocationError, HotAllocationPlacement, HotAllocatorConfigError,
    HotAllocatorReport, HotAllocatorStats, HotObjectAllocator, HotObjectKind, HotReclaimError,
};
pub use synos_numa::{NumaCounters, NumaDecision, NumaPlacement, NumaReport, NumaTopology, NumaTopologyError, PlacementKind, PlacementLocality};
pub use capability::{
    CapabilityError, CapabilityHandle, CapabilityInfo, CapabilityLinks, CapabilityObject,
    CapabilityRevocationHook, CapabilitySpace, MAX_CAPABILITIES, PhysicalRange, Rights,
};
pub use contention::{
    duration_bucket, LockGuard, LockShardReport, ShardedTicketLock, LOCK_DURATION_BUCKETS,
};
pub use dlm::{
    DistributedLockManager, DlmContentionReport, FederationClusterId, FederationFenceTable,
    LockError, LockGrant, LockHandle, LockMode, LockOwner, LockOwnership, LockRange,
    NodeFenceState, NodeFenceTable, NodeFenceToken, NodeId, ResourceId, ResourceKind,
    ResourceName, DEFAULT_FEDERATION_CAPACITY, DEFAULT_NODE_FENCE_CAPACITY,
};
pub use micro_silo::{
    BlindMicroSilo, ConfidentialCpu, HardwareIsolation, MemoryProtection, SiloError,
    SiloMemoryRange, SiloObject, SiloOperation, MAX_SILO_MEMORY_RANGES,
};
pub use page_fault::{
    PageFault, PageFaultDispatchError, PageFaultHandler, PageFaultHandlerError,
};
pub use process::{
    KernelProcessBackend, KernelProcessError, ProcessMemory, DEFAULT_KERNEL_PROCESS_CAPACITY,
};
pub use quota::{
    BucketConfig, CapabilityQuota, QuotaContentionReport, QuotaDecision, QuotaPolicy,
    QuotaResource, QuotaUsage,
};
pub use persona::{
    ExecutionPersona, IdentityId, PersonaError, RightIdentifier, MAX_PERSONA_RIGHTS,
};
pub use scheduler::{ContextSwitch, Scheduler, SchedulerError};
pub use synos_observability::{AffinitySet, ScalePath, ScalePolicy, SCALE_CPU_TIERS};
pub use task::{
    AddressSpaceId, Context, CpuId, CpuMask, ExecutionMode, SchedulingPolicy, Thread, ThreadId,
    ThreadState,
};
pub use invariants::{
    CATALOGUE as INVARIANT_CATALOGUE, InvariantDefinition, InvariantFailure, InvariantId,
};

// Scheduler state starts in BSS so the BIOS image carries no large prebuilt
// table. kernel_entry initializes it before interrupts or shell code use it.
static mut SCHEDULER: MaybeUninit<Scheduler> = MaybeUninit::uninit();
static SCHEDULER_READY: AtomicBool = AtomicBool::new(false);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static INIT_REPORTED: AtomicBool = AtomicBool::new(false);
#[allow(dead_code)]
static DLM: DistributedLockManager = DistributedLockManager::new();
#[allow(dead_code)]
static NODE_FENCES: NodeFenceTable = NodeFenceTable::new();

#[allow(unsafe_code)]
pub(crate) fn current_address_space() -> Option<AddressSpaceId> {
    if !SCHEDULER_READY.load(Ordering::Acquire) {
        return None
    }
    unsafe {
        let scheduler = (&*core::ptr::addr_of!(SCHEDULER)).assume_init_ref();
        let current = scheduler.current()?;
        scheduler.thread(current).ok().map(|thread| thread.address_space)
    }
}

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn kernel_entry(boot_info: &'static BootInfo) -> ! {
    if let Err(status) = validate_boot_info(boot_info) {
        fatal_kernel_halt(status)
    }

    arch::disable_interrupts();
    console::init(boot_info.framebuffer);
    info!(
        EventKind::Boot,
        EventField::unsigned(field::OPERATION, 1),
    );
    println!("SynOS kernel bootstrap");

    println!(
        "boot method={} memory regions={}",
        boot_info.method as u32,
        boot_info.memory_region_count
    );

    let mut frames = EarlyFrameAllocator::new(boot_info.regions());
    let mut page_tables = [0; arch::paging::TABLE_FRAME_COUNT];
    for frame in &mut page_tables {
        let Ok(address) = frames.allocate() else {
            fatal_kernel_halt(Status::NO_SPACE)
        };
        *frame = address;
    }
    invariants::debug_assert_valid(invariants::check_page_table_transition(
        &page_tables,
        boot_info.physical_address_offset,
    ));

    let scheduler = unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(SCHEDULER);
        slot.write(Scheduler::new());
        slot.assume_init_mut()
    };
    SCHEDULER_READY.store(true, Ordering::Release);

    arch::initialize(&page_tables, boot_info.physical_address_offset);
    let acpi = power::discover(boot_info);
    if let Some(platform) = acpi {
        let _ = power::enable(&platform);
        println!("ACPI power and thermal tables ready")
    } else {
        println!("ACPI tables unavailable; platform fallback active")
    }
    info!(
        EventKind::Boot,
        EventField::unsigned(field::OPERATION, 2),
        EventField::unsigned(
            field::LENGTH,
            boot_info.memory_region_count as u64,
        ),
    );
    record_profile_sample(ProfileSample::single(
        ProfileDomain::Boot,
        boot_info.memory_region_count as u64,
        0,
        0x1001,
    ));

    let scheduler_clock = scheduler.clock();
    println!(
        "paging, interrupts, capabilities, IPC, and scheduler ready ({} capability slots, {} thread slots, clock={})",
        MAX_CAPABILITIES,
        task::MAX_THREADS,
        scheduler_clock
    );

    let boot_services = boot_services::start()
        .unwrap_or_else(|error| fatal_kernel_halt(error.status()));
    println!(
        "filesystem service registered and started (process={})",
        boot_services.filesystem_process.raw()
    );
    println!(
        "storage service registered and started (process={})",
        boot_services.storage_process.raw()
    );
    println!(
        "network service registered and started (process={})",
        boot_services.network_process.raw()
    );

    // Early hardware setup is complete. Start the first user-space process.
    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    ))]
    boot_synos_init(&mut frames, scheduler, boot_info.physical_address_offset);

    #[cfg(not(all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    )))]
    shell::run(boot_info, scheduler, &DLM, &NODE_FENCES, scheduler_clock, acpi)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn boot_init_dispatch(caller: AddressSpaceId, request: Request) -> Response {
    if Operation::from_raw(request.operation) != Some(Operation::Yield) {
        return Response {
            status: Status::INVALID_ARGUMENT.raw(),
            flags: 0,
            values: [0; 4],
        }
    }
    if INIT_REPORTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        println!("synos-init running in Ring 3 (address space {})", caller.raw());
    }
    Response {
        status: Status::NORMAL.raw(),
        flags: 0,
        values: [0; 4],
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
#[allow(unsafe_code)]
fn boot_synos_init(
    frames: &mut EarlyFrameAllocator<'_>,
    scheduler: &'static mut Scheduler,
    physical_offset: u64,
) -> ! {
    let mut tables = [0u64; arch::paging::DEMO_TABLE_FRAME_COUNT];
    for frame in &mut tables {
        let Ok(address) = frames.allocate() else {
            fatal_kernel_halt(Status::NO_SPACE)
        };
        *frame = address;
    }
    let mut pages = [0u64; 4];
    for page in &mut pages {
        let Ok(address) = frames.allocate() else {
            fatal_kernel_halt(Status::NO_SPACE)
        };
        *page = address;
    }

    let Some(root) = (unsafe {
        arch::paging::install_boot_init_root(&tables, &pages, physical_offset)
    }) else {
        fatal_kernel_halt(Status::INVALID_ARGUMENT)
    };
    unsafe { arch::paging::write_boot_init_image(&pages, physical_offset) };

    let address_space = AddressSpaceId::new(1).expect("boot address space id");
    let mut capabilities: CapabilitySpace<MAX_CAPABILITIES> = CapabilitySpace::new();
    let authority = capabilities
        .mint_root(
            AddressSpaceId::KERNEL,
            CapabilityObject::AddressSpace(address_space),
            Rights::ALL,
        )
        .expect("boot address-space capability");
    let thread = scheduler
        .create_user(
            &capabilities,
            AddressSpaceId::KERNEL,
            authority,
            address_space,
            root,
            SchedulingPolicy::Cooperative,
            pages[0] as usize,
            (pages[3] + FRAME_SIZE) as usize,
        )
        .unwrap_or_else(|_| fatal_kernel_halt(Status::CORRUPT));
    let Some(_) = scheduler.dispatch() else {
        fatal_kernel_halt(Status::BUSY)
    };
    if syscall::install_dispatcher(boot_init_dispatch).is_err() {
        fatal_kernel_halt(Status::BUSY)
    }
    let context = scheduler
        .thread(thread)
        .map(|thread| thread.context)
        .unwrap_or_else(|_| {
            fatal_kernel_halt(Status::CORRUPT)
        });
    println!("starting synos-init in Ring 3 (thread={})", thread.raw());
    arch::enter_user(&context, root)
}

pub fn halt() -> ! {
    loop {
        arch::halt()
    }
}

/// Validate bootloader data before any operation can use its variable-length
/// regions. Invalid metadata is an external input failure, not a panic.
pub fn validate_boot_info(boot_info: &BootInfo) -> Result<(), Status> {
    boot_info.is_valid().then_some(()).ok_or(Status::INVALID_ARGUMENT)
}

/// The only deliberate non-returning failure boundary in kernel code.
pub fn fatal_kernel_halt(status: Status) -> ! {
    crash::capture_and_persist(
        crash::RegisterState::empty(),
        0,
        status,
        1,
        crash_scheduler(),
    );
    println!("KERNEL HALT: status={} ({})", status.raw(), status.message());
    halt()
}

pub fn panic_report(info: &PanicInfo<'_>) -> ! {
    let _ = info;
    println!("KERNEL PANIC");
    fatal_kernel_halt(Status::CORRUPT)
}

#[cfg(any(
    all(
        target_arch = "aarch64",
        any(target_os = "none", target_os = "uefi")
    ),
    all(
        target_arch = "riscv64",
        any(target_os = "none", target_os = "uefi")
    ),
    all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    )
))]
pub(crate) fn capture_exception(
    registers: crash::RegisterState,
    fault_address: u64,
    status: Status,
    reason: u16,
) {
    crash::capture_and_persist(
        registers,
        fault_address,
        status,
        reason,
        crash_scheduler(),
    )
}

#[allow(unsafe_code)]
fn crash_scheduler() -> Option<&'static Scheduler> {
    if !SCHEDULER_READY.load(Ordering::Acquire) {
        return None
    }
    unsafe {
        Some((&*core::ptr::addr_of!(SCHEDULER)).assume_init_ref())
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::console::_print(core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {{
        $crate::print!($($arg)*);
        $crate::print!("\n")
    }};
}

#[cfg(test)]
mod tests;
