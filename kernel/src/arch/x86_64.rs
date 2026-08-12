use core::arch::{asm, global_asm};

pub mod paging {
    use core::arch::asm;

    pub const TABLE_FRAME_COUNT: usize = 6;
    const PRESENT: u64 = 1;
    const WRITABLE: u64 = 1 << 1;
    const USER: u64 = 1 << 2;
    const HUGE_PAGE: u64 = 1 << 7;
    const ENTRY_COUNT: usize = 512;
    const TWO_MIB: u64 = 2 * 1024 * 1024;
    const ONE_GIB: u64 = 1024 * 1024 * 1024;
    pub const DEMO_TABLE_FRAME_COUNT: usize = 7;

    /// Creates a fresh four-level root table with low physical memory identity mapped.
    ///
    /// # Safety
    /// All addresses must name distinct, writable 4 KiB physical frames.
    pub unsafe fn install_root(tables: &[u64; TABLE_FRAME_COUNT], physical_offset: u64) {
        let root = tables[0];
        let level_three = tables[1];
        let pml4 = (root + physical_offset) as *mut u64;
        let pdpt = (level_three + physical_offset) as *mut u64;

        unsafe {
            core::ptr::write_bytes(pml4, 0, ENTRY_COUNT);
            core::ptr::write_bytes(pdpt, 0, ENTRY_COUNT);
            pml4.write(level_three | PRESENT | WRITABLE);

            if supports_one_gib_pages() {
                for index in 0..ENTRY_COUNT {
                    pdpt.add(index).write(
                        index as u64 * ONE_GIB | PRESENT | WRITABLE | HUGE_PAGE,
                    );
                }
            } else {
                for directory_index in 0..4 {
                    let directory_physical = tables[directory_index + 2];
                    let directory =
                        (directory_physical + physical_offset) as *mut u64;
                    core::ptr::write_bytes(directory, 0, ENTRY_COUNT);
                    pdpt.add(directory_index).write(
                        directory_physical | PRESENT | WRITABLE,
                    );

                    for index in 0..ENTRY_COUNT {
                        let address =
                            (directory_index * ENTRY_COUNT + index) as u64 * TWO_MIB;
                        directory
                            .add(index)
                            .write(address | PRESENT | WRITABLE | HUGE_PAGE);
                    }
                }
            }

            asm!("mov cr3, {}", in(reg) root, options(nostack, preserves_flags));
        }
    }

    /// Switch to a process page-table root before entering Ring 3.
    ///
    /// # Safety
    /// `root` must be a live, fully initialized user page-table root that
    /// retains the kernel mappings required by interrupt and return paths.
    pub unsafe fn activate_root(root: u64) {
        unsafe {
            asm!("mov cr3, {}", in(reg) root, options(nostack, preserves_flags));
        }
    }

