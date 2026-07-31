use crate::cpu::decoder::{DecodedInstruction, InstructionDecoder, InstructionDecodeError};
use crate::cpu::executor::InstructionExecutor;
use crate::memory::Mmu;
use crate::devices::InterruptController;
use crate::firmware::bios::BiosContext;

const CR0_PE_FLAG: u64 = 1 << 0;
const CR0_PG_FLAG: u64 = 1 << 31;
const EFER_LME_FLAG: u64 = 1 << 8;
const EFER_LMA_FLAG: u64 = 1 << 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivilegeLevel {
    Ring0,
    Ring3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuMode {
    Real16,
    Protected16,
    Protected32,
    Long64,
}

#[derive(Clone, Copy, Debug)]
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
    pub mode: CpuMode,
    pub privilege: PrivilegeLevel,
    pub halted: bool,
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
            rip: 0,
            rflags: 0x2,
            cr0: 0,
            cr2: 0,
            cr3: 0,
            cr4: 0,
            efer: 0,
            cs: SegmentRegister::default(),
            ds: SegmentRegister::default(),
            es: SegmentRegister::default(),
            fs: SegmentRegister::default(),
            gs: SegmentRegister::default(),
            ss: SegmentRegister::default(),
            mode: CpuMode::Real16,
            privilege: PrivilegeLevel::Ring0,
            halted: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SegmentRegister {
    pub selector: u16,
    pub base: u64,
    pub limit: u32,
    pub attributes: u16,
}

pub struct Cpu {
    pub state: CpuState,
    decoder: InstructionDecoder,
    executor: InstructionExecutor,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            state: CpuState::default(),
            decoder: InstructionDecoder::new(),
            executor: InstructionExecutor::new(),
        }
    }

    pub fn reset(&mut self) {
        self.state = CpuState::default();
    }

    pub fn step(&mut self, mmu: &mut Mmu, intc: &mut InterruptController, bios: &mut BiosContext) -> Result<(), CpuError> {
        if self.state.halted {
            return Ok(());
        }

        let ip = self.state.rip;
        let instruction = self.decoder.decode(ip, mmu).map_err(CpuError::from)?;
        self.executor.execute(&instruction, &mut self.state, mmu, intc, bios)?;

        Ok(())
    }

    pub fn enter_protected_mode(&mut self) {
        self.state.cr0 |= CR0_PE_FLAG;
        self.state.mode = CpuMode::Protected32;
    }

    pub fn enable_paging(&mut self, cr3: u64) {
        self.state.cr3 = cr3;
        self.state.cr0 |= CR0_PG_FLAG;
        if self.state.mode == CpuMode::Protected32 {
            self.state.mode = CpuMode::Long64;
            self.state.efer |= EFER_LME_FLAG | EFER_LMA_FLAG;
        }
    }

    pub fn handle_interrupt(&mut self, vector: u8, intc: &mut InterruptController) -> Result<(), CpuError> {
        let idt_entry = intc.get_idt_entry(vector);
        if idt_entry == 0 {
            return Err(CpuError::InterruptNotConfigured);
        }

        self.state.rflags &= !0x200;
        self.state.rflags |= 0x100;

        let new_rip = idt_entry;

        self.state.rsp = self.state.rsp.saturating_sub(128);
        self.state.rip = new_rip;
        self.state.cs.selector = 0x08;
        self.state.cs.base = 0;
        self.state.cs.limit = 0xFFFFFFFF;
        self.state.privilege = PrivilegeLevel::Ring0;

        Ok(())
    }

    pub fn handle_exception(&mut self, vector: u8, error_code: Option<u64>, intc: &mut InterruptController) -> Result<(), CpuError> {
        if vector >= 32 {
            return Err(CpuError::InvalidExceptionVector);
        }

        if let Some(code) = error_code {
            self.state.rsp = self.state.rsp.saturating_sub(8);
            unsafe {
                *(self.state.rsp as *mut u64) = code;
            }
        }

        self.handle_interrupt(vector, intc)
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum CpuError {
    InvalidInterruptVector,
    InterruptNotConfigured,
    InvalidExceptionVector,
    InstructionDecodeError,
    PageFault,
    GeneralProtectionFault,
    InvalidOpcode,
    UnsupportedInstruction,
    MemoryAccessError,
}

impl From<InstructionDecodeError> for CpuError {
    fn from(_: InstructionDecodeError) -> Self {
        CpuError::InstructionDecodeError
    }
}

pub mod decoder;
pub mod executor;