#![cfg_attr(target_os = "ghostos", no_std)]
#![cfg_attr(target_os = "ghostos", no_main)]

include!(concat!(env!("OUT_DIR"), "/build-script-output.rs"));

acceptance_proc_macro::acceptance_marker!();

#[cfg(not(target_os = "ghostos"))]
fn main() {
    assert!(BUILD_SCRIPT_RAN);
    assert!(PROC_MACRO_RAN);
}

#[cfg(target_os = "ghostos")]
use core::panic::PanicInfo;

#[cfg(target_os = "ghostos")]
#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {}
}

#[cfg(target_os = "ghostos")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    loop {}
}
