#![no_std]
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

mod allocator;
#[allow(unsafe_code)]
mod arch;
pub mod capability;
#[allow(unsafe_code)]
mod console;
pub mod dlm;
pub mod ipc;
pub mod micro_silo;
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
pub mod scheduler;
pub mod runtime;
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
use synos_boot_protocol::BootInfo;
use synos_observability::{EventField, EventKind, field, info};

pub use allocator::{AllocationError, EarlyFrameAllocator, FRAME_SIZE};
pub use capability::{
    CapabilityError, CapabilityHandle, CapabilityInfo, CapabilityLinks, CapabilityObject,
    CapabilityRevocationHook, CapabilitySpace, MAX_CAPABILITIES, PhysicalRange, Rights,
};
pub use dlm::{
    DistributedLockManager, FederationClusterId, FederationFenceTable, LockError, LockGrant,
    LockHandle, LockMode, LockOwner, LockRange, NodeFenceState, NodeFenceTable, NodeFenceToken,
    NodeId, ResourceId, ResourceKind, ResourceName, DEFAULT_FEDERATION_CAPACITY,
    DEFAULT_NODE_FENCE_CAPACITY,
};
pub use micro_silo::{
    BlindMicroSilo, ConfidentialCpu, HardwareIsolation, MemoryProtection, SiloError,
    SiloMemoryRange, SiloObject, SiloOperation, MAX_SILO_MEMORY_RANGES,
};
pub use page_fault::{PageFault, PageFaultHandler, PageFaultHandlerError};
pub use persona::{
    ExecutionPersona, IdentityId, PersonaError, RightIdentifier, MAX_PERSONA_RIGHTS,
};
pub use scheduler::{ContextSwitch, Scheduler, SchedulerError};
pub use task::{
    AddressSpaceId, Context, ExecutionMode, SchedulingPolicy, Thread, ThreadId, ThreadState,
};

static SCHEDULER: Scheduler = Scheduler::new();
static DLM: DistributedLockManager = DistributedLockManager::new();
static NODE_FENCES: NodeFenceTable = NodeFenceTable::new();

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn kernel_entry(boot_info: &'static BootInfo) -> ! {
    console::init(boot_info.framebuffer);
    info!(
        EventKind::Boot,
        EventField::unsigned(field::OPERATION, 1),
    );
    println!("SynOS kernel bootstrap");

    if !boot_info.is_valid() {
        panic!("bootloader gave invalid BootInfo")
    }

    println!(
        "boot method={} memory regions={}",
        boot_info.method as u32,
        boot_info.memory_region_count
    );

    let mut frames = EarlyFrameAllocator::new(boot_info.regions());
    let mut page_tables = [0; arch::paging::TABLE_FRAME_COUNT];
    for frame in &mut page_tables {
        *frame = frames.allocate().expect("no frame for paging");
    }

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

    println!(
        "paging, interrupts, capabilities, IPC, and scheduler ready ({} capability slots, {} thread slots, clock={})",
        MAX_CAPABILITIES,
        task::MAX_THREADS,
        SCHEDULER.clock()
    );
    filesystem_boot_smoke_test();
    shell::run(boot_info, &SCHEDULER, &DLM, &NODE_FENCES, SCHEDULER.clock(), acpi)
}

fn filesystem_boot_smoke_test() {
    use synos_fsd::{Daemon, Flags, ProcessRights};
    use synos_synfs::SynFs;

    let process = synos_fsd::ProcessId::new(1).expect("valid boot process");
    let rights = ProcessRights::from_bits(ProcessRights::READ.bits() | ProcessRights::WRITE.bits());
    let mut daemon = Daemon::<32, 2, 8, 2, 8, 1024>::new(SynFs::new())
        .expect("root filesystem daemon starts");
    let authority = daemon
        .register_process(process, rights)
        .expect("boot application authority");
    let flags = Flags::READ.union(Flags::WRITE).union(Flags::CREATE);
    let file = daemon
        .open(process, authority, "/data/qemu-boot", flags)
        .expect("boot application opens root file");
    daemon
        .write(process, file.capability, 0, b"synos-root-io")
        .expect("boot application writes root file");
    let mut contents = [0; 13];
    let bytes = daemon
        .read(process, file.capability, 0, &mut contents)
        .expect("boot application reads root file");
    assert_eq!(&contents[..bytes], b"synos-root-io");
    assert!(daemon.namespace().is_active());
    println!(
        "SynFS root mounted generation={} application I/O validated",
        daemon.root_activation().generation
    );
}

pub fn halt() -> ! {
    loop {
        arch::halt()
    }
}

pub fn panic_report(info: &PanicInfo<'_>) -> ! {
    println!("KERNEL PANIC: {info}");
    halt()
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
