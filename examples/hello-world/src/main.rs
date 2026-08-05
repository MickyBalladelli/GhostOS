#![cfg_attr(target_os = "synos", no_std)]
#![cfg_attr(target_os = "synos", no_main)]

#[cfg(not(target_os = "synos"))]
fn main() {
    println!("Hello World from SynOS")
}

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
    loop {}
}
