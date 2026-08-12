#![no_std]
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

mod allocator;
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

pub use allocator::{
    AllocationError, EarlyFrameAllocator, QuotaAllocationError, FRAME_SIZE,
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
static DLM: DistributedLockManager = DistributedLockManager::new();
static NODE_FENCES: NodeFenceTable = NodeFenceTable::new();

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn kernel_entry(boot_info: &'static BootInfo) -> ! {
    if let Err(status) = validate_boot_info(boot_info) {
        fatal_kernel_halt(status)
    }

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
    shell::run(boot_info, scheduler, &DLM, &NODE_FENCES, scheduler_clock, acpi)
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
