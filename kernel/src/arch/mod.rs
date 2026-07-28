#[cfg(target_arch = "aarch64")]
#[path = "aarch64.rs"]
mod current;
#[cfg(target_arch = "riscv64")]
#[path = "riscv64.rs"]
mod current;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64.rs"]
mod current;
#[cfg(not(any(
    target_arch = "aarch64",
    target_arch = "riscv64",
    target_arch = "x86_64"
)))]
#[path = "unsupported.rs"]
mod current;

pub use current::{halt, interrupts, paging};

