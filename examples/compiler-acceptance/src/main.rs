#![cfg_attr(target_os = "synos", no_std)]
#![cfg_attr(target_os = "synos", no_main)]

include!(concat!(env!("OUT_DIR"), "/build-script-output.rs"));

acceptance_proc_macro::acceptance_marker!();

#[cfg(not(target_os = "synos"))]
fn main() {
    assert!(BUILD_SCRIPT_RAN);
    assert!(PROC_MACRO_RAN);
}

#[cfg(target_os = "synos")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    loop {}
}
