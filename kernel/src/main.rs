#![cfg_attr(any(target_os = "none", target_os = "uefi"), no_std)]
#![cfg_attr(any(target_os = "none", target_os = "uefi"), no_main)]

#[cfg(any(target_os = "none", target_os = "uefi"))]
use core::panic::PanicInfo;
#[cfg(any(target_os = "none", target_os = "uefi"))]
unsafe extern "C" {
    fn ghostos_main_panic(panic_handler: extern "C" fn(*const core::ffi::c_void), context: *const core::ffi::c_void) -> !;
}

#[cfg(any(target_os = "none", target_os = "uefi"))]
extern "C" fn panic_bridge(context: *const core::ffi::c_void) {
    let info = unsafe { &*(context.cast::<PanicInfo<'_>>()) };
    ghostos_kernel::panic_report(info)
}

#[cfg(any(target_os = "none", target_os = "uefi"))]
#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    unsafe { ghostos_main_panic(panic_bridge, (info as *const PanicInfo<'_>).cast()) }
}

#[cfg(not(any(target_os = "none", target_os = "uefi")))]
fn main() {}
