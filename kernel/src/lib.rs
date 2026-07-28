#![no_std]
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

mod allocator;
#[allow(unsafe_code)]
mod arch;
pub mod capability;
#[allow(unsafe_code)]
mod console;
pub mod ipc;
pub mod scheduler;
pub mod task;

use core::panic::PanicInfo;
use synos_boot_protocol::BootInfo;

pub use allocator::{AllocationError, EarlyFrameAllocator, FRAME_SIZE};
pub use capability::{
    CapabilityError, CapabilityHandle, CapabilityInfo, CapabilityObject, CapabilitySpace,
    MAX_CAPABILITIES, Rights,
};
pub use scheduler::{ContextSwitch, Scheduler, SchedulerError};
pub use task::{
    AddressSpaceId, Context, ExecutionMode, SchedulingPolicy, Thread, ThreadId, ThreadState,
};

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn kernel_entry(boot_info: &'static BootInfo) -> ! {
    console::init(boot_info.framebuffer);
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

    let scheduler = Scheduler::new();

    println!(
        "paging, interrupts, capabilities, IPC, and scheduler ready ({} capability slots, {} thread slots, clock={})",
        MAX_CAPABILITIES,
        task::MAX_THREADS,
        scheduler.clock()
    );
    halt()
}

pub fn halt() -> ! {
    loop {
        arch::halt()
    }
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
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
