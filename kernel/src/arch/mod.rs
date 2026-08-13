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

pub(crate) use current::enter_user;

pub(crate) fn disable_interrupts() {
    current::interrupts::disable()
}

pub(crate) fn invalidate_tlb_range(start: u64, length: u64) {
    // The caller validates alignment and overflow before reaching the
    // architecture backend.
    unsafe { paging::invalidate_range(start, length) }
}

pub(crate) fn with_user_access<R>(operation: impl FnOnce() -> R) -> R {
    #[cfg(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi")))]
    {
        return current::with_user_access(operation)
    }
    #[cfg(not(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi"))))]
    operation()
}

pub(crate) unsafe fn read_user<T: Copy>(pointer: *const T) -> T {
    with_user_access(|| unsafe { pointer.read() })
}

pub(crate) unsafe fn write_user<T>(pointer: *mut T, value: T) {
    with_user_access(|| unsafe { pointer.write(value) })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArchitectureEvidence {
    pub name: &'static str,
    pub smp: bool,
    pub interrupts: bool,
    pub user_mode: bool,
    pub isolation: bool,
}

#[cfg(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi")))]
pub const fn evidence() -> ArchitectureEvidence {
    ArchitectureEvidence {
        name: "x86_64",
        smp: true,
        interrupts: true,
        user_mode: true,
        isolation: true,
    }
}

#[cfg(all(target_arch = "aarch64", any(target_os = "none", target_os = "uefi")))]
pub const fn evidence() -> ArchitectureEvidence {
    ArchitectureEvidence {
        name: "aarch64",
        smp: false,
        interrupts: true,
        user_mode: true,
        isolation: true,
    }
}

#[cfg(all(target_arch = "riscv64", any(target_os = "none", target_os = "uefi")))]
pub const fn evidence() -> ArchitectureEvidence {
    ArchitectureEvidence {
        name: "riscv64",
        smp: false,
        interrupts: true,
        user_mode: true,
        isolation: true,
    }
}

#[cfg(not(any(
    all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi")),
    all(target_arch = "aarch64", any(target_os = "none", target_os = "uefi")),
    all(target_arch = "riscv64", any(target_os = "none", target_os = "uefi")),
)))]
pub const fn evidence() -> ArchitectureEvidence {
    ArchitectureEvidence {
        name: "unsupported",
        smp: false,
        interrupts: false,
        user_mode: false,
        isolation: false,
    }
}

pub(crate) fn initialize(tables: &[u64; paging::TABLE_FRAME_COUNT], physical_offset: u64) {
    crate::invariants::debug_assert_valid(crate::invariants::check_page_table_transition(
        tables,
        physical_offset,
    ));
    // Safety: kernel_entry supplies distinct frames owned by the boot allocator.
    unsafe {
        paging::install_root(tables, physical_offset);
        #[cfg(all(target_arch = "x86_64", any(target_os = "none", target_os = "uefi")))]
        current::enable_supervisor_protections();
        interrupts::init();
    }
}
pub mod cpu;

#[allow(dead_code)]
pub const RESCHEDULE_IPI_VECTOR: u8 = 0xf0;
pub const TLB_SHOOTDOWN_IPI_VECTOR: u8 = 0xf1;
