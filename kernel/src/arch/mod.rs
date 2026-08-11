#[cfg(all(
    target_arch = "aarch64",
    any(target_os = "none", target_os = "uefi")
))]
#[path = "aarch64.rs"]
mod current;
#[cfg(all(
    target_arch = "riscv64",
    any(target_os = "none", target_os = "uefi")
))]
#[path = "riscv64.rs"]
mod current;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
#[path = "x86_64.rs"]
mod current;
#[cfg(not(any(
    all(
        target_arch = "aarch64",
        any(target_os = "none", target_os = "uefi")
    ),
    all(
        target_arch = "riscv64",
        any(target_os = "none", target_os = "uefi")
    ),
    all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    )
)))]
#[path = "unsupported.rs"]
mod current;

pub use current::{halt, interrupts, paging};

pub(crate) fn initialize(tables: &[u64; paging::TABLE_FRAME_COUNT], physical_offset: u64) {
    crate::invariants::debug_assert_valid(crate::invariants::check_page_table_transition(
        tables,
        physical_offset,
    ));
    // Safety: kernel_entry supplies distinct frames owned by the boot allocator.
    unsafe {
        paging::install_root(tables, physical_offset);
        interrupts::init();
    }
}
