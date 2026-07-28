use core::arch::asm;

pub mod paging {
    use core::arch::asm;

    pub const TABLE_FRAME_COUNT: usize = 6;
    const ENTRY_COUNT: usize = 512;
    const VALID: u64 = 1;
    const READ: u64 = 1 << 1;
    const WRITE: u64 = 1 << 2;
    const EXECUTE: u64 = 1 << 3;
    const ACCESSED: u64 = 1 << 6;
    const DIRTY: u64 = 1 << 7;
    const SV39: u64 = 8 << 60;

    /// Installs an Sv39 root with the first GiB identity mapped.
    ///
    /// # Safety
    /// `root` must be an aligned writable physical frame in supervisor mode.
    pub unsafe fn install_root(
        tables: &[u64; TABLE_FRAME_COUNT],
        physical_offset: u64,
    ) {
        let root = tables[0];
        let table = (root + physical_offset) as *mut u64;
        unsafe {
            core::ptr::write_bytes(table, 0, ENTRY_COUNT);
            for index in 0..4 {
                let physical_page_number =
                    (index as u64 * 1024 * 1024 * 1024) >> 12;
                table.add(index).write(
                    (physical_page_number << 10)
                        | VALID
                        | READ
                        | WRITE
                        | EXECUTE
                        | ACCESSED
                        | DIRTY,
                );
            }

            let satp = SV39 | (root >> 12);
            asm!("csrw satp, {}", in(reg) satp, options(nostack));
            asm!("sfence.vma zero, zero", options(nostack));
        }
    }
}

pub mod interrupts {
    use core::arch::asm;

    pub unsafe fn init() {
        unsafe {
            asm!("csrw stvec, {}", in(reg) trap_entry as *const () as usize, options(nostack));
        }
    }

    pub unsafe fn enable() {
        const SUPERVISOR_INTERRUPT_ENABLE: usize = 1 << 1;
        unsafe {
            asm!("csrs sstatus, {}", in(reg) SUPERVISOR_INTERRUPT_ENABLE, options(nostack));
        }
    }

    extern "C" fn trap_entry() {
        crate::println!("RISC-V supervisor trap");
        crate::halt()
    }
}

#[inline(always)]
pub fn halt() {
    unsafe {
        asm!("wfi", options(nomem, nostack));
    }
}
