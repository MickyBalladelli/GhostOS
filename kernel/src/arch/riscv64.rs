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

    /// Switch SATP to a live user address-space root.
    ///
    /// # Safety
    /// The root must contain the kernel mappings needed by trap return.
    pub unsafe fn activate_root(root: u64) {
        unsafe {
            let satp = SV39 | (root >> 12);
            asm!("csrw satp, {}", in(reg) satp, options(nostack));
            invalidate_all();
        }
    }

    /// Invalidate one page in the current address space.
    ///
    /// # Safety
    /// The caller must have completed the page-table update before calling.
    pub unsafe fn invalidate_page(address: u64) {
        unsafe {
            asm!("sfence.vma {}, zero", in(reg) address, options(nostack));
        }
    }

    /// Invalidate every translation on this logical CPU.
    ///
    /// # Safety
    /// The caller must have completed the page-table update before calling.
    pub unsafe fn invalidate_all() {
        unsafe { asm!("sfence.vma zero, zero", options(nostack)) }
    }

    /// Invalidate all pages in an aligned range on this logical CPU.
    ///
    /// # Safety
    /// The caller must have completed the page-table update before calling.
    pub unsafe fn invalidate_range(start: u64, length: u64) {
        let Some(end) = start.checked_add(length) else { return };
        let mut address = start;
        while address < end {
            unsafe { invalidate_page(address) }
            address = address.saturating_add(crate::FRAME_SIZE);
        }
    }
}

pub mod interrupts {
    use core::arch::{asm, global_asm};
    use core::sync::atomic::{AtomicU64, Ordering};

    static ISOLATED_CORES: AtomicU64 = AtomicU64::new(0);

    pub fn set_core_isolated(cpu: u8, isolated: bool) {
        if cpu >= 64 {
            return
        }
        let bit = 1u64 << cpu;
        if isolated {
            ISOLATED_CORES.fetch_or(bit, Ordering::Release);
        } else {
            ISOLATED_CORES.fetch_and(!bit, Ordering::Release);
        }
    }

    pub fn disable() {
        unsafe { asm!("csrc sstatus, {0}", in(reg) (1 << 1), options(nomem, nostack)) }
    }

    unsafe extern "C" {
        fn trap_entry();
    }

    pub unsafe fn init() {
        unsafe {
            asm!("csrw stvec, {}", in(reg) trap_entry as *const () as usize, options(nostack));
        }
    }

    pub fn current_cpu() -> crate::task::CpuId {
        let id: usize;
        unsafe { asm!("csrr {}, mhartid", out(reg) id, options(nomem, nostack)) }
        crate::task::CpuId::new((id & 0x7f) as u8)
            .unwrap_or(crate::task::CpuId::new(0).expect("CPU 0 is valid"))
    }

    pub fn send_ipi(_target: crate::task::CpuId, _vector: u8) -> bool {
        false
    }

    pub fn end_of_interrupt() {}

    #[repr(C)]
    struct TrapFrame {
        registers: [u64; 32],
        sepc: u64,
        sstatus: u64,
        scause: u64,
        stval: u64,
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn synos_riscv_trap_dispatch(frame: *mut TrapFrame) {
        let frame = unsafe { &mut *frame };
        let is_interrupt = frame.scause >> 63 != 0;
        let cause = frame.scause & !(1u64 << 63);
        if !is_interrupt && cause == 8 {
            crate::syscall::synos_call_gate_dispatch(
                frame.registers[10] as *const synos_runtime::Request,
                frame.registers[11] as *mut synos_runtime::Response,
            );
            frame.sepc = frame.sepc.saturating_add(4);
            return
        }
        crate::capture_exception(
            super::capture_registers(frame.stval),
            frame.stval,
            synos_status::Status::CORRUPT,
            cause as u16,
        );
        crate::println!("RISC-V supervisor trap cause={cause:#x}");
        crate::halt()
    }

    global_asm!(
        r#"
.section .text
.balign 4
.global trap_entry
trap_entry:
    addi sp, sp, -288
    sd x0, 0(sp)
    sd x1, 8(sp)
    sd x2, 16(sp)
    sd x3, 24(sp)
    sd x4, 32(sp)
    sd x5, 40(sp)
    sd x6, 48(sp)
    sd x7, 56(sp)
    sd x8, 64(sp)
    sd x9, 72(sp)
    sd x10, 80(sp)
    sd x11, 88(sp)
    sd x12, 96(sp)
    sd x13, 104(sp)
    sd x14, 112(sp)
    sd x15, 120(sp)
    sd x16, 128(sp)
    sd x17, 136(sp)
    sd x18, 144(sp)
    sd x19, 152(sp)
    sd x20, 160(sp)
    sd x21, 168(sp)
    sd x22, 176(sp)
    sd x23, 184(sp)
    sd x24, 192(sp)
    sd x25, 200(sp)
    sd x26, 208(sp)
    sd x27, 216(sp)
    sd x28, 224(sp)
    sd x29, 232(sp)
    sd x30, 240(sp)
    sd x31, 248(sp)
    csrr t0, sepc
    sd t0, 256(sp)
    csrr t0, sstatus
    sd t0, 264(sp)
    csrr t0, scause
    sd t0, 272(sp)
    csrr t0, stval
    sd t0, 280(sp)
    mv a0, sp
    call synos_riscv_trap_dispatch
    ld t0, 256(sp)
    csrw sepc, t0
    ld t0, 264(sp)
    csrw sstatus, t0
    ld x1, 8(sp)
    ld x2, 16(sp)
    ld x3, 24(sp)
    ld x4, 32(sp)
    ld x5, 40(sp)
    ld x6, 48(sp)
    ld x7, 56(sp)
    ld x8, 64(sp)
    ld x9, 72(sp)
    ld x10, 80(sp)
    ld x11, 88(sp)
    ld x12, 96(sp)
    ld x13, 104(sp)
    ld x14, 112(sp)
    ld x15, 120(sp)
    ld x16, 128(sp)
    ld x17, 136(sp)
    ld x18, 144(sp)
    ld x19, 152(sp)
    ld x20, 160(sp)
    ld x21, 168(sp)
    ld x22, 176(sp)
    ld x23, 184(sp)
    ld x24, 192(sp)
    ld x25, 200(sp)
    ld x26, 208(sp)
    ld x27, 216(sp)
    ld x28, 224(sp)
    ld x29, 232(sp)
    ld x30, 240(sp)
    ld x31, 248(sp)
    addi sp, sp, 288
    sret
"#
    );
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

pub(crate) fn enter_user(context: &crate::Context, root: crate::PageTableRoot) -> ! {
    unsafe {
        paging::activate_root(root.frame());
        let mut status: u64;
        asm!("csrr {}, sstatus", out(reg) status, options(nomem, nostack));
        status &= !(1 << 8);
        status |= 1 << 5;
        asm!("csrw sepc, {}", in(reg) context.instruction_pointer, options(nostack));
        asm!("csrw sstatus, {}", in(reg) status, options(nostack));
        asm!("mv sp, {}", in(reg) context.stack_pointer, options(nostack));
        asm!("sret", options(noreturn));
    }
}

pub(crate) fn idle(_state: synos_power::CpuIdleState) {
    unsafe {
        asm!("wfi", options(nomem, nostack));
    }
}

#[inline(always)]
pub fn halt() {
    unsafe {
        asm!("wfi", options(nomem, nostack));
    }
}
