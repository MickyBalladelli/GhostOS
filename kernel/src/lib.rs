#![no_std]
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

mod allocator;
#[allow(unsafe_code)]
mod boot_services;
pub mod boot_diagnostics;
pub mod address_space;
pub mod hot_allocator;
#[allow(unsafe_code)]
mod arch;
pub mod capability;
#[allow(unsafe_code)]
mod console;
pub mod contention;
#[allow(unsafe_code)]
pub mod crash;
pub mod cow;
#[allow(unsafe_code)]
pub mod dma;
#[allow(unsafe_code)]
#[allow(dead_code)]
mod driver_capabilities;
#[allow(unsafe_code)]
pub mod dlm;
pub mod ipc;
#[allow(unsafe_code)]
pub mod invariants;
pub mod litmus;
#[allow(unsafe_code)]
pub mod micro_silo;
#[allow(unsafe_code)]
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
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
#[allow(unsafe_code)]
pub mod mouse;
#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
#[path = "mouse_stub.rs"]
#[allow(unsafe_code)]
pub mod mouse;
#[allow(unsafe_code)]
pub mod page_fault;
#[allow(unsafe_code)]
pub mod persona;
#[allow(unsafe_code)]
mod power;
#[allow(unsafe_code)]
#[allow(dead_code)]
mod persistence;
#[allow(unsafe_code)]
mod pci;
#[allow(unsafe_code)]
mod physical_storage;
#[allow(unsafe_code)]
pub mod partition;
#[allow(unsafe_code)]
pub mod process;
#[allow(unsafe_code)]
pub mod scheduler;
#[allow(unsafe_code)]
pub mod runtime;
#[allow(unsafe_code)]
pub mod saturation;
#[allow(unsafe_code)]
pub mod random;
#[allow(unsafe_code)]
pub mod syscall;
#[allow(unsafe_code)]
#[allow(dead_code)]
mod shell;
#[allow(unsafe_code)]
pub mod task;
#[allow(unsafe_code)]
pub mod time;
#[allow(unsafe_code)]
pub mod monitor;
#[allow(unsafe_code)]
pub mod tlb;
#[allow(dead_code)]
#[allow(unsafe_code)]
mod watchdog;
#[cfg(any(
    all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi")),
    test
))]
#[allow(unsafe_code)]
mod webauthn;
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
#[allow(unsafe_code)]
#[path = "usb_keyboard_stub.rs"]
mod usb_keyboard;

use core::panic::PanicInfo;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
use core::sync::atomic::AtomicU8;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
use core::sync::atomic::{AtomicU32, AtomicU64};
use ghostos_boot_protocol::BootInfo;
use ghostos_observability::{
    EventField, EventKind, ProfileDomain, ProfileSample, field, info, record_profile_sample,
};
use ghostos_status::Status;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
use ghostos_runtime::{Operation, Request, Response};

pub use allocator::{
    AllocationError, EarlyFrameAllocator, QuotaAllocationError, ReclaimError,
    MAX_OWNED_FRAME_RANGES, FRAME_SIZE,
};

/// Ring 3 boot-service code pages. Keep `arch/x86_64.rs` paging in sync.
#[allow(dead_code)]
pub(crate) const SERVICE_CODE_PAGE_COUNT: usize = 19;
/// Ring 3 boot-service stack pages (64 KiB). Keep `arch/x86_64.rs` paging in sync.
#[allow(dead_code)]
pub(crate) const SERVICE_STACK_PAGE_COUNT: usize = 16;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const SHELL_LIST_SCRATCH_BYTES: usize = 4096;

