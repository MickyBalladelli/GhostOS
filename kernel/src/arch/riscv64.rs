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

    pub fn set_core_isolated(_cpu: u8, _isolated: bool) {}


    pub unsafe fn init() {
        unsafe {
            asm!("csrw stvec, {}", in(reg) trap_entry as *const () as usize, options(nostack));
        }
    }

    extern "C" fn trap_entry() {
        crate::capture_exception(super::capture_registers(0), 0, synos_status::Status::CORRUPT, 0);
        crate::println!("RISC-V supervisor trap");
        crate::halt()
    }
}

pub(crate) fn capture_registers(fault_address: u64) -> crate::crash::RegisterState {
    let (x1, x2, x3, x4, sp, ip, flags);
    unsafe {
        asm!("mv {}, x1", out(reg) x1, options(nomem, nostack, preserves_flags));
        asm!("mv {}, x2", out(reg) x2, options(nomem, nostack, preserves_flags));
        asm!("mv {}, x3", out(reg) x3, options(nomem, nostack, preserves_flags));
        asm!("mv {}, x4", out(reg) x4, options(nomem, nostack, preserves_flags));
        asm!("mv {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
        asm!("auipc {}, 0", out(reg) ip, options(nomem, nostack, preserves_flags));
        asm!("csrr {}, sstatus", out(reg) flags, options(nomem, nostack, preserves_flags));
    }
    crate::crash::RegisterState {
        general: [x1, x2, x3, x4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        instruction_pointer: ip,
        stack_pointer: sp,
        flags,
        fault_address,
    }
}

#[inline(always)]
pub fn halt() {
    unsafe {
        asm!("wfi", options(nomem, nostack));
    }
}
