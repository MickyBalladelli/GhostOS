//! CPU emulation core: state, mode transitions, and the execute loop.

use crate::cpu::decoder::{InstructionDecoder, InstructionDecodeError};
use crate::cpu::executor::InstructionExecutor;
use crate::devices::{IdtGate, InterruptController, LocalApic, PortBus, PvClock};
use crate::firmware::bios::BiosContext;
use crate::memory::Mmu;
use std::cell::RefCell;
use std::rc::Rc;

const CR0_PE_FLAG: u64 = 1 << 0;
const CR0_PG_FLAG: u64 = 1 << 31;
const EFER_LME_FLAG: u64 = 1 << 8;
const EFER_LMA_FLAG: u64 = 1 << 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrivilegeLevel {
    Ring0,
    Ring3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CpuMode {
    Real16,
    Protected16,
    Protected32,
    Long64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuState {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub efer: u64,
    pub cs: SegmentRegister,
    pub ds: SegmentRegister,
    pub es: SegmentRegister,
    pub fs: SegmentRegister,
    pub gs: SegmentRegister,
    pub ss: SegmentRegister,
    pub gdtr: DescriptorTableRegister,
    pub idtr: DescriptorTableRegister,
    pub mode: CpuMode,
    pub privilege: PrivilegeLevel,
    /// Per-CPU interrupt stack table (only index 0 used for simplicity).
    pub ist_stack: u64,
    /// STAR / LSTAR MSRs used by the syscall/sysret instructions.
    pub star: u64,
    pub lstar: u64,
    /// Hidden segment bases for FS and GS (set via wrmsr).
    pub fs_base: u64,
    pub gs_base: u64,
    pub halted: bool,
    /// STI interrupt-enable shadow: maskable interrupts are held for one
    /// instruction after STI, matching real hardware.
    pub interrupt_shadow: bool,
}

impl Default for CpuState {
    fn default() -> Self {
        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            rsp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            // RESET state: first instruction executes at 0x0000_FFF0.
            rip: 0x0000_FFF0,
            rflags: 0x2,
            cr0: 0x6000_0010,
            cr2: 0,
            cr3: 0,
            cr4: 0,
            efer: 0,
            cs: SegmentRegister {
                selector: 0xF000,
                base: 0xFFFF_0000,
                limit: 0xFFFF,
                attributes: 0x9B,
            },
            ds: SegmentRegister::default(),
            es: SegmentRegister::default(),
            fs: SegmentRegister::default(),
            gs: SegmentRegister::default(),
            ss: SegmentRegister {
                selector: 0,
                base: 0,
                limit: 0xFFFF,
                attributes: 0x93,
            },
            gdtr: DescriptorTableRegister::default(),
            idtr: DescriptorTableRegister::default(),
            mode: CpuMode::Real16,
            privilege: PrivilegeLevel::Ring0,
            ist_stack: 0,
            star: 0,
            lstar: 0,
            fs_base: 0,
            gs_base: 0,
            halted: false,
            interrupt_shadow: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SegmentRegister {
    pub selector: u16,
    pub base: u64,
    pub limit: u32,
    pub attributes: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DescriptorTableRegister {
    pub base: u64,
    pub limit: u16,
}

impl CpuState {
    /// Read a full-width register.
    pub fn reg(&self, idx: u8) -> u64 {
        match idx {
            0 => self.rax,
            1 => self.rcx,
            2 => self.rdx,
            3 => self.rbx,
            4 => self.rsp,
            5 => self.rbp,
            6 => self.rsi,
            7 => self.rdi,
            8 => self.r8,
            9 => self.r9,
            10 => self.r10,
            11 => self.r11,
            12 => self.r12,
            13 => self.r13,
            14 => self.r14,
            15 => self.r15,
            _ => 0,
        }
    }

    /// Write a full-width register.
    pub fn set_reg(&mut self, idx: u8, value: u64) {
        match idx {
            0 => self.rax = value,
            1 => self.rcx = value,
            2 => self.rdx = value,
            3 => self.rbx = value,
            4 => self.rsp = value,
            5 => self.rbp = value,
            6 => self.rsi = value,
            7 => self.rdi = value,
            8 => self.r8 = value,
            9 => self.r9 = value,
            10 => self.r10 = value,
            11 => self.r11 = value,
            12 => self.r12 = value,
            13 => self.r13 = value,
            14 => self.r14 = value,
            15 => self.r15 = value,
            _ => {}
        }
    }

    /// Read a debug register (simplified: only DR0-DR7 stored as 0).
    pub fn debug_reg(&self, _dr: u8) -> u64 {
        0
    }

    /// Read a segment register's selector by our segment code.
    pub fn seg_read(&self, code: u8) -> u16 {
        match code {
            1 => self.cs.selector,
            2 => self.ds.selector,
            3 => self.es.selector,
            4 => self.fs.selector,
            5 => self.gs.selector,
            6 => self.ss.selector,
            _ => 0,
        }
    }

    /// Write a segment register's selector by our segment code. The hidden
    /// base is left unchanged for FS/GS (those are set via wrmsr).
    pub fn seg_write(&mut self, code: u8, selector: u16) {
        match code {
            1 => {
                self.cs.selector = selector;
                self.cs.base = 0;
                self.cs.limit = 0xFFFF_FFFF;
            }
            2 => {
                self.ds.selector = selector;
                self.ds.base = 0;
            }
            3 => {
                self.es.selector = selector;
                self.es.base = 0;
            }
            4 => self.fs.selector = selector,
            5 => self.gs.selector = selector,
            6 => self.ss.selector = selector,
            _ => {}
        }
    }

    /// Load a visible and hidden segment state from the guest GDT.
    pub fn load_segment(
        &mut self,
        code: u8,
        selector: u16,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if self.mode == CpuMode::Real16 {
            self.seg_write(code, selector);
            move_segment_real_base(self, code, selector);
            return Ok(())
        }
        if selector == 0 || selector & 4 != 0 {
            return Err(CpuError::GeneralProtectionFault)
        }
        // `enter_protected` may use the VM's built-in flat segments without
        // installing a guest-visible GDT. Keep those two selectors usable.
        if self.gdtr.limit == 0 && (selector == 0x08 || selector == 0x10) {
            if (code == 1) != (selector == 0x08) {
                return Err(CpuError::GeneralProtectionFault)
            }
            let segment = SegmentRegister {
                selector,
                base: 0,
                limit: 0xFFFF_FFFF,
                attributes: if selector == 0x08 { 0x9B } else { 0x93 },
            };
            match code {
                1 => self.cs = segment,
                2 => self.ds = segment,
                3 => self.es = segment,
                4 => self.fs = segment,
                5 => self.gs = segment,
                6 => self.ss = segment,
                _ => return Err(CpuError::GeneralProtectionFault),
            }
            return Ok(())
        }
        let offset = (selector as u64 & !7)
            .checked_add(self.gdtr.base)
            .ok_or(CpuError::GeneralProtectionFault)?;
        if (selector as u32 & !7) + 7 > self.gdtr.limit as u32 {
            return Err(CpuError::GeneralProtectionFault)
        }
        let raw = mmu.read_u64(offset).map_err(|_| CpuError::GeneralProtectionFault)?;
        let access = ((raw >> 40) & 0xFF) as u8;
        if access & 0x80 == 0 {
            return Err(CpuError::GeneralProtectionFault)
        }
        let is_code = access & 0x08 != 0;
        if (code == 1) != is_code {
            return Err(CpuError::GeneralProtectionFault)
        }
        let mut base = ((raw >> 16) & 0xFFFF) | (((raw >> 32) & 0xFF) << 16) | (((raw >> 56) & 0xFF) << 24);
        let mut limit = (raw & 0xFFFF) | (((raw >> 48) & 0x0F) << 16);
        if raw & (1 << 55) != 0 {
            limit = (limit << 12) | 0xFFF;
        }
        if self.mode == CpuMode::Long64 && code != 4 && code != 5 {
            base = 0;
            limit = 0xFFFF_FFFF;
        }
        let segment = SegmentRegister {
            selector,
            base,
            limit: limit as u32,
            attributes: u16::from(access) | (((raw >> 52) as u16) & 0xF) << 8,
        };
        match code {
            1 => self.cs = segment,
            2 => self.ds = segment,
            3 => self.es = segment,
            4 => self.fs = segment,
            5 => self.gs = segment,
            6 => self.ss = segment,
            _ => return Err(CpuError::GeneralProtectionFault),
        }
        Ok(())
    }

    /// Read a register with a given operand size (this VM runs in 64-bit
    /// mode, so 32-bit reads zero-extend and 8/16-bit reads mask).
    pub fn reg_size(&self, idx: u8, size: u8) -> u64 {
        let full = self.reg(idx);
        // 8-bit registers are special: indices 4-7 map to AH/CH/DH/BH in the
        // legacy encoding, but our decoder already remaps them to SPL/BPL/
        // SIL/DIL or the high bytes via rex. For emulation purposes we treat
        // bytes 0-15 as the low byte of each register; high-byte access is
        // handled by the executor for the (rare) legacy no-rex 8-bit case.
        match size {
            1 => full & 0xFF,
            2 => full & 0xFFFF,
            4 => full & 0xFFFF_FFFF,
            _ => full,
        }
    }

    pub fn set_reg_size(&mut self, idx: u8, size: u8, value: u64) {
        match size {
            1 => {
                let full = self.reg(idx);
                let masked = (full & !0xFF) | (value & 0xFF);
                self.set_reg(idx, masked);
            }
            2 => {
                let full = self.reg(idx);
                let masked = (full & !0xFFFF) | (value & 0xFFFF);
                self.set_reg(idx, masked);
            }
            4 => {
                // Writing a 32-bit register zero-extends to 64 bits.
                self.set_reg(idx, value & 0xFFFF_FFFF);
            }
            _ => self.set_reg(idx, value),
        }
    }

    /// Enter protected mode with a flat GDT, if `gdtr_base` is non-zero.
    pub fn enter_protected(&mut self, mmu: &mut Mmu, gdtr_base: u64) -> Result<(), CpuError> {
        if self.mode != CpuMode::Real16 {
            return Err(CpuError::InvalidModeTransition);
        }

        if gdtr_base != 0 {
            install_flat_gdt(mmu, gdtr_base)?;
            self.gdtr = DescriptorTableRegister {
                base: gdtr_base,
                limit: 5 * 8 - 1,
            };
        }

        self.cs = SegmentRegister {
            selector: 0x08,
            base: 0,
            limit: 0xFFFF_FFFF,
            attributes: 0x9B,
        };
        for seg in [
            &mut self.ds,
            &mut self.es,
            &mut self.fs,
            &mut self.gs,
            &mut self.ss,
        ] {
            *seg = SegmentRegister {
                selector: 0x10,
                base: 0,
                limit: 0xFFFF_FFFF,
                attributes: 0x93,
            };
        }

        self.cr0 |= CR0_PE_FLAG;
        self.mode = CpuMode::Protected32;
        self.privilege = PrivilegeLevel::Ring0;
        mmu.set_privilege(false);
        Ok(())
    }

    /// Enter long mode from protected mode with paging enabled.
    pub fn enter_long(&mut self, mmu: &mut Mmu, cr3: u64) -> Result<(), CpuError> {
        if self.mode != CpuMode::Protected32 {
            return Err(CpuError::InvalidModeTransition);
        }
        if self.efer & EFER_LME_FLAG == 0 {
            return Err(CpuError::InvalidModeTransition);
        }

        self.cr3 = cr3 & 0x000F_FFFF_FFFF_F000;
        self.efer |= EFER_LME_FLAG | EFER_LMA_FLAG;
        self.cr0 |= CR0_PE_FLAG | CR0_PG_FLAG;
        self.mode = CpuMode::Long64;

        self.cs = SegmentRegister {
            selector: 0x08,
            base: 0,
            limit: 0xFFFF_FFFF,
            attributes: 0x9B | 0x2000, // L-bit
        };
        self.ds = SegmentRegister {
            selector: 0x10,
            base: 0,
            limit: 0xFFFF_FFFF,
            attributes: 0x93,
        };

        mmu.set_paging(true, self.cr3);
        Ok(())
    }

    /// Re-evaluate paging/mode after `mov crX` or `wrmsr`.
    pub fn update_paging(&mut self, mmu: &mut Mmu) -> Result<(), CpuError> {
        let pe = self.cr0 & CR0_PE_FLAG != 0;
        let pg = self.cr0 & CR0_PG_FLAG != 0;
        let lme = self.efer & EFER_LME_FLAG != 0;

        if !pe {
            if pg {
                return Err(CpuError::InvalidModeTransition);
            }
            self.mode = CpuMode::Real16;
            self.efer &= !EFER_LMA_FLAG;
            mmu.set_paging(false, 0);
            return Ok(());
        }

        if pg && lme && self.mode != CpuMode::Long64 {
            self.efer |= EFER_LMA_FLAG;
            self.mode = CpuMode::Long64;
            self.cs.attributes |= 0x2000;
            mmu.set_paging(true, self.cr3);
            return Ok(());
        }

        if self.mode == CpuMode::Long64 && !pg {
            self.efer &= !EFER_LMA_FLAG;
            self.mode = CpuMode::Protected32;
            mmu.set_paging(false, 0);
            return Ok(());
        }

        if self.mode != CpuMode::Long64 {
            mmu.set_paging(pg, self.cr3);
            if !pg {
                self.efer &= !EFER_LMA_FLAG;
            }
        }

        mmu.set_privilege(self.privilege == PrivilegeLevel::Ring3);
        Ok(())
    }

    /// Deliver an interrupt or exception through the IDT.
    pub fn deliver(
        &mut self,
        vector: u8,
        error_code: Option<u64>,
        is_exception: bool,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
    ) -> Result<(), CpuError> {
        if is_exception && vector >= 32 {
            return Err(CpuError::InvalidExceptionVector);
        }

        let entry_addr = intc
            .idt_entry_address(vector)
            .ok_or(CpuError::InterruptNotConfigured)?;
        // Hardware performs IDT lookup in supervisor context even when the
        // interrupted code runs with user page tables.
        let was_user = self.privilege == PrivilegeLevel::Ring3;
        mmu.set_privilege(false);
        let raw_result = mmu.read_descriptor(entry_addr);
        mmu.set_privilege(was_user);
        let raw = raw_result.map_err(|_| CpuError::InterruptNotConfigured)?;
        let gate = IdtGate::decode(&raw);

        if !gate.present() {
            return Err(CpuError::InterruptNotConfigured);
        }

        let old_ss = self.ss.selector;
        let old_rsp = self.rsp;
        let old_rflags = self.rflags;
        let old_cs = self.cs.selector;
        let old_rip = self.rip;

        let mut new_rsp = self.rsp;

        // IST handling.
        if gate.ist != 0 && self.mode == CpuMode::Long64 && self.ist_stack != 0 {
            new_rsp = self.ist_stack;
        }

        let push64 = |mmu: &mut Mmu, rsp: &mut u64, value: u64| -> Result<(), CpuError> {
            *rsp = rsp.saturating_sub(8);
            mmu.write_to_addr(*rsp, value, 8)
                .map_err(|_| CpuError::MemoryAccessError)
        };

        // DPL controls which callers may enter the gate. The target ring is
        // determined by the selector in the gate, so a DPL 3 syscall gate
        // still enters its Ring 0 handler.
        let new_privilege = if gate.selector & 3 == 0 {
            PrivilegeLevel::Ring0
        } else {
            PrivilegeLevel::Ring3
        };
        if new_privilege != self.privilege {
            push64(mmu, &mut new_rsp, old_ss as u64)?;
            push64(mmu, &mut new_rsp, old_rsp)?;
        }

        push64(mmu, &mut new_rsp, old_rflags)?;
        push64(mmu, &mut new_rsp, old_cs as u64)?;
        push64(mmu, &mut new_rsp, old_rip)?;

        if let Some(code) = error_code {
            push64(mmu, &mut new_rsp, code)?;
        }

        // Interrupt gates clear IF (bit 9); trap gates leave it.
        let type_low = gate.type_attr & 0x0F;
        if type_low == 0xE {
            self.rflags &= !(1 << 9);
        }

        self.rsp = new_rsp;
        self.rip = gate.offset;
        self.cs.selector = gate.selector;
        self.cs.base = 0;
        self.cs.limit = 0xFFFF_FFFF;
        self.privilege = new_privilege;
        mmu.set_privilege(new_privilege == PrivilegeLevel::Ring3);
        Ok(())
    }

    /// Deliver an external interrupt (IRQ).
    pub fn handle_interrupt(
        &mut self,
        vector: u8,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
    ) -> Result<(), CpuError> {
        // HLT is a wait state, not a terminal CPU state. Any accepted
        // external interrupt wakes the processor before entering its handler.
        self.halted = false;
        self.deliver(vector, None, false, mmu, intc)
    }

    /// Deliver an exception with an optional error code.
    pub fn handle_exception(
        &mut self,
        vector: u8,
        error_code: Option<u64>,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
    ) -> Result<(), CpuError> {
        if vector == 8 {
            return Err(CpuError::DoubleFault);
        }
        self.deliver(vector, error_code, true, mmu, intc)
    }

    /// Convenience: raise a #PF with CR2/error-code setup.
    pub fn raise_page_fault(
        &mut self,
        fault_addr: u64,
        present: bool,
        write: bool,
        user: bool,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
    ) -> Result<(), CpuError> {
        self.cr2 = fault_addr & 0x000F_FFFF_FFFF_F000;
        let mut error = 0u64;
        if present {
            error |= 1;
        }
        if write {
            error |= 2;
        }
        if user {
            error |= 4;
        }
        self.handle_exception(14, Some(error), mmu, intc)
    }
}

pub struct Cpu {
    pub state: CpuState,
    decoder: InstructionDecoder,
    executor: InstructionExecutor,
    apic: Option<Rc<RefCell<LocalApic>>>,
    pv_clock: Option<Rc<RefCell<PvClock>>>,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            state: CpuState::default(),
            decoder: InstructionDecoder::new(),
            executor: InstructionExecutor::new(),
            apic: None,
            pv_clock: None,
        }
    }

    /// Attach the shared local APIC so `wrmsr`/`rdmsr` can route the
    /// `IA32_APIC_BASE` MSR to the same device exposed via MMIO.
    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic);
    }

    pub fn attach_pv_clock(&mut self, pv_clock: Rc<RefCell<PvClock>>) {
        self.pv_clock = Some(pv_clock);
    }

    pub fn reset(&mut self) {
        self.state = CpuState::default();
    }

    /// Execute a single instruction.
    pub fn step(
        &mut self,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
        ports: &mut PortBus,
        bios: &mut BiosContext,
    ) -> Result<(), CpuError> {
        if self.state.halted {
            return Ok(());
        }

        // The STI window covers exactly one instruction: clear it before the
        // next instruction executes so a pending maskable interrupt may be
        // delivered on the following step.
        let previous_interrupt_shadow = self.state.interrupt_shadow;
        self.state.interrupt_shadow = false;

        let ip = self.state.rip;
        let instruction = self.decode_instruction(ip, mmu)?;
        let result = self.execute_decoded(&instruction, mmu, intc, ports, bios);
        if matches!(result, Err(CpuError::UnsupportedInstruction)) {
            self.state.interrupt_shadow = previous_interrupt_shadow;
        }
        result
    }

    /// Decode one instruction without executing it. The execution engine
    /// uses this to build and cache straight-line translation blocks.
    pub fn decode_instruction(
        &self,
        ip: u64,
        mmu: &Mmu,
    ) -> Result<crate::cpu::decoder::DecodedInstruction, CpuError> {
        self.decoder.decode(ip, mmu).map_err(CpuError::from)
    }

    /// Prepare the architectural state for one instruction boundary.
    pub fn prepare_instruction(&mut self) {
        self.state.interrupt_shadow = false;
    }

    /// Execute a previously decoded instruction. Callers must ensure that
    /// the instruction still starts at the current RIP.
    pub fn execute_decoded(
        &mut self,
        instruction: &crate::cpu::decoder::DecodedInstruction,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
        ports: &mut PortBus,
        bios: &mut BiosContext,
    ) -> Result<(), CpuError> {
        // Take an owned handle to the shared APIC before borrowing `state`
        // so the borrow checker sees disjoint sources (state field vs. the
        // Rc'd device behind the APIC field).
        let apic_rc = self.apic.clone();
        let pv_clock_rc = self.pv_clock.clone();
        let needs_msr = matches!(instruction.mnemonic, "RDMSR" | "WRMSR");
        let mut apic = if needs_msr {
            apic_rc.as_ref().map(|a| a.borrow_mut())
        } else {
            None
        };
        let mut pv_clock = if needs_msr {
            pv_clock_rc.as_ref().map(|clock| clock.borrow_mut())
        } else {
            None
        };
        let state = &mut self.state;
        self.executor.execute(
            instruction,
            state,
            mmu,
            intc,
            ports,
            bios,
            apic.as_deref_mut(),
            pv_clock.as_deref_mut(),
        )?;
        Ok(())
    }

    // Compatibility wrappers over CpuState methods.

    pub fn enter_protected_mode(&mut self, mmu: &mut Mmu, gdtr_base: u64) -> Result<(), CpuError> {
        self.state.enter_protected(mmu, gdtr_base)
    }

    pub fn enter_long_mode(&mut self, mmu: &mut Mmu, cr3: u64) -> Result<(), CpuError> {
        self.state.enter_long(mmu, cr3)
    }

    pub fn enable_paging(&mut self, cr3: u64) {
        self.state.cr3 = cr3 & 0x000F_FFFF_FFFF_F000;
        self.state.cr0 |= CR0_PG_FLAG;
    }

    pub fn update_paging_state(&mut self, mmu: &mut Mmu) -> Result<(), CpuError> {
        self.state.update_paging(mmu)
    }

    pub fn handle_interrupt(
        &mut self,
        vector: u8,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
    ) -> Result<(), CpuError> {
        self.state.handle_interrupt(vector, mmu, intc)
    }

    pub fn handle_exception(
        &mut self,
        vector: u8,
        error_code: Option<u64>,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
    ) -> Result<(), CpuError> {
        self.state.handle_exception(vector, error_code, mmu, intc)
    }

    pub fn raise_page_fault(
        &mut self,
        fault_addr: u64,
        present: bool,
        write: bool,
        user: bool,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
    ) -> Result<(), CpuError> {
        self.state.raise_page_fault(fault_addr, present, write, user, mmu, intc)
    }

    pub fn set_gdtr(&mut self, base: u64, limit: u16) {
        self.state.gdtr = DescriptorTableRegister { base, limit };
    }

    pub fn set_idtr(&mut self, base: u64, limit: u16, intc: &mut InterruptController) {
        self.state.idtr = DescriptorTableRegister { base, limit };
        intc.set_idt(base, limit);
    }

    pub fn mode(&self) -> CpuMode {
        self.state.mode
    }

    pub fn privilege(&self) -> PrivilegeLevel {
        self.state.privilege
    }

    pub fn rip(&self) -> u64 {
        self.state.rip
    }

    pub fn set_rip(&mut self, rip: u64) {
        self.state.rip = rip;
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

fn move_segment_real_base(state: &mut CpuState, code: u8, selector: u16) {
    if state.mode != CpuMode::Real16 || code > 6 {
        return
    }
    let base = (selector as u64) << 4;
    match code {
        1 => state.cs.base = base,
        2 => state.ds.base = base,
        3 => state.es.base = base,
        4 => state.fs.base = base,
        5 => state.gs.base = base,
        6 => state.ss.base = base,
        _ => {}
    }
}

/// Install a minimal flat GDT (null, code, data) at `base` in guest RAM.
fn install_flat_gdt(mmu: &mut Mmu, base: u64) -> Result<(), CpuError> {
    let gdt: [u8; 24] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // null
        0xFF, 0xFF, 0x00, 0x00, 0x00, 0x9A, 0xAF, 0x00, // code
        0xFF, 0xFF, 0x00, 0x00, 0x00, 0x93, 0xCF, 0x00, // data
    ];
    mmu.write_phys(base, &gdt)
        .map_err(|_| CpuError::MemoryAccessError)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuError {
    InvalidInterruptVector,
    InterruptNotConfigured,
    InvalidExceptionVector,
    InvalidModeTransition,
    InstructionDecodeError,
    PageFault,
    GeneralProtectionFault,
    InvalidOpcode,
    /// A valid instruction was decoded, but the VM has no implementation for
    /// it. This is a VM execution error: do not inject #UD and do not halt the
    /// guest. The caller must stop or report the VM error.
    UnsupportedInstruction,
    MemoryAccessError,
    ReplayDivergence,
    AlignmentCheck,
    DoubleFault,
    DivideError,
}

impl From<InstructionDecodeError> for CpuError {
    fn from(_: InstructionDecodeError) -> Self {
        CpuError::InstructionDecodeError
    }
}

pub mod decoder;
pub mod executor;
