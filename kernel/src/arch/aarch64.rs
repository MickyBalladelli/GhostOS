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
            invalidate_all();
            asm!("isb", options(nostack));
        }
    }

    /// Invalidate one page in the current TTBR0_EL1 address space.
    ///
    /// # Safety
    /// The caller must have completed the page-table update before calling.
    pub unsafe fn invalidate_page(address: u64) {
        let page = address >> 12;
        unsafe {
            asm!(
                "dsb ishst",
                "tlbi vae1is, {}",
                "dsb ish",
                "isb",
                in(reg) page,
                options(nostack)
            );
        }
    }

    /// Invalidate every EL1 translation on this logical CPU cluster.
    ///
    /// # Safety
    /// The caller must have completed the page-table update before calling.
    pub unsafe fn invalidate_all() {
        unsafe {
            asm!(
                "dsb ishst",
                "tlbi vmalle1is",
                "dsb ish",
                "isb",
                options(nostack)
            );
        }
    }

    /// Invalidate all pages in an aligned range on this logical CPU cluster.
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

    const MAX_CPU_MASK_BITS: u8 = 64;
    static ISOLATED_CORES: AtomicU64 = AtomicU64::new(0);

    pub fn set_core_isolated(cpu: u8, isolated: bool) {
        if cpu >= MAX_CPU_MASK_BITS {
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
        unsafe { asm!("msr daifset, #2", options(nomem, nostack)) }
    }

    pub fn enable() {
        unsafe { asm!("msr daifclr, #2", options(nomem, nostack)) }
    }


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

    pub fn current_cpu() -> crate::task::CpuId {
        let id: u64;
        unsafe { asm!("mrs {}, mpidr_el1", out(reg) id, options(nomem, nostack)) }
        crate::task::CpuId::new((id & 0xff) as u8)
            .unwrap_or(crate::task::CpuId::new(0).expect("CPU 0 is valid"))
    }

    pub fn send_ipi(_target: crate::task::CpuId, _vector: u8) -> bool {
        // GIC discovery is platform-specific. The vector entry and CPU
        // state are ready; the platform driver supplies the distributor write.
        false
    }

    pub fn end_of_interrupt() {}

    #[repr(C)]
    struct ExceptionFrame {
        registers: [u64; 31],
        elr: u64,
        spsr: u64,
        esr: u64,
        far: u64,
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn synos_aarch64_exception_dispatch(frame: *mut ExceptionFrame) {
        let frame = unsafe { &mut *frame };
        let exception_class = (frame.esr >> 26) & 0x3f;
        if exception_class == 0x15 {
            crate::syscall::synos_call_gate_dispatch(
                frame.registers[0] as *const synos_runtime::Request,
                frame.registers[1] as *mut synos_runtime::Response,
            );
            frame.elr = frame.elr.saturating_add(4);
            return
        }
        crate::capture_exception(
            super::capture_registers(frame.far),
            frame.far,
            synos_status::Status::CORRUPT,
            exception_class as u16,
        );
        crate::println!("AArch64 exception class={exception_class:#x}");
        crate::halt()
    }

    global_asm!(
        r#"
.section .text
.balign 2048
.global synos_aarch64_vectors
synos_aarch64_vectors:
.rept 16
    b synos_aarch64_exception_entry
    .space 124
.endr

.balign 16
synos_aarch64_exception_entry:
    sub sp, sp, #288
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x16, elr_el1
    str x16, [sp, #248]
    mrs x16, spsr_el1
    str x16, [sp, #256]
    mrs x16, esr_el1
    str x16, [sp, #264]
    mrs x16, far_el1
    str x16, [sp, #272]
    mov x0, sp
    bl synos_aarch64_exception_dispatch
    ldr x16, [sp, #248]
    msr elr_el1, x16
    ldr x16, [sp, #256]
    msr spsr_el1, x16
    ldp x0, x1, [sp, #0]
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x19, [sp, #144]
    ldp x20, x21, [sp, #160]
    ldp x22, x23, [sp, #176]
    ldp x24, x25, [sp, #192]
    ldp x26, x27, [sp, #208]
    ldp x28, x29, [sp, #224]
    ldr x30, [sp, #240]
    add sp, sp, #288
    eret
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
