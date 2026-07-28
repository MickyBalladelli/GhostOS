#![no_std]

mod allocator;
mod arch;
mod console;

use core::panic::PanicInfo;
use synos_boot_protocol::BootInfo;

pub use allocator::{AllocationError, EarlyFrameAllocator, FRAME_SIZE};

#[unsafe(no_mangle)]
pub extern "C" fn kernel_entry(boot_info: &'static BootInfo) -> ! {
    console::init();
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

    // Safety: the bootloader owns the machine and supplied usable physical memory.
    unsafe {
        arch::paging::install_root(&page_tables, boot_info.physical_address_offset);
        arch::interrupts::init();
        arch::interrupts::enable();
    }

    println!("paging and interrupts ready");
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
