#![cfg_attr(target_os = "ghostos", no_std)]
#![cfg_attr(target_os = "ghostos", no_main)]

#[cfg(not(target_os = "ghostos"))]
fn main() {
    println!("Hello World from GhostOS")
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