#[allow(dead_code)]
pub(crate) fn service_image_fits(length: usize) -> bool {
    length <= SERVICE_CODE_PAGE_COUNT * FRAME_SIZE as usize
}
pub use address_space::{
    AddressSpace, AddressSpaceError, AddressSpaceTable, MemoryAccess, PageTableRoot,
    ProcessIsolationError,
    MAX_ADDRESS_SPACE_REGIONS, PAGE_SIZE as ADDRESS_SPACE_PAGE_SIZE, USER_SPACE_END,
    USER_SPACE_START, KERNEL_SPACE_END, KERNEL_SPACE_START, VIRTUAL_ADDRESS_LAYOUT,
    StackGrowth, StackGrowthError, VirtualAddressLayout, is_user_range,
};
pub use hot_allocator::{
    HotAllocation, HotAllocationError, HotAllocationPlacement, HotAllocatorConfigError,
    HotAllocatorReport, HotAllocatorStats, HotObjectAllocator, HotObjectKind, HotReclaimError,
};
pub use ghostos_numa::{NumaCounters, NumaDecision, NumaPlacement, NumaReport, NumaTopology, NumaTopologyError, PlacementKind, PlacementLocality};
pub use capability::{
    CapabilityError, CapabilityHandle, CapabilityInfo, CapabilityLinks, CapabilityObject,
    CapabilityRevocationHook, CapabilitySpace, DmaDeviceId, MAX_CAPABILITIES, PhysicalRange,
    Rights,
};
pub use dma::{DmaError, DmaManager, DmaMapping, DmaPermissions, Iommu, MAX_DMA_MAPPINGS};
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
    CowFaultError, CowFaultResult, PageFault, PageFaultDispatchError, PageFaultHandler,
    PageFaultHandlerError, StackFaultError, StackPageMapper, resolve_cow_fault,
    resolve_stack_fault,
};
pub use cow::{CowError, CowManager, CowPageCopier, CowPageInfo, CowWriteResult, MAX_COW_PAGES};
pub use process::{
    KernelProcessBackend, KernelProcessError, KernelSupervisorRuntime, NativeServiceImage,
    ProcessMemory, ServiceImageProvider, DEFAULT_KERNEL_PROCESS_CAPACITY,
};
pub use quota::{
    BucketConfig, CapabilityQuota, QuotaContentionReport, QuotaDecision, QuotaPolicy,
    QuotaResource, QuotaUsage,
};
pub use persona::{
    ExecutionPersona, IdentityId, PersonaError, RightIdentifier, MAX_PERSONA_RIGHTS,
};
pub use scheduler::{ContextSwitch, Scheduler, SchedulerError};
pub use ghostos_observability::{AffinitySet, ScalePath, ScalePolicy, SCALE_CPU_TIERS};
pub use task::{
    AddressSpaceId, Context, CpuId, CpuMask, ExecutionMode, SchedulingPolicy, Thread, ThreadId,
    ThreadState,
};
pub use tlb::{TlbShootdownCoordinator, TlbShootdownError, TlbShootdownId, MAX_TLB_SHOOTDOWNS};
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
static mut BOOT_SERVICE_CAPABILITIES:
    [MaybeUninit<CapabilitySpace<MAX_CAPABILITIES>>; 15] =
    [const { MaybeUninit::uninit() }; 15];
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static SERVICE_READY: AtomicU32 = AtomicU32::new(0);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_AUTHORIZED: AtomicBool = AtomicBool::new(false);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_REQUESTED: AtomicBool = AtomicBool::new(true);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_ADMINISTRATOR_EXISTS: AtomicBool = AtomicBool::new(false);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_BOOTSTRAP_PROOF: AtomicBool = AtomicBool::new(false);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_BRIDGE_ACTIVE: AtomicBool = AtomicBool::new(false);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_SESSION_EXPIRES: AtomicU64 = AtomicU64::new(0);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_SESSION_LAST_ACTIVITY: AtomicU64 = AtomicU64::new(0);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_SESSION_IDENTITY: AtomicU64 = AtomicU64::new(0);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_REVOCATION_EPOCH: AtomicU64 = AtomicU64::new(1);
static SESSION_STATE_LOCKED: AtomicBool = AtomicBool::new(false);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOGIN_USERNAME_CAPACITY: usize = 32;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_USERNAME_LENGTH: AtomicU8 = AtomicU8::new(0);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_USERNAME: [AtomicU8; LOGIN_USERNAME_CAPACITY] =
    [const { AtomicU8::new(0) }; LOGIN_USERNAME_CAPACITY];
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_FAILED_ATTEMPTS: AtomicU32 = AtomicU32::new(0);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_RETRY_AFTER_US: AtomicU64 = AtomicU64::new(0);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_LOCKED_UNTIL_US: AtomicU64 = AtomicU64::new(0);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_PASSKEY_CHALLENGE_READY: AtomicBool = AtomicBool::new(false);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_PASSKEY_CHALLENGE: [AtomicU8; 32] = [const { AtomicU8::new(0) }; 32];
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_TPM_CHALLENGE_READY: AtomicBool = AtomicBool::new(false);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static LOGIN_TPM_CHALLENGE: [AtomicU8; 32] = [const { AtomicU8::new(0) }; 32];
#[allow(dead_code)]
static DLM: dlm::KernelDlm = dlm::KernelDlm::new();
static NODE_FENCES: dlm::KernelNodeFences = dlm::KernelNodeFences::new();

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
    watchdog::init();
    let previous_boot_failure = boot_diagnostics::begin();
    if let Err(status) = validate_boot_info(boot_info) {
        fatal_kernel_halt(status)
    }

    arch::disable_interrupts();
    DLM.initialize();
    console::init(boot_info.framebuffer);
    if let Some(failure) = previous_boot_failure {
        let status = ghostos_status::Status::from_raw(failure.status)
            .map(|status| status.message())
            .unwrap_or("unknown");
        println!(
            "previous boot failure: attempt={} stage={} status={} ({}){}",
            failure.id,
            failure.stage.name(),
            failure.status,
            status,
            if failure.interrupted { " (interrupted)" } else { "" },
        );
    }
    info!(
        EventKind::Boot,
        EventField::unsigned(field::OPERATION, 1),
    );
    println!("GhostOS kernel bootstrap");

    println!(
        "boot method={} memory regions={}",
        boot_info.method as u32,
        boot_info.memory_region_count
    );
    boot_diagnostics::checkpoint(boot_diagnostics::BootStage::BootInfoValidated);

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
    boot_diagnostics::checkpoint(boot_diagnostics::BootStage::MemoryReady);
    time::initialize();
    random::initialize();
    let architecture = arch::evidence();
    println!(
        "architecture={} smp={} interrupts={} user-mode={} isolation={}",
        architecture.name,
        architecture.smp,
        architecture.interrupts,
        architecture.user_mode,
        architecture.isolation,
    );
    println!(
        "time rtc={} monotonic={}us entropy={}",
        time::realtime_ready(),
        time::monotonic_now_us(),
        random::ready(),
    );
    boot_diagnostics::checkpoint(boot_diagnostics::BootStage::ArchitectureReady);
    let bootstrap_cpu = arch::interrupts::current_cpu();
    let mut cpu_topology = arch::cpu::CpuTopology::<{ task::MAX_CPUS }>::new();
    let _ = cpu_topology.add_bootstrap(bootstrap_cpu.raw() as u32);
    watchdog::cpu_online(bootstrap_cpu, time::monotonic_now_us());
    println!(
        "cpu topology online={} bootstrap={}",
        cpu_topology.online_count(),
        bootstrap_cpu.raw(),
    );
    let pci_inventory = pci::discover();
    println!(
        "PCI discovery complete ({} device{})",
        pci_inventory.len(),
        if pci_inventory.len() == 1 { "" } else { "s" }
    );
    let acpi = power::discover(boot_info);
    if let Some(platform) = acpi {
        let _ = power::enable(&platform);
        println!("ACPI power and thermal tables ready")
    } else {
        println!("ACPI tables unavailable; platform fallback active")
    }
    boot_diagnostics::checkpoint(boot_diagnostics::BootStage::HardwareReady);

    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    ))]
    if !arch::ring3_supported() {
        println!(
            "NX/XD unavailable; Ring 3 services disabled, entering kernel shell"
        );
        boot_diagnostics::complete();
        shell::run(boot_info, scheduler, &DLM, &NODE_FENCES, scheduler.clock(), acpi)
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

    let physical_filesystem = physical_storage::mount(&pci_inventory);
    if physical_storage::missing_expected_system_volume(
        &pci_inventory,
        physical_filesystem.is_some(),
    ) {
        println!("AHCI adapter present but no mountable GhostOS system volume");
        fatal_kernel_halt(Status::NOT_FOUND);
    }
    boot_diagnostics::checkpoint(boot_diagnostics::BootStage::StorageReady);
    let boot_services = boot_services::start(physical_filesystem)
        .unwrap_or_else(|error| fatal_kernel_halt(error.status()));
    boot_diagnostics::checkpoint(boot_diagnostics::BootStage::ServicesReady);
    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    ))]
    {
        LOGIN_ADMINISTRATOR_EXISTS.store(
            !boot_services.provisioning_required,
            Ordering::Release,
        );
        if boot_services.provisioning_required {
            println!(
                "unprovisioned system detected: authorization database is missing"
            );
            println!("First-run setup mode is active; normal login is disabled.");
        } else {
            println!(
                "authorization database path exists: first-admin setup disabled"
            );
        }
    }
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
    println!(
        "logging service registered and started (process={})",
        boot_services.logging_process.raw()
    );
    println!(
        "audit service registered and started (process={})",
        boot_services.audit_process.raw()
    );
    println!(
        "authentication service registered and started (process={})",
        boot_services.authentication_process.raw()
    );
    println!(
        "package service registered and started (process={})",
        boot_services.package_process.raw()
    );
    println!(
        "shell service registered and started (process={})",
        boot_services.shell_process.raw()
    );
    println!(
        "login service registered and started (process={})",
        boot_services.login_process.raw()
    );
    println!(
        "PCI driver service registered and started (process={})",
        boot_services.pci_process.raw()
    );
    println!(
        "AHCI driver service registered and started (process={})",
        boot_services.ahci_process.raw()
    );
    println!(
        "NVMe driver service registered and started (process={})",
        boot_services.nvme_process.raw()
    );
    println!(
        "Ethernet driver service registered and started (process={})",
        boot_services.ethernet_process.raw()
    );

    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    ))]
    boot_ghostos_init(
        &mut frames,
        scheduler,
        boot_info.physical_address_offset,
        boot_services,
        boot_info,
        scheduler_clock,
        acpi,
        &pci_inventory,
    );

    #[cfg(not(all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    )))]
    boot_diagnostics::complete();

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
const LOCAL_TPM_QUOTE_HEADER_BYTES: usize = 46;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOCAL_TPM_QUOTE_MIN_BYTES: usize = 8;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOCAL_TPM_SIGNATURE_MIN_BYTES: usize = 16;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOCAL_TPM_PCR_DIGEST_BYTES: usize = 32;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOGIN_RATE_LIMIT_BASE_US: u64 = 1_000_000;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOGIN_RATE_LIMIT_MAX_US: u64 = 60_000_000;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOGIN_LOCK_FAILURE_THRESHOLD: u32 = 5;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOGIN_LOCK_DURATION_US: u64 = 300_000_000;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOGIN_SESSION_LIFETIME_US: u64 = 900_000_000;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOGIN_SESSION_IDLE_TIMEOUT_US: u64 = 300_000_000;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const AUDIT_LOGIN_SUCCESS: u64 = 1;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const AUDIT_LOGIN_FAILURE: u64 = 2;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const AUDIT_LOGOUT: u64 = 3;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const AUDIT_TIMEOUT: u64 = 4;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const AUDIT_LOCKOUT: u64 = 5;

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn valid_login_username(username: &[u8]) -> bool {
    !username.is_empty()
        && username.len() <= LOGIN_USERNAME_CAPACITY
        && username
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-$".contains(byte))
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn set_login_username(username: &[u8]) {
    for (slot, value) in LOGIN_USERNAME.iter().zip(username.iter().copied()) {
        slot.store(value.to_ascii_lowercase(), Ordering::Relaxed)
    }
    for slot in LOGIN_USERNAME.iter().skip(username.len()) {
        slot.store(0, Ordering::Relaxed)
    }
    LOGIN_USERNAME_LENGTH.store(username.len() as u8, Ordering::Release)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn clear_login_username() {
    LOGIN_USERNAME_LENGTH.store(0, Ordering::Release);
    for slot in &LOGIN_USERNAME {
        slot.store(0, Ordering::Relaxed)
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn valid_local_tpm_quote(quote: &[u8]) -> bool {
    if quote.len() < LOCAL_TPM_QUOTE_HEADER_BYTES
        || quote[..4] != *b"SYTQ"
        || quote[4] != 1
        || quote[5..8] != [0; 3]
        || !LOGIN_TPM_CHALLENGE_READY.load(Ordering::Acquire)
    {
        return false
    }
    for (index, slot) in LOGIN_TPM_CHALLENGE.iter().enumerate() {
        if quote[8 + index] != slot.load(Ordering::Relaxed) {
            return false
        }
    }
    let quote_length = u16::from_le_bytes([quote[40], quote[41]]) as usize;
    let signature_length = u16::from_le_bytes([quote[42], quote[43]]) as usize;
    let pcr_digest_length = u16::from_le_bytes([quote[44], quote[45]]) as usize;
    let payload_length = quote_length
        .saturating_add(signature_length)
        .saturating_add(pcr_digest_length);
    if quote_length < LOCAL_TPM_QUOTE_MIN_BYTES
        || signature_length < LOCAL_TPM_SIGNATURE_MIN_BYTES
        || pcr_digest_length != LOCAL_TPM_PCR_DIGEST_BYTES
        || LOCAL_TPM_QUOTE_HEADER_BYTES.saturating_add(payload_length) != quote.len()
    {
        return false
    }
    let quote_end = LOCAL_TPM_QUOTE_HEADER_BYTES + quote_length;
    quote[LOCAL_TPM_QUOTE_HEADER_BYTES..LOCAL_TPM_QUOTE_HEADER_BYTES + 4]
        == [0xff, 0x54, 0x43, 0x47]
        && quote[LOCAL_TPM_QUOTE_HEADER_BYTES + 4..LOCAL_TPM_QUOTE_HEADER_BYTES + 6]
            == [0x80, 0x18]
        && quote[quote_end + signature_length..]
            .iter()
            .any(|byte| *byte != 0)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn clear_login_challenge() {
    LOGIN_PASSKEY_CHALLENGE_READY.store(false, Ordering::Release);
    for slot in &LOGIN_PASSKEY_CHALLENGE {
        slot.store(0, Ordering::Relaxed)
    }
    LOGIN_TPM_CHALLENGE_READY.store(false, Ordering::Release);
    for slot in &LOGIN_TPM_CHALLENGE {
        slot.store(0, Ordering::Relaxed)
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn login_rate_limited() -> bool {
    time::monotonic_now_us() < LOGIN_RETRY_AFTER_US.load(Ordering::Acquire)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn login_lock_until_us() -> u64 {
    let now_us = time::monotonic_now_us();
    let locked_until_us = LOGIN_LOCKED_UNTIL_US.load(Ordering::Acquire);
    if locked_until_us == 0 || now_us < locked_until_us {
        return locked_until_us
    }
    match LOGIN_LOCKED_UNTIL_US.compare_exchange(
        locked_until_us,
        0,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => {
            LOGIN_FAILED_ATTEMPTS.store(0, Ordering::Release);
            LOGIN_RETRY_AFTER_US.store(0, Ordering::Release);
            0
        }
        Err(current) => current,
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn login_locked() -> bool {
    login_lock_until_us() != 0
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn record_login_failure() {
    let mut previous = LOGIN_FAILED_ATTEMPTS.load(Ordering::Relaxed);
    let failures = loop {
        let next = previous.saturating_add(1);
        match LOGIN_FAILED_ATTEMPTS.compare_exchange_weak(
            previous,
            next,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => break next,
            Err(actual) => previous = actual,
        }
    };
    let shift = failures.saturating_sub(1).min(6);
    let delay = LOGIN_RATE_LIMIT_BASE_US
        .saturating_mul(1_u64 << shift)
        .min(LOGIN_RATE_LIMIT_MAX_US);
    let now_us = time::monotonic_now_us();
    ghostos_observability::audit_event!(
        ghostos_observability::Level::Warn,
        EventField::unsigned(field::AUTH_ACTION, AUDIT_LOGIN_FAILURE),
        EventField::unsigned(field::CALLER, 14),
        EventField::status(Status::ACCESS_DENIED),
    );
    LOGIN_RETRY_AFTER_US.store(
        now_us.saturating_add(delay),
        Ordering::Release,
    );
    if failures >= LOGIN_LOCK_FAILURE_THRESHOLD {
        LOGIN_LOCKED_UNTIL_US.store(
            now_us.saturating_add(LOGIN_LOCK_DURATION_US),
            Ordering::Release,
        );
        ghostos_observability::audit_event!(
            ghostos_observability::Level::Warn,
            EventField::unsigned(field::AUTH_ACTION, AUDIT_LOCKOUT),
            EventField::unsigned(field::CALLER, 14),
            EventField::status(Status::ACCESS_DENIED),
        );
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn clear_login_failures() {
    LOGIN_FAILED_ATTEMPTS.store(0, Ordering::Release);
    LOGIN_RETRY_AFTER_US.store(0, Ordering::Release);
    LOGIN_LOCKED_UNTIL_US.store(0, Ordering::Release);
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn revoke_login_session() {
    session_state_lock();
    revoke_login_session_locked();
    session_state_unlock();
}

pub(crate) fn session_state_lock() {
    while SESSION_STATE_LOCKED
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop()
    }
}

pub(crate) fn session_state_unlock() {
    SESSION_STATE_LOCKED.store(false, Ordering::Release);
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn revoke_login_session_locked() {
    LOGIN_AUTHORIZED.store(false, Ordering::Release);
    LOGIN_REQUESTED.store(true, Ordering::Release);
    LOGIN_SESSION_EXPIRES.store(0, Ordering::Release);
    LOGIN_SESSION_LAST_ACTIVITY.store(0, Ordering::Release);
    LOGIN_SESSION_IDENTITY.store(0, Ordering::Release);
    let epoch = LOGIN_REVOCATION_EPOCH
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1)
        .max(1);
    let _ = boot_services::set_shell_filesystem_rights(ghostos_fsd::ProcessRights::NONE);
    shell::revoke_session(epoch);
    clear_login_challenge();
    clear_login_username();
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn login_session_active() -> bool {
    session_state_lock();
    let active = login_session_active_locked();
    session_state_unlock();
    active
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn login_session_active_locked() -> bool {
    if !LOGIN_AUTHORIZED.load(Ordering::Acquire) {
        return false
    }
    let now_us = time::monotonic_now_us();
    let expires_at_us = LOGIN_SESSION_EXPIRES.load(Ordering::Acquire);
    let identity = LOGIN_SESSION_IDENTITY.load(Ordering::Acquire);
    let epoch = LOGIN_REVOCATION_EPOCH.load(Ordering::Acquire);
    let login_ok = login_session_matches(identity, expires_at_us, epoch, now_us);
    let shell_ok = shell::session_matches(identity, expires_at_us, epoch);
    if !login_ok || !shell_ok {
        ghostos_observability::audit_event!(
            ghostos_observability::Level::Warn,
            EventField::unsigned(field::AUTH_ACTION, AUDIT_TIMEOUT),
            EventField::unsigned(field::IDENTITY, identity),
            EventField::status(Status::ACCESS_DENIED),
        );
        revoke_login_session_locked();
        return false
    }
    true
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn login_session_matches(
    identity: u64,
    expires_at_us: u64,
    revocation_epoch: u64,
    now_us: u64,
) -> bool {
    LOGIN_AUTHORIZED.load(Ordering::Acquire)
        && identity != 0
        && LOGIN_SESSION_IDENTITY.load(Ordering::Acquire) == identity
        && LOGIN_SESSION_EXPIRES.load(Ordering::Acquire) == expires_at_us
        && LOGIN_REVOCATION_EPOCH.load(Ordering::Acquire) == revocation_epoch
        && now_us < expires_at_us
        && now_us.saturating_sub(LOGIN_SESSION_LAST_ACTIVITY.load(Ordering::Acquire))
            < LOGIN_SESSION_IDLE_TIMEOUT_US
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
pub(crate) fn login_session_matches(
    _identity: u64,
    _expires_at_us: u64,
    _revocation_epoch: u64,
    _now_us: u64,
) -> bool {
    true
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn login_identity(username: &[u8]) -> u64 {
    let mut identity = 0xcbf2_9ce4_8422_2325u64;
    for byte in username {
        identity ^= byte.to_ascii_lowercase() as u64;
        identity = identity.wrapping_mul(0x1000_0000_01b3);
    }
    identity.max(1)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn start_login_session(username: &[u8]) -> bool {
    let now_us = time::monotonic_now_us();
    let expires_at_us = now_us.saturating_add(LOGIN_SESSION_LIFETIME_US);
    let epoch = LOGIN_REVOCATION_EPOCH.load(Ordering::Acquire).max(1);
    let identity = login_identity(username);
    session_state_lock();
    let rights_result = boot_services::set_shell_filesystem_rights(
        ghostos_fsd::ProcessRights::from_bits(
            ghostos_fsd::ProcessRights::READ.bits()
                | ghostos_fsd::ProcessRights::WRITE.bits()
                | ghostos_fsd::ProcessRights::DELETE.bits()
                | ghostos_fsd::ProcessRights::ADMIN.bits(),
        ),
    );
    let authorized = if rights_result.is_err() {
        Err(Status::ACCESS_DENIED)
    } else {
        shell::authorize_session(identity, expires_at_us, epoch)
    };
    if rights_result.is_err() || authorized.is_err() {
        let _ = boot_services::set_shell_filesystem_rights(ghostos_fsd::ProcessRights::NONE);
        session_state_unlock();
        return false
    }
    LOGIN_SESSION_LAST_ACTIVITY.store(now_us, Ordering::Release);
    LOGIN_SESSION_EXPIRES.store(expires_at_us, Ordering::Release);
    LOGIN_SESSION_IDENTITY.store(identity, Ordering::Release);
    LOGIN_REQUESTED.store(false, Ordering::Release);
    LOGIN_AUTHORIZED.store(true, Ordering::Release);
    ghostos_observability::audit_event!(
        ghostos_observability::Level::Info,
        EventField::unsigned(field::AUTH_ACTION, AUDIT_LOGIN_SUCCESS),
        EventField::unsigned(field::IDENTITY, identity),
        EventField::unsigned(field::CALLER, 14),
        EventField::status(Status::NORMAL),
    );
    session_state_unlock();
    drain_console_input();
    true
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn drain_console_input() {
    for _ in 0..4096 {
        if console::read_byte().is_none() {
            break
        }
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn record_login_activity() {
    LOGIN_SESSION_LAST_ACTIVITY.store(time::monotonic_now_us(), Ordering::Release)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
#[allow(unsafe_code)]
fn boot_init_dispatch(caller: AddressSpaceId, request: Request) -> Response {
    let role = caller.raw() as usize;
    let Some(name) = service_name(role) else {
        return syscall_error(Status::ACCESS_DENIED)
    };
    if request.abi_version != ghostos_runtime::ABI_SCHEMA_VERSION || request.reserved != 0 {
        return syscall_error(Status::INVALID_ARGUMENT)
    }
    if role == 9 {
        if let Some(operation) = Operation::from_raw(request.operation)
            && matches!(
                operation,
                Operation::SynFsOpen
                    | Operation::SynFsClose
                    | Operation::SynFsRead
                    | Operation::SynFsWrite
                    | Operation::SynFsMetadata
                    | Operation::SynFsMap
                    | Operation::SynFsUnmap
                    | Operation::SynFsMkdir
                    | Operation::SynFsRmdir
                    | Operation::SynFsList
                    | Operation::SynFsDelete
            )
        {
            if !login_session_active() {
                return syscall_error(Status::ACCESS_DENIED)
            }
            record_login_activity();
            let boot_buffer = ghostos_runtime::decode_boot_fs_buffer(&request.arguments);
            let address = boot_buffer.address;
            let length = boot_buffer.length;
            let writable = boot_buffer.writable;
            if operation == Operation::SynFsClose {
                if request.arguments[..4] != [0; 4] || request.arguments[4..] != [0; 2] {
                    return syscall_error(Status::INVALID_ARGUMENT)
                }
                return boot_services::dispatch_shell_filesystem(
                    operation,
                    request.flags,
                    request.capability,
                    0,
                    0,
                    None,
                )
            }
            if operation == Operation::SynFsMetadata {
                if request.flags != 0 || request.arguments != [0; 6] {
                    return syscall_error(Status::INVALID_ARGUMENT)
                }
                return boot_services::dispatch_shell_filesystem(
                    operation,
                    request.flags,
                    request.capability,
                    0,
                    0,
                    None,
                )
            }
            if operation == Operation::SynFsUnmap {
                if request.flags != 0 || request.arguments != [0; 6] {
                    return syscall_error(Status::INVALID_ARGUMENT)
                }
                return boot_services::dispatch_shell_filesystem(
                    operation,
                    request.flags,
                    request.capability,
                    0,
                    0,
                    None,
                )
            }
            if operation == Operation::SynFsMap {
                if request.flags & !ghostos_fsd::Flags::WRITE.bits() != 0
                    || request.arguments[0] % 4096 != 0
                    || request.arguments[1] == 0
                    || request.arguments[1] % 4096 != 0
                    || request.arguments[1] >> 48 != 0
                    || request.arguments[2..] != [0; 4]
                {
                    return syscall_error(Status::INVALID_ARGUMENT)
                }
                return boot_services::dispatch_shell_filesystem(
                    operation,
                    request.flags,
                    request.capability,
                    request.arguments[0],
                    request.arguments[1],
                    None,
                )
            }
            if request.arguments[3] != 0
                || writable > 1
                || length == 0
                || length > ghostos_fsd::MAX_IPC_BUFFER_BYTES as u64
                || !arch::paging::service_user_range(address, length, writable != 0)
                || (operation != Operation::SynFsList && boot_buffer.path_length != 0)
                || (operation == Operation::SynFsList
                    && (boot_buffer.path_length == 0
                        || boot_buffer.path_length > ghostos_fsd::LIST_PATH_REGION_BYTES as u64
                        || length <= ghostos_fsd::LIST_PATH_REGION_BYTES as u64))
            {
                return syscall_error(Status::INVALID_ARGUMENT)
            }
            return arch::with_user_access(|| {
                let bytes = unsafe {
                    core::slice::from_raw_parts_mut(address as *mut u8, length as usize)
                };
                if operation == Operation::SynFsList {
                    let path_length = boot_buffer.path_length as usize;
                    let scratch_length = core::cmp::min(
                        bytes.len(),
                        SHELL_LIST_SCRATCH_BYTES,
                    );
                    let mut scratch = [0u8; SHELL_LIST_SCRATCH_BYTES];
                    scratch[..path_length].copy_from_slice(&bytes[..path_length]);
                    let response = boot_services::dispatch_shell_filesystem(
                        operation,
                        request.flags,
                        request.capability,
                        boot_buffer.continuation,
                        boot_buffer.path_length,
                        Some(&mut scratch[..scratch_length]),
                    );
                    if response.status == Status::NORMAL.raw() {
                        let output_start = ghostos_fsd::LIST_PATH_REGION_BYTES;
                        let output_length = response.values[0] as usize;
                        let Some(output_end) = output_start.checked_add(output_length) else {
                            return syscall_error(Status::INTERNAL)
                        };
                        if output_end > bytes.len() || output_end > scratch_length {
                            return syscall_error(Status::INTERNAL)
                        }
                        bytes[output_start..output_end]
                            .copy_from_slice(&scratch[output_start..output_end]);
                    }
                    response
                } else {
                    boot_services::dispatch_shell_filesystem(
                        operation,
                        request.flags,
                        request.capability,
                        boot_buffer.continuation,
                        0,
                        Some(bytes),
                    )
                }
            })
        }
    }
    if request.flags != 0 || request.capability != 0 {
        return syscall_error(Status::INVALID_ARGUMENT)
    }
    if Operation::from_raw(request.operation) == Some(Operation::Shutdown) {
        if caller.raw() != 9 || request.arguments != [0; 6] {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        shell::shutdown()
    }
    if Operation::from_raw(request.operation) == Some(Operation::TerminalWrite) {
        let shell_active = caller.raw() == 9 && login_session_active();
        let shell_first_run = caller.raw() == 9
            && !LOGIN_ADMINISTRATOR_EXISTS.load(Ordering::Acquire);
        if caller.raw() != 14 && !shell_active && !shell_first_run {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let address = request.arguments[0] as usize;
        let length = request.arguments[1] as usize;
        if request.arguments[2..] != [0; 4]
            || !arch::paging::service_user_range(address as u64, length as u64, false)
            || length > 4096
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        arch::with_user_access(|| {
            let bytes = unsafe { core::slice::from_raw_parts(address as *const u8, length) };
            console::write_bytes(bytes)
        });
        if shell_active {
            record_login_activity()
        }
        return Response {
            status: Status::NORMAL.raw(),
            flags: 0,
            values: [length as u64, 0, 0, 0],
        }
    }
    if Operation::from_raw(request.operation) == Some(Operation::TerminalRead) {
        let shell_allowed = caller.raw() == 9
            && (login_session_active()
                || !LOGIN_ADMINISTRATOR_EXISTS.load(Ordering::Acquire));
        if caller.raw() != 14 && !shell_allowed {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let address = request.arguments[0] as usize;
        if request.arguments[1] != 1
            || request.arguments[2..] != [0; 4]
            || !arch::paging::service_user_range(address as u64, 1, true)
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let first_run = !LOGIN_ADMINISTRATOR_EXISTS.load(Ordering::Acquire);
        if LOGIN_BRIDGE_ACTIVE.load(Ordering::Acquire) {
            drain_console_input();
        }
        let byte = if caller.raw() == 9 && first_run {
            keyboard::read_boot_byte().or_else(usb_keyboard::read_boot_byte)
        } else if LOGIN_BRIDGE_ACTIVE.load(Ordering::Acquire)
            || LOGIN_REQUESTED.load(Ordering::Acquire)
        {
            keyboard::read_boot_byte().or_else(usb_keyboard::read_boot_byte)
        } else {
            keyboard::read_boot_byte().or_else(console::read_byte)
        };
        if let Some(byte) = byte {
            if caller.raw() == 9 && first_run {
                LOGIN_BOOTSTRAP_PROOF.store(true, Ordering::Release)
            }
            if caller.raw() == 9 {
                watchdog::service_activity(9, time::monotonic_now_us());
            }
            unsafe { arch::write_user(address as *mut u8, byte) };
            if caller.raw() == 9 {
                record_login_activity()
            }
            return Response {
                status: Status::NORMAL.raw(),
                flags: 0,
                values: [1, 0, 0, 0],
            }
        }
        if caller.raw() == 9 {
            watchdog::service_activity(9, time::monotonic_now_us());
        }
        return Response {
            status: Status::NORMAL.raw(),
            flags: 0,
            values: [0; 4],
        }
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginBridgeRead) {
        let first_run = !LOGIN_ADMINISTRATOR_EXISTS.load(Ordering::Acquire);
        if !((caller.raw() == 9 && first_run)
            || (caller.raw() == 14 && LOGIN_REQUESTED.load(Ordering::Acquire)))
            || request.flags != 0
            || request.capability != 0
        {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let address = request.arguments[0] as usize;
        if request.arguments[1] != 1
            || request.arguments[2..] != [0; 4]
            || !arch::paging::service_user_range(address as u64, 1, true)
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let serial_byte = console::read_byte();
        let byte = if let Some(byte) = serial_byte {
            if byte == 0 {
                LOGIN_BRIDGE_ACTIVE.store(true, Ordering::Release);
            }
            Some(byte)
        } else if LOGIN_BRIDGE_ACTIVE.load(Ordering::Acquire) {
            None
        } else if caller.raw() == 9 {
            keyboard::read_boot_byte().or_else(usb_keyboard::read_boot_byte)
        } else {
            keyboard::read_boot_byte()
        };
        if let Some(byte) = byte {
            if caller.raw() == 9 {
                LOGIN_BOOTSTRAP_PROOF.store(true, Ordering::Release);
                watchdog::service_activity(9, time::monotonic_now_us());
            }
            unsafe { arch::write_user(address as *mut u8, byte) };
            return syscall_success([
                1,
                u64::from(LOGIN_BRIDGE_ACTIVE.load(Ordering::Acquire)),
                0,
                0,
            ])
        }
        if caller.raw() == 9 {
            watchdog::service_activity(9, time::monotonic_now_us())
        }
        return syscall_success([0, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginBootstrapUsername) {
        if caller.raw() != 9
            || request.flags != 0
            || request.capability != 0
            || request.arguments[0] == 0
            || request.arguments[1] == 0
            || request.arguments[1] > LOGIN_USERNAME_CAPACITY as u64
            || request.arguments[2..] != [0; 4]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if LOGIN_ADMINISTRATOR_EXISTS.load(Ordering::Acquire)
            || !LOGIN_BOOTSTRAP_PROOF.load(Ordering::Acquire)
        {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let address = request.arguments[0];
        let length = request.arguments[1] as usize;
        if !arch::paging::service_user_range(address, length as u64, false) {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let mut username = [0; LOGIN_USERNAME_CAPACITY];
        arch::with_user_access(|| {
            let source = unsafe { core::slice::from_raw_parts(address as *const u8, length) };
            username[..length].copy_from_slice(source);
        });
        return boot_services::create_first_admin_username(&username[..length]).map_or_else(
            syscall_error,
            |_| {
                set_login_username(&username[..length]);
                syscall_success([length as u64, 0, 0, 0])
            },
        )
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginBootstrapCredential) {
        if caller.raw() != 9
            || request.flags != 0
            || request.capability != 0
            || !(1..=3).contains(&request.arguments[0])
            || request.arguments[1] == 0
            || request.arguments[2] == 0
            || request.arguments[2] > 96
            || request.arguments[3..] != [0; 3]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if LOGIN_ADMINISTRATOR_EXISTS.load(Ordering::Acquire)
            || !LOGIN_BOOTSTRAP_PROOF.load(Ordering::Acquire)
        {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let address = request.arguments[1];
        let length = request.arguments[2] as usize;
        if !arch::paging::service_user_range(address, length as u64, false) {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let mut public_material = [0; 96];
        arch::with_user_access(|| {
            let source = unsafe { core::slice::from_raw_parts(address as *const u8, length) };
            public_material[..length].copy_from_slice(source);
        });
        return boot_services::create_first_admin_credential(
            request.arguments[0] as u8,
            &public_material[..length],
        )
        .map_or_else(syscall_error, |_| syscall_success([length as u64, 0, 0, 0]))
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginBootstrapConfirm) {
        if caller.raw() != 9
            || request.flags != 0
            || request.capability != 0
            || request.arguments != [0; 6]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if LOGIN_ADMINISTRATOR_EXISTS.load(Ordering::Acquire)
            || !LOGIN_BOOTSTRAP_PROOF.load(Ordering::Acquire)
        {
            return syscall_error(Status::ACCESS_DENIED)
        }
        if let Err(status) = boot_services::commit_first_admin() {
            return syscall_error(status)
        }
        let username_length = LOGIN_USERNAME_LENGTH.load(Ordering::Acquire) as usize;
        let mut username = [0; LOGIN_USERNAME_CAPACITY];
        for (slot, value) in LOGIN_USERNAME
            .iter()
            .take(username_length)
            .zip(username.iter_mut())
        {
            *value = slot.load(Ordering::Relaxed)
        }
        LOGIN_REQUESTED.store(false, Ordering::Release);
        LOGIN_ADMINISTRATOR_EXISTS.store(true, Ordering::Release);
        if username_length != 0 && !start_login_session(&username[..username_length]) {
            LOGIN_REQUESTED.store(true, Ordering::Release);
        }
        return syscall_success([1, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginBootstrapRecovery) {
        if caller.raw() != 9
            || request.flags != 0
            || request.capability != 0
            || !(1..=3).contains(&request.arguments[0])
            || request.arguments[1..] != [0; 5]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if LOGIN_ADMINISTRATOR_EXISTS.load(Ordering::Acquire)
            || !LOGIN_BOOTSTRAP_PROOF.load(Ordering::Acquire)
        {
            return syscall_error(Status::ACCESS_DENIED)
        }
        return boot_services::first_admin_recovery(request.arguments[0])
            .map_or_else(syscall_error, syscall_success)
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginComplete) {
        if caller.raw() != 14
            || request.flags != 0
            || request.capability != 0
            || request.arguments[0] == 0
            || request.arguments[1] == 0
            || request.arguments[1] > 127
            || request.arguments[2] == 0
            || request.arguments[3] == 0
            || request.arguments[3] > 512
            || request.arguments[4..] != [0; 2]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if !LOGIN_REQUESTED.load(Ordering::Acquire) {
            return syscall_error(Status::ACCESS_DENIED)
        }
        if login_rate_limited() || login_locked() {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let username_address = request.arguments[0];
        let username_length = request.arguments[1] as usize;
        let assertion_address = request.arguments[2];
        let assertion_length = request.arguments[3] as usize;
        if !arch::paging::service_user_range(username_address, username_length as u64, false)
            || !arch::paging::service_user_range(assertion_address, assertion_length as u64, false)
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let mut username = [0; 128];
        let mut assertion = [0; 512];
        arch::with_user_access(|| {
            let source = unsafe {
                core::slice::from_raw_parts(username_address as *const u8, username_length)
            };
            username[..username_length].copy_from_slice(source);
            let source = unsafe {
                core::slice::from_raw_parts(assertion_address as *const u8, assertion_length)
            };
            assertion[..assertion_length].copy_from_slice(source);
        });
        let mut challenge = [0; 32];
        for (slot, value) in LOGIN_PASSKEY_CHALLENGE.iter().zip(challenge.iter_mut()) {
            *value = slot.load(Ordering::Relaxed)
        }
        let passkey_result = webauthn::verify_local_assertion(
            &assertion[..assertion_length],
            &username[..username_length],
            &challenge,
        );
        let passkey_valid = if let Some((_, key_fingerprint, sign_count)) = passkey_result {
            let persisted = boot_services::local_passkey_sign_count(
                &username[..username_length],
                &key_fingerprint,
            );
            match persisted {
                Err(_) => false,
                Ok(persisted) => {
                    let previous = persisted;
                    if previous != 0 && sign_count != 0 && sign_count <= previous {
                        false
                    } else {
                        if sign_count != 0 {
                            if boot_services::record_local_passkey_sign_count(
                                &username[..username_length],
                                &key_fingerprint,
                                sign_count,
                            )
                            .is_err()
                            {
                                false
                            } else {
                                true
                            }
                        } else {
                            true
                        }
                    }
                }
            }
        } else {
            false
        };
        if !valid_login_username(&username[..username_length]) || !passkey_valid {
            clear_login_challenge();
            record_login_failure();
            return syscall_error(Status::ACCESS_DENIED)
        }
        clear_login_challenge();
        clear_login_failures();
        set_login_username(&username[..username_length]);
        LOGIN_ADMINISTRATOR_EXISTS.store(true, Ordering::Release);
        if !start_login_session(&username[..username_length]) {
            clear_login_username();
            return syscall_error(Status::BUSY)
        }
        return syscall_success([1, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginTpmComplete) {
        if caller.raw() != 14
            || request.flags != 0
            || request.capability != 0
            || request.arguments[0] == 0
            || request.arguments[1] == 0
            || request.arguments[1] > 127
            || request.arguments[2] == 0
            || request.arguments[3] == 0
            || request.arguments[3] > 512
            || request.arguments[4..] != [0; 2]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if !LOGIN_REQUESTED.load(Ordering::Acquire) {
            return syscall_error(Status::ACCESS_DENIED)
        }
        if login_rate_limited() || login_locked() {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let username_address = request.arguments[0];
        let username_length = request.arguments[1] as usize;
        let quote_address = request.arguments[2];
        let quote_length = request.arguments[3] as usize;
        if !arch::paging::service_user_range(username_address, username_length as u64, false)
            || !arch::paging::service_user_range(quote_address, quote_length as u64, false)
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let mut username = [0; 128];
        let mut quote = [0; 512];
        arch::with_user_access(|| {
            let source = unsafe {
                core::slice::from_raw_parts(username_address as *const u8, username_length)
            };
            username[..username_length].copy_from_slice(source);
            let source = unsafe {
                core::slice::from_raw_parts(quote_address as *const u8, quote_length)
            };
            quote[..quote_length].copy_from_slice(source);
        });
        if !valid_login_username(&username[..username_length])
            || !valid_local_tpm_quote(&quote[..quote_length])
        {
            clear_login_challenge();
            record_login_failure();
            return syscall_error(Status::ACCESS_DENIED)
        }
        clear_login_challenge();
        clear_login_failures();
        set_login_username(&username[..username_length]);
        LOGIN_ADMINISTRATOR_EXISTS.store(true, Ordering::Release);
        if !start_login_session(&username[..username_length]) {
            clear_login_username();
            return syscall_error(Status::BUSY)
        }
        return syscall_success([1, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginStatus) {
        if caller.raw() != 9 && caller.raw() != 14
            || request.flags != 0
            || request.capability != 0
            || request.arguments != [0; 6]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let authorized = login_session_active();
        return syscall_success([
            LOGIN_REQUESTED.load(Ordering::Acquire) as u64,
            LOGIN_ADMINISTRATOR_EXISTS.load(Ordering::Acquire) as u64,
            login_lock_until_us(),
            authorized as u64,
        ])
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginStart) {
        if caller.raw() != 9
            || request.flags != 0
            || request.capability != 0
            || request.arguments != [0; 6]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if !login_session_active() {
            return syscall_error(Status::ACCESS_DENIED)
        }
        ghostos_observability::audit_event!(
            ghostos_observability::Level::Info,
            EventField::unsigned(field::AUTH_ACTION, AUDIT_LOGOUT),
            EventField::unsigned(
                field::IDENTITY,
                LOGIN_SESSION_IDENTITY.load(Ordering::Acquire),
            ),
            EventField::status(Status::NORMAL),
        );
        revoke_login_session();
        return syscall_success([1, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginLogout) {
        if caller.raw() != 9
            || request.flags != 0
            || request.capability != 0
            || request.arguments != [0; 6]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if !login_session_active() {
            return syscall_error(Status::ACCESS_DENIED)
        }
        ghostos_observability::audit_event!(
            ghostos_observability::Level::Info,
            EventField::unsigned(field::AUTH_ACTION, AUDIT_LOGOUT),
            EventField::unsigned(
                field::IDENTITY,
                LOGIN_SESSION_IDENTITY.load(Ordering::Acquire),
            ),
            EventField::status(Status::NORMAL),
        );
        revoke_login_session();
        return syscall_success([1, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginWhoami) {
        if caller.raw() != 9
            || request.flags != 0
            || request.capability != 0
            || request.arguments[0] == 0
            || request.arguments[1] < LOGIN_USERNAME_CAPACITY as u64
            || request.arguments[1] > 128
            || request.arguments[2..] != [0; 4]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if !login_session_active() {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let address = request.arguments[0];
        let capacity = request.arguments[1];
        if !arch::paging::service_user_range(address, capacity, true) {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let length = LOGIN_USERNAME_LENGTH.load(Ordering::Acquire) as usize;
        arch::with_user_access(|| unsafe {
            let target = core::slice::from_raw_parts_mut(address as *mut u8, capacity as usize);
            for (slot, byte) in LOGIN_USERNAME.iter().zip(target.iter_mut()).take(length) {
                *byte = slot.load(Ordering::Relaxed);
            }
        });
        return syscall_success([length as u64, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginRevokeIdentity) {
        if caller.raw() != 9
            || request.flags != 0
            || request.capability != 0
            || request.arguments[0] == 0
            || request.arguments[1] == 0
            || request.arguments[1] > LOGIN_USERNAME_CAPACITY as u64
            || request.arguments[2..] != [0; 4]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if !login_session_active() {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let address = request.arguments[0];
        let length = request.arguments[1] as usize;
        if !arch::paging::service_user_range(address, length as u64, false) {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let mut username = [0; LOGIN_USERNAME_CAPACITY];
        arch::with_user_access(|| unsafe {
            let source = core::slice::from_raw_parts(address as *const u8, length);
            username[..length].copy_from_slice(source);
        });
        if !valid_login_username(&username[..length]) {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let identity = login_identity(&username[..length]);
        if LOGIN_SESSION_IDENTITY.load(Ordering::Acquire) == identity {
            revoke_login_session();
            return syscall_success([1, 0, 0, 0])
        }
        return syscall_success([0, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginChallenge) {
        if caller.raw() != 14
            || request.flags != 0
            || request.capability != 0
            || request.arguments[0] == 0
            || request.arguments[1] != 32
            || request.arguments[2..] != [0; 4]
            || !arch::paging::service_user_range(request.arguments[0], 32, true)
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if login_rate_limited() || login_locked() {
            return syscall_error(Status::ACCESS_DENIED)
        }
        if !LOGIN_REQUESTED.load(Ordering::Acquire) || !random::ready() {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let mut challenge = [0; 32];
        if !random::fill(&mut challenge) {
            return syscall_error(Status::BUSY)
        }
        arch::with_user_access(|| unsafe {
            core::slice::from_raw_parts_mut(request.arguments[0] as *mut u8, 32)
                .copy_from_slice(&challenge)
        });
        for (slot, value) in LOGIN_PASSKEY_CHALLENGE.iter().zip(challenge) {
            slot.store(value, Ordering::Relaxed)
        }
        LOGIN_PASSKEY_CHALLENGE_READY.store(true, Ordering::Release);
        return syscall_success([32, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::LoginTpmChallenge) {
        if caller.raw() != 14
            || request.flags != 0
            || request.capability != 0
            || request.arguments[0] == 0
            || request.arguments[1] != 32
            || request.arguments[2..] != [0; 4]
            || !arch::paging::service_user_range(request.arguments[0], 32, true)
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if login_rate_limited() || login_locked() {
            return syscall_error(Status::ACCESS_DENIED)
        }
        if !LOGIN_REQUESTED.load(Ordering::Acquire) || !random::ready() {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let mut challenge = [0; 32];
        if !random::fill(&mut challenge) {
            return syscall_error(Status::BUSY)
        }
        arch::with_user_access(|| unsafe {
            core::slice::from_raw_parts_mut(request.arguments[0] as *mut u8, 32)
                .copy_from_slice(&challenge)
        });
        for (slot, value) in LOGIN_TPM_CHALLENGE.iter().zip(challenge) {
            slot.store(value, Ordering::Relaxed)
        }
        LOGIN_TPM_CHALLENGE_READY.store(true, Ordering::Release);
        return syscall_success([32, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::ServiceReady) {
        if request.arguments[0] as usize != role || request.arguments[1..] != [0; 5] {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if role == 9
            && SERVICE_READY.load(Ordering::Acquire) & ((1u32 << 7) | (1u32 << 14))
                != ((1u32 << 7) | (1u32 << 14))
        {
            return syscall_error(Status::BUSY)
        }
        let bit = 1u32 << role;
        let ready = SERVICE_READY.load(Ordering::Acquire);
        if ready & bit == 0 {
            SERVICE_READY.store(ready | bit, Ordering::Release);
            watchdog::service_ready(role, time::monotonic_now_us());
            println!("{} ready in Ring 3 (address space {})", name, role);
        }
        return syscall_success([role as u64, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::ServiceHeartbeat) {
        if request.arguments[0] as usize != role || request.arguments[1] == 0
            || request.arguments[2..] != [0; 4]
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if SERVICE_READY.load(Ordering::Acquire) & (1u32 << role) == 0 {
            return syscall_error(Status::BUSY)
        }
        watchdog::service_heartbeat(role, request.arguments[1], time::monotonic_now_us());
        return syscall_success([request.arguments[1], 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::SystemInfo) {
        if role != 9 || request.arguments != [0; 6] {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let heartbeats = watchdog::service_sequences()
            .skip(1)
            .fold(0u64, |total, value| total.saturating_add(value));
        return syscall_success([
            scheduler_clock(),
            SERVICE_READY.load(Ordering::Acquire) as u64,
            heartbeats,
            role as u64,
        ])
    }
    if Operation::from_raw(request.operation) == Some(Operation::WatchdogDiagnostics) {
        if role != 9 || request.arguments[1..] != [0; 5] || request.arguments[0] > 2 {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if request.arguments[0] == 1 {
            watchdog::set_diagnostics_enabled(true);
        } else if request.arguments[0] == 2 {
            watchdog::set_diagnostics_enabled(false);
        }
        return syscall_success([
            watchdog::diagnostics_enabled() as u64,
            watchdog::SERVICE_TIMEOUT_US,
            watchdog::SERVICE_CAPACITY as u64,
            0,
        ])
    }
    if Operation::from_raw(request.operation) == Some(Operation::ShellPoll) {
        if role != 9 || request.arguments != [0; 6] {
            return syscall_error(Status::ACCESS_DENIED)
        }
        if SERVICE_READY.load(Ordering::Acquire) & (1u32 << role) == 0 {
            return syscall_error(Status::BUSY)
        }
        return match shell::poll_input() {
            Ok(processed) => syscall_success([processed, 0, 0, 0]),
            Err(status) => syscall_error(status),
        }
    }
    if Operation::from_raw(request.operation) == Some(Operation::ClockNow) {
        if request.arguments != [0; 6] {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        return syscall_success([time::monotonic_now_us(), 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::RealtimeNow) {
        if request.arguments != [0; 6] {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let Some(now_ns) = time::realtime_now_ns() else {
            return syscall_error(Status::BUSY)
        };
        return syscall_success([now_ns, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::SleepUntil) {
        if request.arguments[1..] != [0; 5] {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        return syscall_success([request.arguments[0], 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::RandomGet) {
        let address = request.arguments[0];
        let length = request.arguments[1];
        if request.arguments[2..] != [0; 4]
            || length == 0
            || length > random::MAX_REQUEST_BYTES as u64
            || !arch::paging::service_user_range(address, length, true)
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        let filled = arch::with_user_access(|| {
            let bytes = unsafe {
                core::slice::from_raw_parts_mut(address as *mut u8, length as usize)
            };
            random::fill(bytes)
        });
        if !filled {
            return syscall_error(Status::BUSY)
        }
        return syscall_success([length, 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::Yield) {
        if request.arguments != [0; 6] {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        return syscall_success([0; 4])
    }
    syscall_error(Status::INVALID_ARGUMENT)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn service_name(role: usize) -> Option<&'static str> {
    [
        None,
        Some("ghostos-init"),
        Some("ghostos-fsd"),
        Some("ghostos-storaged"),
        Some("ghostos-netd"),
        Some("ghostos-logd"),
        Some("ghostos-auditd"),
        Some("ghostos-authd"),
        Some("ghostos-pkgd"),
        Some("ghostos-shell"),
        Some("ghostos-pcid"),
        Some("ghostos-ahcid"),
        Some("ghostos-nvmed"),
        Some("ghostos-ethernetd"),
        Some("ghostos-logind"),
    ]
    .get(role)
    .copied()
    .flatten()
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
#[allow(unsafe_code)]
fn scheduler_clock() -> u64 {
    unsafe { (&*core::ptr::addr_of!(SCHEDULER)).assume_init_ref().clock() }
}

pub(crate) fn service_ready_mask() -> u32 {
    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    ))]
    {
        return SERVICE_READY.load(Ordering::Acquire)
    }
    #[cfg(not(all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    )))]
    0
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn syscall_success(values: [u64; 4]) -> Response {
    Response {
        status: Status::NORMAL.raw(),
        flags: 0,
        values,
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn syscall_error(status: Status) -> Response {
    Response {
        status: status.raw(),
        flags: 0,
        values: [0; 4],
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
#[allow(unsafe_code)]
fn boot_ghostos_init(
    frames: &mut EarlyFrameAllocator,
    scheduler: &'static mut Scheduler,
    physical_offset: u64,
    services: boot_services::BootServices,
    boot_info: &'static BootInfo,
    scheduler_clock: u64,
    acpi: Option<ghostos_power::AcpiPlatform>,
    pci_inventory: &pci::PciInventory,
) -> ! {
    unsafe {
        shell::initialize(
            boot_info,
            scheduler as *mut Scheduler,
            &DLM,
            &NODE_FENCES,
            scheduler_clock,
            acpi,
        )
    };
    let (init_thread, init_root) = boot_service_process(
        frames,
        scheduler,
        physical_offset,
        AddressSpaceId::new(1).expect("boot address space id"),
        false,
        1,
        pci_inventory,
    );
    let service_processes = [
        services.filesystem_process,
        services.storage_process,
        services.network_process,
        services.logging_process,
        services.audit_process,
        services.authentication_process,
        services.package_process,
        services.shell_process,
        services.pci_process,
        services.ahci_process,
        services.nvme_process,
        services.ethernet_process,
        services.login_process,
    ];
    for (address_space_raw, process) in (2u32..=14).zip(service_processes) {
        let address_space = AddressSpaceId::new(address_space_raw)
            .expect("boot service address space id");
        let (thread, _) = boot_service_process(
            frames,
            scheduler,
            physical_offset,
            address_space,
            address_space_raw == 9,
            address_space_raw as u8,
            pci_inventory,
        );
        println!(
            "starting service entrypoint (process={}, thread={}, address space={})",
            process.raw(),
            thread.raw(),
            address_space.raw()
        );
    }

    let Some(_) = scheduler.dispatch() else {
        fatal_kernel_halt(Status::BUSY)
    };
    if syscall::install_dispatcher(boot_init_dispatch).is_err() {
        fatal_kernel_halt(Status::BUSY)
    }
    let context = scheduler
        .thread(init_thread)
        .map(|thread| thread.context)
        .unwrap_or_else(|_| {
            fatal_kernel_halt(Status::CORRUPT)
        });
    println!(
        "starting ghostos-init in Ring 3 (thread={}, services=13)",
        init_thread.raw()
    );
    boot_diagnostics::checkpoint(boot_diagnostics::BootStage::UserHandoff);
    boot_diagnostics::complete();
    arch::enter_user(&context, init_root)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
#[allow(unsafe_code)]
fn boot_service_process(
    frames: &mut EarlyFrameAllocator,
    scheduler: &mut Scheduler,
    physical_offset: u64,
    address_space: AddressSpaceId,
    shell: bool,
    role: u8,
    pci_inventory: &pci::PciInventory,
) -> (ThreadId, PageTableRoot) {
    let mut tables = [0u64; arch::paging::PROCESS_TABLE_FRAME_COUNT];
    for frame in &mut tables {
        let Ok(address) = frames.allocate_for_owner(address_space) else {
            fatal_kernel_halt(Status::NO_SPACE)
        };
        *frame = address;
    }
    let mut pages = [0u64; arch::paging::SERVICE_PAGE_COUNT];
    for page in &mut pages {
        let Ok(address) = frames.allocate_for_owner(address_space) else {
            fatal_kernel_halt(Status::NO_SPACE)
        };
        *page = address;
    }

    if role as usize >= 15 {
        fatal_kernel_halt(Status::INVALID_ARGUMENT)
    }
    let capabilities = unsafe {
        let slots = core::ptr::addr_of_mut!(BOOT_SERVICE_CAPABILITIES)
            as *mut MaybeUninit<CapabilitySpace<MAX_CAPABILITIES>>;
        (*slots.add(role as usize)).write(CapabilitySpace::new());
        &mut *slots.add(role as usize).cast::<CapabilitySpace<MAX_CAPABILITIES>>()
    };
    let authority = capabilities
        .mint_root(
            AddressSpaceId::KERNEL,
            CapabilityObject::AddressSpace(address_space),
            Rights::ALL,
        )
        .expect("boot address-space capability");
    let driver_grant = driver_capabilities::grant(
        capabilities,
        address_space,
        role,
        pci_inventory,
    )
    .unwrap_or_else(|error| fatal_kernel_halt(error.status()));

    let Some(root) = (unsafe {
        arch::paging::install_service_root(
            &tables,
            &pages,
            physical_offset,
            driver_grant.mmio_mappings(),
        )
    }) else {
        fatal_kernel_halt(Status::INVALID_ARGUMENT)
    };
    unsafe {
        arch::paging::write_service_image(
            &pages,
            physical_offset,
            address_space.raw() as u8,
            shell,
            physical_storage::service_image(role),
        );
        arch::paging::write_service_resources(
            &pages,
            physical_offset,
            &driver_grant.manifest,
        );
    };
    let thread = scheduler
        .create_user(
            capabilities,
            AddressSpaceId::KERNEL,
            authority,
            address_space,
            root,
            SchedulingPolicy::Cooperative,
            arch::paging::service_entry() as usize,
            arch::paging::service_stack_top() as usize,
        )
        .unwrap_or_else(|_| fatal_kernel_halt(Status::CORRUPT));
    (thread, root)
}

pub fn halt() -> ! {
    loop {
        arch::halt()
    }
}

#[allow(unsafe_code)]
pub(crate) fn cpu_idle() {
    if !SCHEDULER_READY.load(Ordering::Acquire) {
        arch::halt();
        return
    }
    let state = unsafe {
        let scheduler = (&mut *core::ptr::addr_of_mut!(SCHEDULER)).assume_init_mut();
        let cpu = arch::interrupts::current_cpu();
        let now = scheduler.clock();
        scheduler.idle_state_on(cpu, now.saturating_add(1_000), 1_000)
    };
    arch::idle(state)
}

/// Validate bootloader data before any operation can use its variable-length
/// regions. Invalid metadata is an external input failure, not a panic.
pub fn validate_boot_info(boot_info: &BootInfo) -> Result<(), Status> {
    boot_info.is_valid().then_some(()).ok_or(Status::INVALID_ARGUMENT)
}

/// The only deliberate non-returning failure boundary in kernel code.
pub fn fatal_kernel_halt(status: Status) -> ! {
    boot_diagnostics::fail(status);
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
    boot_diagnostics::fail(status);
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
