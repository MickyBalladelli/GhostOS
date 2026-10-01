#![cfg_attr(any(target_os = "none", target_os = "uefi"), no_std)]
#![cfg_attr(any(target_os = "none", target_os = "uefi"), no_main)]

#[cfg(any(target_os = "none", target_os = "uefi"))]
use core::panic::PanicInfo;
#[cfg(any(target_os = "none", target_os = "uefi"))]
use ghostos_boot_protocol::BootInfo;

#[cfg(any(target_os = "none", target_os = "uefi"))]
unsafe extern "C" {
    fn ghostos_main_enter(boot_info: *const BootInfo, entry: extern "C" fn(&'static BootInfo)) -> !;
    fn ghostos_main_panic(panic_handler: extern "C" fn(*const core::ffi::c_void), context: *const core::ffi::c_void) -> !;
}

#[cfg(any(target_os = "none", target_os = "uefi"))]
extern "C" fn kernel_entry_bridge(boot_info: &'static BootInfo) {
    ghostos_kernel::kernel_entry(boot_info)
}

#[cfg(any(target_os = "none", target_os = "uefi"))]
extern "C" fn panic_bridge(context: *const core::ffi::c_void) {
    let info = unsafe { &*(context.cast::<PanicInfo<'_>>()) };
    ghostos_kernel::panic_report(info)
}

#[cfg(any(target_os = "none", target_os = "uefi"))]
#[unsafe(no_mangle)]
#[cfg_attr(
    any(target_os = "none", target_os = "uefi"),
    unsafe(link_section = ".text._start")
)]
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    unsafe { ghostos_main_enter(boot_info, kernel_entry_bridge) }
}

#[cfg(any(target_os = "none", target_os = "uefi"))]
#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    unsafe { ghostos_main_panic(panic_bridge, (info as *const PanicInfo<'_>).cast()) }
}

#[cfg(not(any(target_os = "none", target_os = "uefi")))]
fn main() {}
