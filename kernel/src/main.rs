#![no_std]
#![no_main]

use synos_boot_protocol::BootInfo;

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    synos_kernel::kernel_entry(boot_info)
}

