use core::arch::asm;

pub mod paging {
    use core::arch::asm;

    pub const TABLE_FRAME_COUNT: usize = 6;
    const ENTRY_COUNT: usize = 512;
    const TABLE: u64 = 0b11;
    const BLOCK: u64 = 0b01;
    const ACCESS_FLAG: u64 = 1 << 10;
    const INNER_SHAREABLE: u64 = 0b11 << 8;

    /// Installs a 4 KiB-granule TTBR0_EL1 table mapping the first GiB.
    ///
    /// # Safety
    /// The addresses must be distinct writable physical frames at EL1.
    pub unsafe fn install_root(
        tables: &[u64; TABLE_FRAME_COUNT],
        physical_offset: u64,
    ) {
        let root = tables[0];
        let level_one = tables[1];
        let l0 = (root + physical_offset) as *mut u64;
        let l1 = (level_one + physical_offset) as *mut u64;

        unsafe {
            core::ptr::write_bytes(l0, 0, ENTRY_COUNT);
            core::ptr::write_bytes(l1, 0, ENTRY_COUNT);
            l0.write(level_one | TABLE);
            for index in 0..4 {
                l1.add(index).write(
                    index as u64 * 1024 * 1024 * 1024
                        | BLOCK
                        | ACCESS_FLAG
                        | INNER_SHAREABLE,
                );
            }

            let mair: u64 = 0xff;
            let tcr: u64 = (16 << 0) | (0b00 << 14) | (0b11 << 12) | (0b01 << 10);
            asm!("msr mair_el1, {}", in(reg) mair, options(nostack));
            asm!("msr tcr_el1, {}", in(reg) tcr, options(nostack));
            asm!("isb", options(nostack));
            asm!("msr ttbr0_el1, {}", in(reg) root, options(nostack));
            asm!("dsb ish", "isb", options(nostack));
            let mut sctlr: u64;
            asm!("mrs {}, sctlr_el1", out(reg) sctlr, options(nostack));
            sctlr |= (1 << 0) | (1 << 2) | (1 << 12);
            asm!("msr sctlr_el1, {}", in(reg) sctlr, options(nostack));
            asm!("isb", options(nostack));
        }
    }

    /// Switch TTBR0_EL1 to a live user address-space root.
    ///
    /// # Safety
    /// The root must contain the kernel mappings needed by exception return.
    pub unsafe fn activate_root(root: u64) {
        unsafe {
            asm!("msr ttbr0_el1, {}", in(reg) root, options(nostack));
            asm!("isb", options(nostack));
        }
    }
}

pub mod interrupts {
    use core::arch::{asm, global_asm};

    pub fn set_core_isolated(_cpu: u8, _isolated: bool) {}


    unsafe extern "C" {
        static synos_aarch64_vectors: u8;
    }

    pub unsafe fn init() {
        unsafe {
            let address = (&raw const synos_aarch64_vectors) as usize;
            asm!("msr vbar_el1, {}", in(reg) address, options(nostack));
            asm!("isb", options(nostack));
        }
    }

    #[unsafe(no_mangle)]
    extern "C" fn synos_aarch64_exception() -> ! {
        crate::capture_exception(super::capture_registers(0), 0, synos_status::Status::CORRUPT, 0);
        crate::println!("AArch64 exception");
        crate::halt()
    }

    global_asm!(
        r#"
.section .text
.balign 2048
.global synos_aarch64_vectors
synos_aarch64_vectors:
.rept 16
    b synos_aarch64_exception
    .space 124
.endr
"#
    );
}

pub(crate) fn capture_registers(fault_address: u64) -> crate::crash::RegisterState {
    let (x0, x1, x2, x3, sp, ip, flags);
    unsafe {
        asm!("mov {}, x0", out(reg) x0, options(nomem, nostack, preserves_flags));
        asm!("mov {}, x1", out(reg) x1, options(nomem, nostack, preserves_flags));
        asm!("mov {}, x2", out(reg) x2, options(nomem, nostack, preserves_flags));
        asm!("mov {}, x3", out(reg) x3, options(nomem, nostack, preserves_flags));
        asm!("mov {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
        asm!("adr {}, .", out(reg) ip, options(nomem, nostack, preserves_flags));
        asm!("mrs {}, spsr_el1", out(reg) flags, options(nomem, nostack, preserves_flags));
    }
    crate::crash::RegisterState {
        general: [x0, x1, x2, x3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        instruction_pointer: ip,
        stack_pointer: sp,
        flags,
        fault_address,
    }
}

pub(crate) fn enter_user(context: &crate::Context, root: crate::PageTableRoot) -> ! {
    unsafe {
        paging::activate_root(root.frame());
        asm!(
            "msr sp_el0, {stack}",
            "msr elr_el1, {entry}",
            "msr spsr_el1, xzr",
            "isb",
            "eret",
            stack = in(reg) context.stack_pointer,
            entry = in(reg) context.instruction_pointer,
            options(noreturn),
        )
    }
}

#[inline(always)]
pub fn halt() {
    unsafe {
        asm!("wfi", options(nomem, nostack));
    }
}
