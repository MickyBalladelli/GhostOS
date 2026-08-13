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
pub mod cow;
pub mod dma;
#[allow(unsafe_code)]
#[allow(dead_code)]
mod driver_capabilities;
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
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub mod mouse;
#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
#[path = "mouse_stub.rs"]
pub mod mouse;
#[allow(unsafe_code)]
pub mod page_fault;
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
pub mod partition;
pub mod process;
pub mod scheduler;
pub mod runtime;
pub mod saturation;
#[allow(unsafe_code)]
pub mod random;
#[allow(unsafe_code)]
pub mod syscall;
#[allow(unsafe_code)]
#[allow(dead_code)]
mod shell;
pub mod task;
#[allow(unsafe_code)]
pub mod time;
pub mod monitor;
pub mod tlb;
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
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
use core::sync::atomic::{AtomicU32, AtomicU64};
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
    AllocationError, EarlyFrameAllocator, QuotaAllocationError, ReclaimError,
    MAX_OWNED_FRAME_RANGES, FRAME_SIZE,
};
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
pub use synos_numa::{NumaCounters, NumaDecision, NumaPlacement, NumaReport, NumaTopology, NumaTopologyError, PlacementKind, PlacementLocality};
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
pub use synos_observability::{AffinitySet, ScalePath, ScalePolicy, SCALE_CPU_TIERS};
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
    [MaybeUninit<CapabilitySpace<MAX_CAPABILITIES>>; 14] =
    [const { MaybeUninit::uninit() }; 14];
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static SERVICE_READY: AtomicU32 = AtomicU32::new(0);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static SERVICE_HEARTBEATS: [AtomicU64; 14] = [const { AtomicU64::new(0) }; 14];
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
    let bootstrap_cpu = arch::interrupts::current_cpu();
    let mut cpu_topology = arch::cpu::CpuTopology::<{ task::MAX_CPUS }>::new();
    let _ = cpu_topology.add_bootstrap(bootstrap_cpu.raw() as u32);
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

    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    ))]
    if !arch::ring3_supported() {
        println!(
            "NX/XD unavailable; Ring 3 services disabled, entering kernel shell"
        );
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
    let boot_services = boot_services::start(physical_filesystem)
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
    boot_synos_init(
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
    shell::run(boot_info, scheduler, &DLM, &NODE_FENCES, scheduler_clock, acpi)
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
    if request.abi_version != synos_runtime::ABI_SCHEMA_VERSION || request.reserved != 0 {
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
                    | Operation::SynFsMap
                    | Operation::SynFsUnmap
                    | Operation::SynFsMkdir
                    | Operation::SynFsRmdir
                    | Operation::SynFsList
                    | Operation::SynFsDelete
            )
        {
            let address = request.arguments[0];
            let length = request.arguments[1];
            let writable = request.arguments[2];
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
                if request.flags & !synos_fsd::Flags::WRITE.bits() != 0
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
                || length > synos_fsd::MAX_IPC_BUFFER_BYTES as u64
                || !arch::paging::service_user_range(address, length, writable != 0)
            {
                return syscall_error(Status::INVALID_ARGUMENT)
            }
            return arch::with_user_access(|| {
                let bytes = unsafe {
                    core::slice::from_raw_parts_mut(address as *mut u8, length as usize)
                };
                boot_services::dispatch_shell_filesystem(
                    operation,
                    request.flags,
                    request.capability,
                    request.arguments[4],
                    0,
                    Some(bytes),
                )
            })
        }
    }
    if request.flags != 0 || request.capability != 0 {
        return syscall_error(Status::INVALID_ARGUMENT)
    }
    if Operation::from_raw(request.operation) == Some(Operation::TerminalWrite)
        && caller.raw() == 9
    {
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
        return Response {
            status: Status::NORMAL.raw(),
            flags: 0,
            values: [length as u64, 0, 0, 0],
        }
    }
    if Operation::from_raw(request.operation) == Some(Operation::TerminalRead)
        && caller.raw() == 9
    {
        let address = request.arguments[0] as usize;
        if request.arguments[1] != 1
            || request.arguments[2..] != [0; 4]
            || !arch::paging::service_user_range(address as u64, 1, true)
        {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if let Some(byte) = keyboard::read_boot_byte().or_else(console::read_byte) {
            unsafe { arch::write_user(address as *mut u8, byte) };
            return Response {
                status: Status::NORMAL.raw(),
                flags: 0,
                values: [1, 0, 0, 0],
            }
        }
        return Response {
            status: Status::NORMAL.raw(),
            flags: 0,
            values: [0; 4],
        }
    }
    if Operation::from_raw(request.operation) == Some(Operation::ServiceReady) {
        if request.arguments[0] as usize != role || request.arguments[1..] != [0; 5] {
            return syscall_error(Status::INVALID_ARGUMENT)
        }
        if role == 9 && SERVICE_READY.load(Ordering::Acquire) & (1u32 << 7) == 0 {
            return syscall_error(Status::BUSY)
        }
        let bit = 1u32 << role;
        let ready = SERVICE_READY.load(Ordering::Acquire);
        if ready & bit == 0 {
            SERVICE_READY.store(ready | bit, Ordering::Release);
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
        SERVICE_HEARTBEATS[role].store(request.arguments[1], Ordering::Release);
        return syscall_success([request.arguments[1], 0, 0, 0])
    }
    if Operation::from_raw(request.operation) == Some(Operation::SystemInfo) {
        if role != 9 || request.arguments != [0; 6] {
            return syscall_error(Status::ACCESS_DENIED)
        }
        let heartbeats = SERVICE_HEARTBEATS
            .iter()
            .skip(1)
            .fold(0u64, |total, value| total.saturating_add(value.load(Ordering::Acquire)));
        return syscall_success([
            scheduler_clock(),
            SERVICE_READY.load(Ordering::Acquire) as u64,
            heartbeats,
            role as u64,
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
        Some("synos-init"),
        Some("synos-fsd"),
        Some("synos-storaged"),
        Some("synos-netd"),
        Some("synos-logd"),
        Some("synos-auditd"),
        Some("synos-authd"),
        Some("synos-pkgd"),
        Some("synos-shell"),
        Some("synos-pcid"),
        Some("synos-ahcid"),
        Some("synos-nvmed"),
        Some("synos-ethernetd"),
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
fn boot_synos_init(
    frames: &mut EarlyFrameAllocator,
    scheduler: &'static mut Scheduler,
    physical_offset: u64,
    services: boot_services::BootServices,
    boot_info: &'static BootInfo,
    scheduler_clock: u64,
    acpi: Option<synos_power::AcpiPlatform>,
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
    ];
    for (address_space_raw, process) in (2u32..=13).zip(service_processes) {
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
        "starting synos-init in Ring 3 (thread={}, services=12)",
        init_thread.raw()
    );
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

    if role as usize >= 14 {
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
