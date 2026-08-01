#![cfg_attr(any(target_os = "none", target_os = "uefi"), no_std)]
#![cfg_attr(any(target_os = "none", target_os = "uefi"), no_main)]

#[cfg(any(target_os = "none", target_os = "uefi"))]
use core::panic::PanicInfo;
#[cfg(any(target_os = "none", target_os = "uefi"))]
use synos_boot_protocol::BootInfo;

#[cfg(any(target_os = "none", target_os = "uefi"))]
#[unsafe(no_mangle)]
#[cfg_attr(
    any(target_os = "none", target_os = "uefi"),
    unsafe(link_section = ".text._start")
)]
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    synos_kernel::kernel_entry(boot_info)
}

#[cfg(any(target_os = "none", target_os = "uefi"))]
#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    synos_kernel::panic_report(info)
}

#[cfg(not(any(target_os = "none", target_os = "uefi")))]
fn main() {}