    /// Build a small real user address space for a boot service image.
    ///
    /// The image pages are identity-mapped, while every other low-memory page
    /// remains supervisor-only. This gives the proof process a separate root,
    /// user permissions, and a user stack without requiring an ELF loader.
    ///
    /// # Safety
    /// All frames must be distinct, aligned, writable physical frames. The
    /// four image frames must occupy one 2 MiB physical region.
    pub unsafe fn install_service_root(
        tables: &[u64; DEMO_TABLE_FRAME_COUNT],
        pages: &[u64; 4],
        physical_offset: u64,
    ) -> Option<crate::PageTableRoot> {
        let region = pages[0] / TWO_MIB;
        if pages.iter().any(|page| {
            *page == 0
                || *page % crate::FRAME_SIZE != 0
                || *page / TWO_MIB != region
                || region >= 4 * ENTRY_COUNT as u64
        }) {
            return None
        }
        if tables.iter().any(|frame| *frame == 0 || *frame % crate::FRAME_SIZE != 0) {
            return None
        }

        let root = (tables[0] + physical_offset) as *mut u64;
        let pdpt = (tables[1] + physical_offset) as *mut u64;
        unsafe {
            for frame in tables {
                core::ptr::write_bytes(
                    (*frame + physical_offset) as *mut u8,
                    0,
                    crate::FRAME_SIZE as usize,
                );
            }
            core::ptr::write_bytes(
                (tables[6] + physical_offset) as *mut u8,
                0,
                crate::FRAME_SIZE as usize,
            );

            root.write(tables[1] | PRESENT | WRITABLE | USER);
            for directory_group in 0..4 {
                let directory = (tables[directory_group + 2] + physical_offset) as *mut u64;
                pdpt.add(directory_group).write(
                    tables[directory_group + 2] | PRESENT | WRITABLE | USER,
                );
                for index in 0..ENTRY_COUNT {
                    let address =
                        (directory_group * ENTRY_COUNT + index) as u64 * TWO_MIB;
                    directory
                        .add(index)
                        .write(address | PRESENT | WRITABLE | HUGE_PAGE);
                }
            }

            let directory_group = (region as usize) / ENTRY_COUNT;
            let directory_index = (region as usize) % ENTRY_COUNT;
            let directory = (tables[directory_group + 2] + physical_offset) as *mut u64;
            directory.add(directory_index).write(
                tables[6] | PRESENT | WRITABLE | USER,
            );

            let page_table = (tables[6] + physical_offset) as *mut u64;
            for page in pages {
                let index = ((*page % TWO_MIB) / crate::FRAME_SIZE) as usize;
                page_table.add(index).write(*page | PRESENT | WRITABLE | USER);
            }
        }

        crate::PageTableRoot::new(tables[0])
    }

    /// Install the request, response, and machine code for a boot service.
    ///
    /// # Safety
    /// `pages` must be the four writable frames passed to
    /// [`install_service_root`].
    pub unsafe fn write_service_image(
        pages: &[u64; 4],
        physical_offset: u64,
        shell: bool,
    ) {
        let mut image = [0u8; 32];
        image[0..2].copy_from_slice(&[0x48, 0xbf]);
        image[2..10].copy_from_slice(&pages[1].to_le_bytes());
        image[10..12].copy_from_slice(&[0x48, 0xbe]);
        image[12..20].copy_from_slice(&pages[2].to_le_bytes());
        if shell {
            image[20..22].copy_from_slice(&[0xcd, 0x80]);
            image[22..28].copy_from_slice(&[0xc7, 0x07, 0x01, 0x00, 0x00, 0x00]);
            image[28..30].copy_from_slice(&[0xeb, 0xf4]);
        } else {
            // Yield repeatedly while a service waits for work.
            image[20..24].copy_from_slice(&[0xcd, 0x80, 0xeb, 0xfc]);
        }

        unsafe {
            core::ptr::write_bytes(
                (pages[0] + physical_offset) as *mut u8,
                0,
                crate::FRAME_SIZE as usize,
            );
            core::ptr::write_bytes(
                (pages[1] + physical_offset) as *mut u8,
                0,
                crate::FRAME_SIZE as usize,
            );
            core::ptr::write_bytes(
                (pages[2] + physical_offset) as *mut u8,
                0,
                crate::FRAME_SIZE as usize,
            );
            core::ptr::write_bytes(
                (pages[3] + physical_offset) as *mut u8,
                0,
                crate::FRAME_SIZE as usize,
            );
            if shell {
                let banner = b"SynOS service shell ready\r\nsynos> ";
                core::ptr::copy_nonoverlapping(
                    banner.as_ptr(),
                    (pages[3] + physical_offset) as *mut u8,
                    banner.len(),
                );
            }
            core::ptr::copy_nonoverlapping(
                image.as_ptr(),
                (pages[0] + physical_offset) as *mut u8,
                image.len(),
            );
            let mut request = synos_runtime::Request::new(if shell {
                synos_runtime::Operation::TerminalWrite
            } else {
                synos_runtime::Operation::Yield
            });
            if shell {
                request.arguments[0] = pages[3];
                request.arguments[1] = b"SynOS service shell ready\r\nsynos> ".len() as u64;
            }
            ((pages[1] + physical_offset) as *mut synos_runtime::Request).write(request);
            ((pages[2] + physical_offset) as *mut synos_runtime::Response)
                .write(synos_runtime::Response::EMPTY);
        }
    }

