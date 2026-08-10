#![cfg_attr(target_os = "synos", no_std)]
#![cfg_attr(target_os = "synos", no_main)]

#[cfg(not(target_os = "synos"))]
fn main() {}

#[cfg(target_os = "synos")]
use core::panic::PanicInfo;

#[cfg(target_os = "synos")]
#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {}
}

#[cfg(target_os = "synos")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let _ = synos_runtime::PANIC_MODEL;
    let _ = synos_runtime::DynamicLoadingPolicy::StaticOnly;
    loop {}
}
