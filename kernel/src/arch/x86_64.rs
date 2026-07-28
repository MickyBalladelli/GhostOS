use core::arch::{asm, global_asm};

pub mod paging {
    use core::arch::asm;

    pub const TABLE_FRAME_COUNT: usize = 6;
    const PRESENT: u64 = 1;
    const WRITABLE: u64 = 1 << 1;
    const HUGE_PAGE: u64 = 1 << 7;
    const ENTRY_COUNT: usize = 512;
    const TWO_MIB: u64 = 2 * 1024 * 1024;
    const ONE_GIB: u64 = 1024 * 1024 * 1024;

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

    const IDT_ENTRIES: usize = 256;
    static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry::MISSING; IDT_ENTRIES];

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

    impl IdtEntry {
        const MISSING: Self = Self {
            offset_low: 0,
            selector: 0,
            options: 0,
            offset_middle: 0,
            offset_high: 0,
            reserved: 0,
        };

        fn handler(address: u64, selector: u16) -> Self {
            Self {
                offset_low: address as u16,
                selector,
                options: 0x8e00,
                offset_middle: (address >> 16) as u16,
                offset_high: (address >> 32) as u32,
                reserved: 0,
            }
        }
    }

    unsafe extern "C" {
        static synos_isr_table: [u64; IDT_ENTRIES];
    }

    /// Installs an IDT, remaps the legacy PIC, and enables only the timer IRQ.
    ///
    /// # Safety
    /// Must run once on the bootstrap processor while interrupts are disabled.
    pub unsafe fn init() {
        unsafe {
            let code_selector: u16;
            asm!("mov {0:x}, cs", out(reg) code_selector, options(nomem, nostack, preserves_flags));

            for index in 0..IDT_ENTRIES {
                IDT[index] = IdtEntry::handler(synos_isr_table[index], code_selector);
            }

            remap_pic();
            outb(0x21, 0xfe);
            outb(0xa1, 0xff);

            let pointer = IdtPointer {
                limit: (size_of::<[IdtEntry; IDT_ENTRIES]>() - 1) as u16,
                base: (&raw const IDT) as u64,
            };
            asm!("lidt [{}]", in(reg) &pointer, options(readonly, nostack, preserves_flags));
        }
    }

    /// # Safety
    /// The active IDT and interrupt controllers must be initialized first.
    pub unsafe fn enable() {
        unsafe {
            asm!("sti", options(nomem, nostack));
        }
    }

    #[unsafe(no_mangle)]
    extern "C" fn interrupt_dispatch(vector: u64, error_code: u64) {
        if vector < 32 {
            println!("cpu exception vector={vector} error={error_code:#x}");
            if vector != 3 {
                crate::halt()
            }
        }

        if (32..48).contains(&vector) {
            unsafe {
                if vector >= 40 {
                    outb(0xa0, 0x20);
                }
                outb(0x20, 0x20);
            }
        }
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
    mov rdi, [rsp + 120]
    mov rsi, [rsp + 128]
    call interrupt_dispatch
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

#[inline(always)]
pub fn halt() {
    unsafe {
        asm!("hlt", options(nomem, nostack));
    }
}