    fn supports_one_gib_pages() -> bool {
        if core::arch::x86_64::__cpuid(0x8000_0000).eax < 0x8000_0001 {
            return false
        }
        let result = core::arch::x86_64::__cpuid(0x8000_0001);
        result.edx & (1 << 26) != 0
    }
}

pub mod interrupts {
    use super::{asm, global_asm};
    use crate::println;
    use core::sync::atomic::{AtomicU64, Ordering};

    const IDT_ENTRIES: usize = 256;
    const PIT_DIVISOR: u16 = 1_193;
    const PIT_TICK_US: u64 = 1_000;
    const KERNEL_CODE_SELECTOR: u16 = 0x08;
    const KERNEL_DATA_SELECTOR: u16 = 0x10;
    const USER_CODE_SELECTOR: u16 = 0x18 | 3;
    const USER_DATA_SELECTOR: u16 = 0x20 | 3;
    const TSS_SELECTOR: u16 = 0x28;
    static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry::MISSING; IDT_ENTRIES];
    static mut GDT: [u64; 7] = [0; 7];
    static mut TSS: TaskStateSegment = TaskStateSegment::new();
    static ISOLATED_CORES: AtomicU64 = AtomicU64::new(0);

    #[repr(C)]
    struct TaskStateSegment {
        reserved0: u32,
        rsp0: u64,
        rsp1: u64,
        rsp2: u64,
        reserved1: u64,
        ist: [u64; 7],
        reserved2: u64,
        reserved3: u16,
        iomap_base: u16,
    }

    impl TaskStateSegment {
        const fn new() -> Self {
            Self {
                reserved0: 0,
                rsp0: 0,
                rsp1: 0,
                rsp2: 0,
                reserved1: 0,
                ist: [0; 7],
                reserved2: 0,
                reserved3: 0,
                iomap_base: size_of::<Self>() as u16,
            }
        }
    }

    #[repr(C, packed)]
    struct DescriptorTablePointer {
        limit: u16,
        base: u64,
    }

    pub fn set_core_isolated(cpu: u8, isolated: bool) {
        let bit = 1u64 << cpu;
        if isolated {
            ISOLATED_CORES.fetch_or(bit, Ordering::Relaxed);
        } else {
            ISOLATED_CORES.fetch_and(!bit, Ordering::Relaxed);
        }
    }

    fn core_isolated(cpu: u8) -> bool {
        ISOLATED_CORES.load(Ordering::Relaxed) & (1u64 << cpu) != 0
    }

    #[repr(C, packed)]
    struct IdtPointer {
        limit: u16,
        base: u64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct IdtEntry {
        offset_low: u16,
        selector: u16,
        options: u16,
        offset_middle: u16,
        offset_high: u32,
        reserved: u32,
    }

    #[repr(C)]
    struct InterruptFrame {
        r15: u64,
        r14: u64,
        r13: u64,
        r12: u64,
        r11: u64,
        r10: u64,
        r9: u64,
        r8: u64,
        rbp: u64,
        rdi: u64,
        rsi: u64,
        rdx: u64,
        rcx: u64,
        rbx: u64,
        rax: u64,
        vector: u64,
        error_code: u64,
        rip: u64,
        cs: u64,
        rflags: u64,
        rsp: u64,
        ss: u64,
    }

    impl IdtEntry {
        const MISSING: Self = Self {
            offset_low: 0,
            selector: 0,
            options: 0,
            offset_middle: 0,
            offset_high: 0,
            reserved: 0,
        };

        fn handler(address: u64, selector: u16, privilege: u16) -> Self {
            Self {
                offset_low: address as u16,
                selector,
                options: 0x8e00 | (privilege << 13),
                offset_middle: (address >> 16) as u16,
                offset_high: (address >> 32) as u32,
                reserved: 0,
            }
        }
    }

    unsafe extern "C" {
        static synos_isr_table: [u64; IDT_ENTRIES];
    }

    /// Installs an IDT, remaps the legacy PIC, and enables timer and keyboard IRQs.
    ///
    /// # Safety
    /// Must run once on the bootstrap processor while interrupts are disabled.
    pub unsafe fn init() {
        unsafe {
            asm!("cli", options(nomem, nostack));
            install_gdt();

            for index in 0..IDT_ENTRIES {
                let privilege = (index == crate::syscall::CALL_GATE_VECTOR as usize) as u16 * 3;
                IDT[index] = IdtEntry::handler(
                    synos_isr_table[index],
                    KERNEL_CODE_SELECTOR,
                    privilege,
                );
            }

            remap_pic();
            configure_pit();
            outb(0x21, 0xfc);
            outb(0xa1, 0xff);

            let pointer = IdtPointer {
                limit: (size_of::<[IdtEntry; IDT_ENTRIES]>() - 1) as u16,
                base: (&raw const IDT) as u64,
            };
            asm!("lidt [{}]", in(reg) &pointer, options(readonly, nostack, preserves_flags));
        }
    }

    pub fn disable() {
        unsafe { asm!("cli", options(nomem, nostack)) }
    }

    unsafe fn install_gdt() {
        let rsp0: u64;
        unsafe {
            asm!("mov {}, rsp", out(reg) rsp0, options(nomem, nostack, preserves_flags));
            TSS.rsp0 = rsp0;
            GDT[0] = 0;
            GDT[1] = 0x00af9a000000ffff;
            GDT[2] = 0x00af92000000ffff;
            GDT[3] = 0x00affa000000ffff;
            GDT[4] = 0x00aff2000000ffff;
            let base = (&raw const TSS) as u64;
            let limit = (size_of::<TaskStateSegment>() - 1) as u64;
            GDT[5] = (limit & 0xffff)
                | ((base & 0x00ff_ffff) << 16)
                | (0x89 << 40)
                | (((limit >> 16) & 0xf) << 48)
                | (((base >> 24) & 0xff) << 56);
            GDT[6] = base >> 32;
            let pointer = DescriptorTablePointer {
                limit: (size_of::<[u64; 7]>() - 1) as u16,
                base: (&raw const GDT) as u64,
            };
            asm!("lgdt [{}]", in(reg) &pointer, options(readonly, nostack, preserves_flags));
            asm!(
                "mov ax, {data}",
                "mov ds, ax",
                "mov es, ax",
                "mov ss, ax",
                "push {code}",
                "lea rax, [rip + 2f]",
                "push rax",
                "retfq",
                "2:",
                "mov ax, {tss}",
                "ltr ax",
                data = const KERNEL_DATA_SELECTOR,
                code = const KERNEL_CODE_SELECTOR,
                tss = const TSS_SELECTOR,
                out("rax") _,
                options(preserves_flags),
            );
        }
    }

    /// Enter Ring 3 with an already-mapped process context. Hardware returns
    /// through the common ISR epilogue's `iretq` frame.
    pub(crate) fn enter_user(context: &crate::Context, root: crate::PageTableRoot) -> ! {
        unsafe {
            super::paging::activate_root(root.frame());
            let user_code = USER_CODE_SELECTOR as u64;
            let user_data = USER_DATA_SELECTOR as u64;
            let flags = 0x202u64;
            asm!(
                "push {user_data}",
                "push {stack}",
                "push {flags}",
                "push {user_code}",
                "push {entry}",
                "iretq",
                user_data = in(reg) user_data,
                stack = in(reg) context.stack_pointer,
                flags = in(reg) flags,
                user_code = in(reg) user_code,
                entry = in(reg) context.instruction_pointer,
                options(noreturn),
            )
        }
    }

    #[unsafe(no_mangle)]
    unsafe fn save_context(frame: *const InterruptFrame, context: &mut crate::Context) {
        let frame = unsafe { &*frame };
        context.instruction_pointer = frame.rip as usize;
        context.stack_pointer = if frame.cs & 3 == 3 {
            frame.rsp as usize
        } else {
            frame as *const InterruptFrame as usize + 20 * size_of::<u64>()
        };
        context.flags = frame.rflags as usize;
        context.registers = [
            frame.rax as usize,
            frame.rbx as usize,
            frame.rcx as usize,
            frame.rdx as usize,
            frame.rsi as usize,
            frame.rdi as usize,
            frame.rbp as usize,
            frame.r8 as usize,
            frame.r9 as usize,
            frame.r10 as usize,
            frame.r11 as usize,
            frame.r12 as usize,
            frame.r13 as usize,
            frame.r14 as usize,
            frame.r15 as usize,
            0,
        ];
        context.callee_saved = [
            frame.rbx as usize,
            frame.rbp as usize,
            frame.r12 as usize,
            frame.r13 as usize,
            frame.r14 as usize,
            frame.r15 as usize,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
    }

    unsafe fn restore_context(
        frame: *mut InterruptFrame,
        context: &crate::Context,
        mode: crate::ExecutionMode,
        root: Option<crate::PageTableRoot>,
    ) -> u64 {
        let frame = unsafe { &mut *frame };
        frame.rax = context.registers[0] as u64;
        frame.rbx = context.registers[1] as u64;
        frame.rcx = context.registers[2] as u64;
        frame.rdx = context.registers[3] as u64;
        frame.rsi = context.registers[4] as u64;
        frame.rdi = context.registers[5] as u64;
        frame.rbp = context.registers[6] as u64;
        frame.r8 = context.registers[7] as u64;
        frame.r9 = context.registers[8] as u64;
        frame.r10 = context.registers[9] as u64;
        frame.r11 = context.registers[10] as u64;
        frame.r12 = context.registers[11] as u64;
        frame.r13 = context.registers[12] as u64;
        frame.r14 = context.registers[13] as u64;
        frame.r15 = context.registers[14] as u64;
        frame.rip = context.instruction_pointer as u64;
        frame.rflags = context.flags as u64 | 0x202;

        match mode {
            crate::ExecutionMode::User => {
                let Some(root) = root else {
                    return 1
                };
                frame.cs = USER_CODE_SELECTOR as u64;
                frame.rsp = context.stack_pointer as u64;
                frame.ss = USER_DATA_SELECTOR as u64;
                unsafe { super::paging::activate_root(root.frame()) };
                0
            }
            crate::ExecutionMode::Kernel => {
                frame.cs = KERNEL_CODE_SELECTOR as u64;
                frame.rsp = context.stack_pointer as u64;
                1
            }
        }
    }

    #[unsafe(no_mangle)]
    extern "sysv64" fn interrupt_dispatch(
        vector: u64,
        error_code: u64,
        frame: *mut InterruptFrame,
    ) -> u64 {
        // The bootstrap CPU remains a housekeeping CPU today. This gate is
        // also the architectural hook used by AP interrupt routing once SMP
        // startup supplies each core's local ID.
        let isolated = core_isolated(0);
        crate::invariants::debug_assert_valid(crate::invariants::check_interrupt_delivery(
            vector,
            crate::task::CpuId::new(0).expect("bootstrap CPU is valid"),
            isolated,
            !isolated,
        ));
        if isolated {
            return 0
        }
        if vector == crate::syscall::CALL_GATE_VECTOR as u64 {
            let frame_ref = unsafe { &mut *frame };
            crate::syscall::synos_call_gate_dispatch(
                frame_ref.rdi as *const synos_runtime::Request,
                frame_ref.rsi as *mut synos_runtime::Response,
            );
            let mut return_mode = 0;
            unsafe {
                let scheduler =
                    (&mut *core::ptr::addr_of_mut!(crate::SCHEDULER)).assume_init_mut();
                if let Ok(Some(context_switch)) = scheduler.yield_current() {
                    if let Some(previous) = context_switch.previous {
                        if let Ok(context) = scheduler.context_mut(previous) {
                            save_context(frame, context);
                        }
                    }
                    if let Ok(next) = scheduler.thread(context_switch.next) {
                        return_mode = restore_context(
                            frame,
                            &next.context,
                            next.mode,
                            context_switch.next_address_space_root,
                        );
                    }
                }
            }
            return return_mode
        }
        if vector == 14 {
            let fault_address: u64;
            unsafe {
                asm!(
                    "mov {}, cr2",
                    out(reg) fault_address,
                    options(nomem, nostack, preserves_flags)
                );
            }
            let fault = synos_fabric::PageFault::from_x86_error(fault_address, error_code);
            if crate::page_fault::dispatch(fault) {
                return 0
            }
            crate::capture_exception(
                super::capture_registers(fault_address),
                fault_address,
                synos_status::Status::CORRUPT,
                vector as u16,
            );
        }
        if vector < 32 {
            if vector != 14 {
                crate::capture_exception(
                    super::capture_registers(0),
                    0,
                    synos_status::Status::CORRUPT,
                    vector as u16,
                );
            }
            println!("cpu exception vector={vector} error={error_code:#x}");
            if vector != 3 {
                crate::halt()
            }
        }

        if (32..48).contains(&vector) {
            let mut return_mode = 0;
            unsafe {
                if vector == 32 {
                    let scheduler =
                        (&mut *core::ptr::addr_of_mut!(crate::SCHEDULER)).assume_init_mut();
                    if let Some(context_switch) = scheduler.tick(PIT_TICK_US) {
                        if let Some(previous) = context_switch.previous {
                            if let Ok(context) = scheduler.context_mut(previous) {
                                save_context(frame, context);
                            }
                        }
                        if let Ok(next) = scheduler.thread(context_switch.next) {
                            return_mode = restore_context(
                                frame,
                                &next.context,
                                next.mode,
                                context_switch.next_address_space_root,
                            );
                        }
                    }
                }
                if vector >= 40 {
                    outb(0xa0, 0x20);
                }
                outb(0x20, 0x20);
            }
            return return_mode
        }
        0
    }

    unsafe fn remap_pic() {
        unsafe {
            outb(0x20, 0x11);
            io_wait();
            outb(0xa0, 0x11);
            io_wait();
            outb(0x21, 0x20);
            io_wait();
            outb(0xa1, 0x28);
            io_wait();
            outb(0x21, 0x04);
            io_wait();
            outb(0xa1, 0x02);
            io_wait();
            outb(0x21, 0x01);
            io_wait();
            outb(0xa1, 0x01);
            io_wait();
        }
    }

    unsafe fn configure_pit() {
        unsafe {
            outb(0x43, 0x36);
            outb(0x40, PIT_DIVISOR as u8);
            outb(0x40, (PIT_DIVISOR >> 8) as u8);
        }
    }

    unsafe fn outb(port: u16, value: u8) {
        unsafe {
            asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack));
        }
    }

    unsafe fn io_wait() {
        unsafe {
            outb(0x80, 0);
        }
    }

    global_asm!(
        r#"
.section .text
.code64
.altmacro
.extern interrupt_dispatch

.macro ISR_NOERR vector
.global synos_isr_\vector
synos_isr_\vector:
    push 0
    push \vector
    jmp synos_isr_common
.endm

.macro ISR_ERR vector
.global synos_isr_\vector
synos_isr_\vector:
    push \vector
    jmp synos_isr_common
.endm

.set vector, 0
.rept 256
    .if vector == 8 || (vector >= 10 && vector <= 14) || vector == 17 || vector == 21 || vector == 29 || vector == 30
        ISR_ERR %vector
    .else
        ISR_NOERR %vector
    .endif
    .set vector, vector + 1
.endr

synos_isr_common:
    cld
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    cmp qword ptr [rsp + 120], 128
    jne 1f
    mov rdi, [rsp + 120]
    mov rsi, [rsp + 128]
    mov rdx, rsp
    call interrupt_dispatch
    mov [rsp + 112], rax
    jmp 2f
1:
    mov rdi, [rsp + 120]
    mov rsi, [rsp + 128]
    mov rdx, rsp
    call interrupt_dispatch
    mov [rsp + 112], rax
2:
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    add rsp, 16
    test rax, rax
    jz 3f
    mov rcx, [rsp]
    mov rdx, [rsp + 24]
    mov rsp, rdx
    jmp rcx
3:
    iretq

.section .rodata
.align 8
.global synos_isr_table
synos_isr_table:
.macro ISR_TABLE_ENTRY vector
    .quad synos_isr_\vector
.endm
.set vector, 0
.rept 256
    ISR_TABLE_ENTRY %vector
    .set vector, vector + 1
.endr
"#
    );
}

