#![no_std]
#![no_main]

use core::panic::PanicInfo;
use synos_boot_protocol::BootInfo;

#[unsafe(no_mangle)]
#[cfg_attr(
    any(target_os = "none", target_os = "uefi"),
    unsafe(link_section = ".text._start")
)]
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    synos_kernel::kernel_entry(boot_info)
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    synos_kernel::panic_report(info)
}