pub(crate) fn enter_user(context: &crate::Context, root: crate::PageTableRoot) -> ! {
    interrupts::enter_user(context, root)
}

#[inline(always)]
pub fn halt() {
    unsafe {
        asm!("hlt", options(nomem, nostack));
    }
}

pub(crate) fn capture_registers(fault_address: u64) -> crate::crash::RegisterState {
    let (rax, rbx, rcx, rdx, rsi, rdi, rbp, r8, r9, r10, r11, r12, r13, r14, r15, rip, rsp, flags);
    unsafe {
        asm!("mov {}, rax", out(reg) rax, options(nomem, nostack, preserves_flags));
        asm!("mov {}, rbx", out(reg) rbx, options(nomem, nostack, preserves_flags));
        asm!("mov {}, rcx", out(reg) rcx, options(nomem, nostack, preserves_flags));
        asm!("mov {}, rdx", out(reg) rdx, options(nomem, nostack, preserves_flags));
        asm!("mov {}, rsi", out(reg) rsi, options(nomem, nostack, preserves_flags));
        asm!("mov {}, rdi", out(reg) rdi, options(nomem, nostack, preserves_flags));
        asm!("mov {}, rbp", out(reg) rbp, options(nomem, nostack, preserves_flags));
        asm!("mov {}, r8", out(reg) r8, options(nomem, nostack, preserves_flags));
        asm!("mov {}, r9", out(reg) r9, options(nomem, nostack, preserves_flags));
        asm!("mov {}, r10", out(reg) r10, options(nomem, nostack, preserves_flags));
        asm!("mov {}, r11", out(reg) r11, options(nomem, nostack, preserves_flags));
        asm!("mov {}, r12", out(reg) r12, options(nomem, nostack, preserves_flags));
        asm!("mov {}, r13", out(reg) r13, options(nomem, nostack, preserves_flags));
        asm!("mov {}, r14", out(reg) r14, options(nomem, nostack, preserves_flags));
        asm!("mov {}, r15", out(reg) r15, options(nomem, nostack, preserves_flags));
        asm!("lea {}, [rip]", out(reg) rip, options(nomem, nostack, preserves_flags));
        asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack, preserves_flags));
        asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags));
    }
    crate::crash::RegisterState {
        general: [rax, rbx, rcx, rdx, rsi, rdi, rbp, r8, r9, r10, r11, r12, r13, r14, r15, 0],
        instruction_pointer: rip,
        stack_pointer: rsp,
        flags,
        fault_address,
    }
}
