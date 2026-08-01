//! Instruction executor: turns decoded instructions into CPU/RAM state
//! changes. This is the core of the CPU emulation loop alongside the decoder
//! in `decoder.rs`.

use crate::cpu::decoder::{DecodedInstruction, MemoryOperand, Operand};
use crate::cpu::{CpuError, CpuMode, CpuState, PrivilegeLevel};
use crate::devices::{DeviceError, InterruptController, LocalApic, PortBus, IA32_APIC_BASE_MSR};
use crate::firmware::bios::BiosContext;
use crate::memory::{MemoryError, Mmu};

// ---------------------------------------------------------------------------
// RFLAGS bit positions
// ---------------------------------------------------------------------------
const CF: u64 = 1 << 0;
const PF: u64 = 1 << 2;
const AF: u64 = 1 << 4;
const ZF: u64 = 1 << 6;
const SF: u64 = 1 << 7;
const IF: u64 = 1 << 9;
const DF: u64 = 1 << 10;
const OF: u64 = 1 << 11;

/// `IA32_APIC_BASE` MSR number as a u64 (match patterns cannot cast).
const IA32_APIC_BASE_MSR_U64: u64 = IA32_APIC_BASE_MSR as u64;

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

#[inline]
fn operand_mask(bits: u8) -> u64 {
    let size = bits / 8;
    match size {
        1 => 0xFF,
        2 => 0xFFFF,
        4 => 0xFFFF_FFFF,
        _ => u64::MAX,
    }
}

#[inline]
fn sign_bit(bits: u8) -> u64 {
    let size = bits / 8;
    match size {
        1 => 0x80,
        2 => 0x8000,
        4 => 0x8000_0000,
        _ => 0x8000_0000_0000_0000,
    }
}

#[inline]
fn bit_count(bits: u8) -> u64 {
    bits as u64
}

#[inline]
fn operand_bytes(bits: u8) -> u8 {
    match bits {
        8 => 1,
        16 => 2,
        32 => 4,
        64 => 8,
        _ => bits / 8,
    }
}

#[inline]
fn parity(x: u64) -> bool {
    (x as u8).count_ones() % 2 == 0
}

fn write_flags(state: &mut CpuState, carry: bool, parity_flag: bool, adjust: bool, zero: bool, sign: bool, overflow: bool) {
    let mut f = state.rflags & !(CF | PF | AF | ZF | SF | OF);
    if carry {
        f |= CF;
    }
    if parity_flag {
        f |= PF;
    }
    if adjust {
        f |= AF;
    }
    if zero {
        f |= ZF;
    }
    if sign {
        f |= SF;
    }
    if overflow {
        f |= OF;
    }
    state.rflags = f;
}

fn mem_err(e: MemoryError) -> CpuError {
    match e {
        MemoryError::PageFault => CpuError::PageFault,
        MemoryError::AccessDenied => CpuError::GeneralProtectionFault,
        _ => CpuError::MemoryAccessError,
    }
}

fn port_err(e: DeviceError) -> CpuError {
    match e {
        DeviceError::AccessDenied => CpuError::GeneralProtectionFault,
        _ => CpuError::MemoryAccessError,
    }
}

/// Compute the linear (segment-offset applied) address of a memory operand.
fn effective_address(ins: &DecodedInstruction, state: &CpuState, mem: &MemoryOperand) -> u64 {
    if mem.rip_relative {
        return ins.next_ip.wrapping_add(mem.displacement as i64 as u64);
    }

    let mut base = mem.base.map(|b| state.reg(b)).unwrap_or(0);
    let mut index = mem.index.map(|i| state.reg(i)).unwrap_or(0);

    if ins.addrsize == 32 {
        base &= 0xFFFF_FFFF;
        index &= 0xFFFF_FFFF;
    }

    let addr = base
        .wrapping_add(mem.displacement as i64 as u64)
        .wrapping_add(index.wrapping_mul(mem.scale as u64));

    if ins.addrsize == 32 {
        addr & 0xFFFF_FFFF
    } else {
        addr
    }
}

fn segment_base(state: &CpuState, seg: u8) -> u64 {
    match seg {
        4 => state.fs_base,
        5 => state.gs_base,
        _ => 0,
    }
}

fn read_operand_sized(
    ins: &DecodedInstruction,
    state: &CpuState,
    mmu: &Mmu,
    op: &Operand,
    size: u8,
) -> Result<u64, CpuError> {
    match op {
        Operand::Register(r) => Ok(state.reg_size(*r, size)),
        Operand::Memory(mem) => {
            let addr = effective_address(ins, state, mem)
                .wrapping_add(segment_base(state, mem.segment));
            mmu.read_from_addr(addr, size).map_err(mem_err)
        }
        Operand::Immediate(v) => Ok(*v),
        Operand::Segment(code) => Ok(state.seg_read(*code) as u64),
        Operand::ControlRegister(cr) => Ok(read_cr(state, *cr)),
        Operand::DebugRegister(dr) => Ok(state.debug_reg(*dr)),
        _ => Ok(0),
    }
}

fn read_operand(
    ins: &DecodedInstruction,
    state: &CpuState,
    mmu: &Mmu,
    op: &Operand,
) -> Result<u64, CpuError> {
    read_operand_sized(ins, state, mmu, op, operand_bytes(ins.opsize))
}

fn write_operand(
    ins: &DecodedInstruction,
    state: &mut CpuState,
    mmu: &mut Mmu,
    op: &Operand,
    value: u64,
) -> Result<(), CpuError> {
    let size = operand_bytes(ins.opsize);
    match op {
        Operand::Register(r) => {
            state.set_reg_size(*r, size, value);
            Ok(())
        }
        Operand::Memory(mem) => {
            let addr = effective_address(ins, state, mem)
                .wrapping_add(segment_base(state, mem.segment));
            mmu.write_to_addr(addr, value, size).map_err(mem_err)
        }
        _ => Ok(()),
    }
}

fn read_cr(state: &CpuState, cr: u8) -> u64 {
    match cr {
        0 => state.cr0,
        2 => state.cr2,
        3 => state.cr3,
        4 => state.cr4,
        _ => 0,
    }
}

fn write_cr(state: &mut CpuState, mmu: &mut Mmu, cr: u8, value: u64) -> Result<(), CpuError> {
    match cr {
        0 => {
            state.cr0 = value;
            state.update_paging(mmu)?;
        }
        2 => state.cr2 = value,
        3 => {
            state.cr3 = value & 0x000F_FFFF_FFFF_F000;
            mmu.set_paging(true, state.cr3);
        }
        4 => state.cr4 = value,
        _ => {}
    }
    Ok(())
}

fn move_segment(state: &mut CpuState, code: u8, selector: u16) {
    state.seg_write(code, selector);
    // In real mode the hidden base of a data/code segment is selector << 4.
    if state.mode == CpuMode::Real16 && code <= 6 {
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
}

/// Push a value onto the stack, growing down.
fn push_value(state: &mut CpuState, mmu: &mut Mmu, value: u64, size: u8) -> Result<(), CpuError> {
    state.rsp = state.rsp.wrapping_sub(size as u64);
    mmu.write_to_addr(state.rsp, value, size).map_err(mem_err)
}

/// Pop a value off the stack, shrinking up.
fn pop_value(state: &mut CpuState, mmu: &mut Mmu, size: u8) -> Result<u64, CpuError> {
    let v = mmu.read_from_addr(state.rsp, size).map_err(mem_err)?;
    state.rsp = state.rsp.wrapping_add(size as u64);
    Ok(v)
}

// ---------------------------------------------------------------------------
// ALU operations with flags
// ---------------------------------------------------------------------------

fn alu_add(state: &mut CpuState, a: u64, b: u64, size: u8, carry_in: u64) -> u64 {
    let mask = operand_mask(size);
    let a128 = (a & mask) as u128;
    let b128 = (b & mask) as u128 + (carry_in & 1) as u128;
    let r128 = a128.wrapping_add(b128);
    let r = (r128 & mask as u128) as u64;
    let sign = sign_bit(size) as u128;
    let of = ((a128 ^ r128) & (b128 ^ r128) & sign) != 0;
    let af = (a128 ^ b128 ^ r128) & 0x10 != 0;
    write_flags(
        state,
        r128 > mask as u128,
        parity(r),
        af,
        (r128 & mask as u128) == 0,
        r128 & sign != 0,
        of,
    );
    r
}

fn alu_sub(state: &mut CpuState, a: u64, b: u64, size: u8, borrow_in: u64) -> u64 {
    let mask = operand_mask(size);
    let a128 = (a & mask) as u128;
    let b128 = (b & mask) as u128 + (borrow_in & 1) as u128;
    let r128 = a128.wrapping_sub(b128);
    let r = (r128 & mask as u128) as u64;
    let sign = sign_bit(size) as u128;
    let of = ((a128 ^ r128) & (a128 ^ b128) & sign) != 0;
    let af = (a128 ^ b128 ^ r128) & 0x10 != 0;
    write_flags(
        state,
        a128 < b128,
        parity(r),
        af,
        (r128 & mask as u128) == 0,
        r128 & sign != 0,
        of,
    );
    r
}

fn alu_logic(state: &mut CpuState, result: u64, size: u8) -> u64 {
    let mask = operand_mask(size);
    let r = result & mask;
    write_flags(
        state,
        false,
        parity(r),
        false,
        r == 0,
        r & sign_bit(size) != 0,
        false,
    );
    r
}

/// INC / DEC: update all flags except CF.
fn alu_inc_dec(state: &mut CpuState, value: u64, size: u8, increment: bool) -> u64 {
    let mask = operand_mask(size);
    let a128 = (value & mask) as u128;
    let r128 = if increment {
        a128.wrapping_add(1)
    } else {
        a128.wrapping_sub(1)
    };
    let r = (r128 & mask as u128) as u64;
    let sign = sign_bit(size) as u128;

    let of = if increment {
        a128 == sign - 1 // crossing from max-positive to min-negative
    } else {
        a128 == sign // crossing from min-negative to max-positive
    };
    let af = (a128 ^ 1 ^ r128) & 0x10 != 0;

    let mut f = state.rflags & !(PF | AF | ZF | SF | OF);
    if parity(r) {
        f |= PF;
    }
    if af {
        f |= AF;
    }
    if (r128 & mask as u128) == 0 {
        f |= ZF;
    }
    if r128 & sign != 0 {
        f |= SF;
    }
    if of {
        f |= OF;
    }
    // CF is preserved by INC/DEC.
    state.rflags = f;
    r
}

fn alu_neg(state: &mut CpuState, value: u64, size: u8) -> u64 {
    let r = alu_sub(state, 0, value, size, 0);
    // For NEG, CF is set unless the operand is zero.
    if value & operand_mask(size) == 0 {
        state.rflags &= !CF;
    } else {
        state.rflags |= CF;
    }
    // OF is set when the operand is the minimum signed value.
    if value & operand_mask(size) == sign_bit(size) {
        state.rflags |= OF;
    } else {
        state.rflags &= !OF;
    }
    r
}

// ---------------------------------------------------------------------------
// Flags / condition codes
// ---------------------------------------------------------------------------

fn condition_met(state: &CpuState, cond: u8) -> bool {
    let f = state.rflags;
    let cf = f & CF != 0;
    let pf = f & PF != 0;
    let zf = f & ZF != 0;
    let sf = f & SF != 0;
    let of = f & OF != 0;
    match cond {
        0x0 => of,                 // JO
        0x1 => !of,                // JNO
        0x2 => cf,                 // JC/JB/JNAE
        0x3 => !cf,                // JNC/JAE/JNB
        0x4 => zf,                 // JE/JZ
        0x5 => !zf,                // JNE/JNZ
        0x6 => cf || zf,           // JBE/JNA
        0x7 => !cf && !zf,         // JA/JNBE
        0x8 => sf,                 // JS
        0x9 => !sf,                // JNS
        0xA => pf,                 // JP/JPE
        0xB => !pf,                // JNP/JPO
        0xC => sf != of,           // JL/JNGE
        0xD => sf == of,           // JGE/JNL
        0xE => zf || sf != of,     // JLE/JNG
        0xF => !zf && sf == of,    // JG/JNLE
        _ => false,
    }
}

fn resolve_target(
    ins: &DecodedInstruction,
    state: &mut CpuState,
    mmu: &mut Mmu,
    op: &Operand,
) -> Result<u64, CpuError> {
    match op {
        Operand::Relative(rel) => Ok(ins.next_ip.wrapping_add(*rel as i64 as u64)),
        Operand::Immediate(v) => Ok(*v),
        Operand::Far { offset, selector } => {
            move_segment(state, 1, *selector);
            Ok(*offset)
        }
        _ => read_operand(ins, state, mmu, op),
    }
}

fn stack_operand_size(state: &CpuState, opsize: u8) -> u8 {
    match opsize {
        16 => 2,
        32 => {
            if state.mode == CpuMode::Long64 {
                8
            } else {
                4
            }
        }
        _ => 8,
    }
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

pub struct InstructionExecutor;

impl InstructionExecutor {
    pub fn new() -> Self {
        Self
    }

    pub fn execute(
        &self,
        instruction: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
        ports: &mut PortBus,
        bios: &mut BiosContext,
        apic: Option<&mut LocalApic>,
    ) -> Result<(), CpuError> {
        match instruction.mnemonic {
            "NOP" | "LFENCE" | "MFENCE" | "SFENCE" | "INVD" | "WBINVD" | "INVLPG"
            | "INVVPID" | "FXRSTOR" | "FXSAVE" => {
                state.rip = instruction.next_ip;
            }
            "MOV" => self.execute_mov(instruction, state, mmu)?,
            "LEA" => self.execute_lea(instruction, state)?,
            "ADD" => self.execute_alu(instruction, state, mmu, false, 0)?,
            "ADC" => self.execute_alu(instruction, state, mmu, false, 1)?,
            "SUB" => self.execute_alu(instruction, state, mmu, true, 0)?,
            "SBB" => self.execute_alu(instruction, state, mmu, true, 1)?,
            "AND" | "OR" | "XOR" => self.execute_logic(instruction, state, mmu)?,
            "CMP" => self.execute_cmp(instruction, state, mmu)?,
            "TEST" => self.execute_test(instruction, state, mmu)?,
            "INC" => self.execute_inc_dec(instruction, state, mmu, true)?,
            "DEC" => self.execute_inc_dec(instruction, state, mmu, false)?,
            "NOT" => self.execute_not(instruction, state, mmu)?,
            "NEG" => self.execute_neg(instruction, state, mmu)?,
            "MUL" => self.execute_mul(instruction, state, mmu, false)?,
            "IMUL" => self.execute_mul(instruction, state, mmu, true)?,
            "DIV" => self.execute_div(instruction, state, mmu, false)?,
            "IDIV" => self.execute_div(instruction, state, mmu, true)?,
            "PUSH" => self.execute_push(instruction, state, mmu)?,
            "POP" => self.execute_pop(instruction, state, mmu)?,
            "PUSHF" => self.execute_pushf(instruction, state, mmu)?,
            "POPF" => self.execute_popf(instruction, state, mmu)?,
            "JMP" => self.execute_jump(instruction, state, mmu)?,
            "JCC" => self.execute_jcc(instruction, state, mmu)?,
            "CALL" => self.execute_call(instruction, state, mmu, false)?,
            "CALLF" => self.execute_call(instruction, state, mmu, true)?,
            "RET" => self.execute_ret(instruction, state, mmu, false)?,
            "RETF" => self.execute_ret(instruction, state, mmu, true)?,
            "INT" | "INT3" => self.execute_int(instruction, state, mmu, intc, bios)?,
            "IRET" => self.execute_iret(instruction, state, mmu)?,
            "HLT" => {
                state.halted = true;
                state.rip = instruction.next_ip;
            }
            "LGDT" => self.execute_lgdt(instruction, state, mmu)?,
            "LIDT" => self.execute_lidt(instruction, state, mmu, intc)?,
            "SGDT" => self.execute_sgdt(instruction, state, mmu)?,
            "SIDT" => self.execute_sidt(instruction, state, mmu)?,
            "SMSW" => self.execute_smsw(instruction, state, mmu)?,
            "LMSW" => self.execute_lmsw(instruction, state, mmu)?,
            "MOVZX" => self.execute_movx(instruction, state, mmu, false)?,
            "MOVSX" | "MOVSXD" => self.execute_movx(instruction, state, mmu, true)?,
            "SHL" | "SHR" | "SAR" | "ROL" | "ROR" | "RCL" | "RCR" => {
                self.execute_shift(instruction, state, mmu)?
            }
            "MOVSB" | "MOVSW" | "MOVSD" | "MOVSQ" | "CMPSB" | "CMPSW" | "CMPSD" | "CMPSQ"
            | "STOSB" | "STOSW" | "STOSD" | "STOSQ" | "LODSB" | "LODSW" | "LODSD" | "LODSQ"
            | "SCASB" | "SCASW" | "SCASD" | "SCASQ" | "INSB" | "INSW" | "INSD" | "INSQ"
            | "OUTSB" | "OUTSW" | "OUTSD" | "OUTSQ" => {
                self.execute_string(instruction, state, mmu, ports)?
            }
            "IN" | "OUT" => self.execute_io(instruction, state, ports)?,
            "CPUID" => self.execute_cpuid(instruction, state)?,
            "WRMSR" | "RDMSR" => self.execute_msr(instruction, state, mmu, apic)?,
            "SYSCALL" | "SYSRET" => self.execute_syscall(instruction, state, mmu)?,
            "SYSENTER" | "SYSEXIT" => {
                state.rip = instruction.next_ip;
            }
            "XCHG" => self.execute_xchg(instruction, state, mmu)?,
            "CMPXCHG" => self.execute_cmpxchg(instruction, state, mmu)?,
            "XADD" => self.execute_xadd(instruction, state, mmu)?,
            "CMOVCC" => self.execute_cmovcc(instruction, state, mmu)?,
            "SETCC" => self.execute_setcc(instruction, state, mmu)?,
            "BSF" | "BSR" => self.execute_bit_scan(instruction, state, mmu)?,
            "CLC" => {
                state.rflags &= !CF;
                state.rip = instruction.next_ip;
            }
            "STC" => {
                state.rflags |= CF;
                state.rip = instruction.next_ip;
            }
            "CMC" => {
                state.rflags ^= CF;
                state.rip = instruction.next_ip;
            }
            "CLD" => {
                state.rflags &= !DF;
                state.rip = instruction.next_ip;
            }
            "STD" => {
                state.rflags |= DF;
                state.rip = instruction.next_ip;
            }
            "CLI" => {
                state.rflags &= !IF;
                state.interrupt_shadow = false;
                state.rip = instruction.next_ip;
            }
            "STI" => {
                state.rflags |= IF;
                // Interrupts are held for one instruction after STI.
                state.interrupt_shadow = true;
                state.rip = instruction.next_ip;
            }
            "CBW" | "CWDE" | "CDQE" | "CWD" | "CDQ" | "CQO" => {
                self.execute_sign_extend(instruction, state)?;
            }
            "SAHF" => self.execute_sahf(state)?,
            "LAHF" => self.execute_lahf(state)?,
            "LOOP" | "LOOPE" | "LOOPNE" | "JRCXZ" => {
                self.execute_loop(instruction, state)?;
            }
            _ => {
                state.rip = instruction.next_ip;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // MOV (all source/destination combinations)
    // ------------------------------------------------------------------

    fn execute_mov(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if ins.operands.is_empty() {
            state.rip = ins.next_ip;
            return Ok(());
        }

        match (&ins.operands[0], ins.operands.get(1)) {
            (Operand::ControlRegister(cr), Some(Operand::Register(reg))) => {
                let v = state.reg(*reg);
                write_cr(state, mmu, *cr, v)?;
            }
            (Operand::Register(reg), Some(Operand::ControlRegister(cr))) => {
                let v = read_cr(state, *cr);
                state.set_reg_size(*reg, operand_bytes(ins.opsize), v);
            }
            (Operand::Register(reg), Some(Operand::DebugRegister(dr))) => {
                let v = state.debug_reg(*dr);
                state.set_reg_size(*reg, operand_bytes(ins.opsize), v);
            }
            (Operand::DebugRegister(dr), Some(Operand::Register(reg))) => {
                let v = state.reg_size(*reg, operand_bytes(ins.opsize));
                // Debug registers are stored as 0; ignore writes.
                let _ = (dr, v);
            }
            (Operand::Segment(code), Some(src)) => {
                let sel = read_operand(ins, state, mmu, src)? as u16;
                move_segment(state, *code, sel);
            }
            (dst, Some(Operand::Segment(code))) => {
                let sel = state.seg_read(*code) as u64;
                write_operand(ins, state, mmu, dst, sel)?;
            }
            (dst, Some(src)) => {
                let v = read_operand(ins, state, mmu, src)?;
                write_operand(ins, state, mmu, dst, v)?;
            }
            _ => {}
        }

        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------

    fn execute_lea(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
    ) -> Result<(), CpuError> {
        if let (Some(Operand::Register(reg)), Some(Operand::Memory(mem))) =
            (ins.operands.first(), ins.operands.get(1))
        {
            let addr = effective_address(ins, state, mem);
            state.set_reg_size(*reg, operand_bytes(ins.opsize), addr);
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // ALU (ADD, ADC, SUB, SBB)
    // ------------------------------------------------------------------

    fn execute_alu(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        is_sub: bool,
        carry: u64,
    ) -> Result<(), CpuError> {
        if ins.operands.len() < 2 {
            state.rip = ins.next_ip;
            return Ok(());
        }
        let dst = ins.operands[0].clone();
        let src = ins.operands[1].clone();
        let a = read_operand(ins, state, mmu, &dst)?;
        let b = read_operand(ins, state, mmu, &src)?;
        let r = if is_sub {
            alu_sub(state, a, b, ins.opsize, carry)
        } else {
            alu_add(state, a, b, ins.opsize, carry)
        };
        write_operand(ins, state, mmu, &dst, r)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_logic(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if ins.operands.len() < 2 {
            state.rip = ins.next_ip;
            return Ok(());
        }
        let dst = ins.operands[0].clone();
        let src = ins.operands[1].clone();
        let a = read_operand(ins, state, mmu, &dst)?;
        let b = read_operand(ins, state, mmu, &src)?;
        let r = match ins.mnemonic {
            "AND" => a & b,
            "OR" => a | b,
            _ => a ^ b,
        };
        let r = alu_logic(state, r, ins.opsize);
        write_operand(ins, state, mmu, &dst, r)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_cmp(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if ins.operands.len() < 2 {
            state.rip = ins.next_ip;
            return Ok(());
        }
        let a = read_operand(ins, state, mmu, &ins.operands[0])?;
        let b = read_operand(ins, state, mmu, &ins.operands[1])?;
        let _ = alu_sub(state, a, b, ins.opsize, 0);
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_test(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if ins.operands.len() < 2 {
            state.rip = ins.next_ip;
            return Ok(());
        }
        let a = read_operand(ins, state, mmu, &ins.operands[0])?;
        let b = read_operand(ins, state, mmu, &ins.operands[1])?;
        let _ = alu_logic(state, a & b, ins.opsize);
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_inc_dec(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        increment: bool,
    ) -> Result<(), CpuError> {
        let op = ins.operands[0].clone();
        let v = read_operand(ins, state, mmu, &op)?;
        let r = alu_inc_dec(state, v, ins.opsize, increment);
        write_operand(ins, state, mmu, &op, r)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_not(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        let op = ins.operands[0].clone();
        let v = read_operand(ins, state, mmu, &op)?;
        let r = !v & operand_mask(ins.opsize);
        write_operand(ins, state, mmu, &op, r)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_neg(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        let op = ins.operands[0].clone();
        let v = read_operand(ins, state, mmu, &op)?;
        let r = alu_neg(state, v, ins.opsize);
        write_operand(ins, state, mmu, &op, r)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // MUL / IMUL
    // ------------------------------------------------------------------

    fn execute_mul(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        signed: bool,
    ) -> Result<(), CpuError> {
        // Single-operand MUL/IMUL.
        if ins.operands.len() == 1 {
            let rm = read_operand(ins, state, mmu, &ins.operands[0])?;
            let mask = operand_mask(ins.opsize);
            let m = rm & mask;
            let (_lo, hi) = match ins.opsize {
                8 => {
                    let a = (state.rax & 0xFF) as u64;
                    let p = if signed {
                        ((a as i8 as i64) * (m as i8 as i64)) as u64
                    } else {
                        a * m
                    };
                    // AL:AH = AX
                    state.rax = (state.rax & !0xFFFF) | (p & 0xFFFF);
                    (p & 0xFF, (p >> 8) & 0xFF)
                }
                16 => {
                    let a = state.rax & 0xFFFF;
                    let p = if signed {
                        ((a as i16 as i64) * (m as i16 as i64)) as u64
                    } else {
                        a as u64 * m as u64
                    };
                    state.rax = (state.rax & !0xFFFF_FFFF) | (p & 0xFFFF_FFFF);
                    state.rdx = (state.rdx & !0xFFFF) | ((p >> 16) & 0xFFFF);
                    (p & 0xFFFF, (p >> 32) & 0xFFFF)
                }
                32 => {
                    let a = state.rax & 0xFFFF_FFFF;
                    let p = if signed {
                        ((a as i32 as i64) * (m as i32 as i64)) as u64
                    } else {
                        a * m
                    };
                    state.rax = (state.rax & !0xFFFF_FFFF) | (p & 0xFFFF_FFFF);
                    state.rdx = (state.rdx & !0xFFFF_FFFF) | ((p >> 32) & 0xFFFF_FFFF);
                    (p & 0xFFFF_FFFF, (p >> 32) & 0xFFFF_FFFF)
                }
                _ => {
                    let a = state.rax;
                    let p = if signed {
                        let p = (a as i64 as i128) * (m as i64 as i128);
                        (p as u64, (p >> 64) as u64)
                    } else {
                        let p = (a as u128) * (m as u128);
                        (p as u64, (p >> 64) as u64)
                    };
                    state.rax = p.0;
                    state.rdx = p.1;
                    p
                }
            };
            let overflow = hi != 0;
            if overflow {
                state.rflags |= CF | OF;
            } else {
                state.rflags &= !(CF | OF);
            }
            state.rip = ins.next_ip;
            return Ok(());
        }

        // Two/three-operand IMUL: dest = rm * src (or rm * imm).
        let dst = ins.operands[0].clone();
        let rm = ins.operands[1].clone();
        let a = read_operand(ins, state, mmu, &rm)?;
        let b = if let Some(Operand::Immediate(v)) = ins.operands.get(2) {
            *v
        } else {
            read_operand(ins, state, mmu, &dst)?
        };
        let mask = operand_mask(ins.opsize);
        let a_s = (a & mask) as i64;
        let b_s = (b & mask) as i64;
        let (lo, hi) = if ins.opsize == 64 {
            let p = (a_s as i128) * (b_s as i128);
            (p as u64, (p >> 64) as u64)
        } else {
            let p = (a_s as i64).wrapping_mul((b & mask) as i64);
            let bits = bit_count(ins.opsize);
            let lo = (p as u64) & mask;
            let hi = p as u64 >> bits;
            (lo, hi)
        };
        write_operand(ins, state, mmu, &dst, lo)?;
        let overflow = if ins.opsize == 64 {
            hi != (if a_s.is_negative() { u64::MAX } else { 0 })
        } else {
            hi != 0 && hi != mask
        };
        if overflow {
            state.rflags |= CF | OF;
        } else {
            state.rflags &= !(CF | OF);
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // DIV / IDIV
    // ------------------------------------------------------------------

    fn execute_div(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        signed: bool,
    ) -> Result<(), CpuError> {
        let rm = read_operand(ins, state, mmu, &ins.operands[0])?;
        let mask = operand_mask(ins.opsize);

        let (quotient, remainder, overflow) = if signed {
            let (dividend, divisor): (i128, i128) = match ins.opsize {
                8 => ((state.rax & 0xFF) as i8 as i128, (rm & 0xFF) as i8 as i128),
                16 => ((state.rax & 0xFFFF) as i16 as i128, (rm & 0xFFFF) as i16 as i128),
                32 => (
                    ((state.rdx & 0xFFFF_FFFF) << 32 | (state.rax & 0xFFFF_FFFF)) as u64 as i64 as i128,
                    (rm & 0xFFFF_FFFF) as i32 as i128,
                ),
                _ => (
                    (((state.rdx as i64) as i128) << 64) | (state.rax as i128),
                    (rm & mask) as i64 as i128,
                ),
            };
            if divisor == 0 {
                return Err(CpuError::DivideError);
            }
            let q = dividend / divisor;
            let r = dividend % divisor;
            let q_min = match ins.opsize {
                8 => i128::from(i8::MIN),
                16 => i128::from(i16::MIN),
                32 => i128::from(i32::MIN),
                _ => i64::MIN as i128,
            };
            let q_max = match ins.opsize {
                8 => i128::from(i8::MAX),
                16 => i128::from(i16::MAX),
                32 => i128::from(i32::MAX),
                _ => i64::MAX as i128,
            };
            (
                q as u64,
                r as u64,
                q < q_min || q > q_max,
            )
        } else {
            let (dividend, divisor): (u128, u128) = match ins.opsize {
                8 => ((state.rax & 0xFFFF) as u128, (rm & 0xFF) as u128),
                16 => ((state.rax & 0xFFFF_FFFF) as u128, (rm & 0xFFFF) as u128),
                32 => (
                    (((state.rdx & 0xFFFF_FFFF) as u64) << 32 | (state.rax & 0xFFFF_FFFF)) as u128,
                    (rm & 0xFFFF_FFFF) as u128,
                ),
                _ => (
                    ((state.rdx as u128) << 64) | (state.rax as u128),
                    (rm & mask) as u128,
                ),
            };
            if divisor == 0 {
                return Err(CpuError::DivideError);
            }
            let q = dividend / divisor;
            let r = dividend % divisor;
            let q_max = match ins.opsize {
                8 => 0xFF,
                16 => 0xFFFF,
                32 => 0xFFFF_FFFF,
                _ => u64::MAX,
            };
            (q as u64, r as u64, q > q_max as u128)
        };

        if overflow {
            // #DE: quotient does not fit in the destination register.
            return Err(CpuError::DivideError);
        }

        match ins.opsize {
            8 => {
                state.rax = (state.rax & !0xFFFF) | (quotient & 0xFF) | ((remainder & 0xFF) << 8);
            }
            16 => {
                state.rax = (state.rax & !0xFFFF_FFFF) | (quotient & 0xFFFF) | ((remainder & 0xFFFF) << 16);
                state.rdx = (state.rdx & !0xFFFF) | (remainder & 0xFFFF);
            }
            32 => {
                state.rax = (state.rax & !0xFFFF_FFFF) | (quotient & 0xFFFF_FFFF);
                state.rdx = (state.rdx & !0xFFFF_FFFF) | (remainder & 0xFFFF_FFFF);
            }
            _ => {
                state.rax = quotient;
                state.rdx = remainder;
            }
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // PUSH / POP
    // ------------------------------------------------------------------

    fn execute_push(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        let size = stack_operand_size(state, ins.opsize);
        if ins.operands.is_empty() {
            state.rip = ins.next_ip;
            return Ok(());
        }
        match &ins.operands[0] {
            Operand::Register(r) => {
                let v = state.reg_size(*r, operand_bytes(ins.opsize));
                push_value(state, mmu, v, size)?;
            }
            Operand::Immediate(v) => {
                // In 64-bit mode, imm32 pushes are sign-extended to 64 bits.
                let val = if ins.opsize == 32 && state.mode == CpuMode::Long64 {
                    (*v as u32 as i32 as i64) as u64
                } else {
                    *v
                };
                push_value(state, mmu, val, size)?;
            }
            Operand::Segment(code) => {
                push_value(state, mmu, state.seg_read(*code) as u64, size)?;
            }
            op => {
                let v = read_operand(ins, state, mmu, op)?;
                push_value(state, mmu, v, size)?;
            }
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_pop(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if ins.operands.is_empty() {
            state.rip = ins.next_ip;
            return Ok(());
        }
        let size = stack_operand_size(state, ins.opsize);
        let op = ins.operands[0].clone();
        let v = pop_value(state, mmu, size)?;
        write_operand(ins, state, mmu, &op, v)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_pushf(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        let size = stack_operand_size(state, 64);
        push_value(state, mmu, state.rflags, size)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_popf(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        let size = stack_operand_size(state, 64);
        let v = pop_value(state, mmu, size)?;
        // Reserved bit (1) is always set.
        state.rflags = v | 0x2;
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Jumps / calls
    // ------------------------------------------------------------------

    fn execute_jump(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if let Some(op) = ins.operands.first() {
            let target = resolve_target(ins, state, mmu, op)?;
            state.rip = target;
        } else {
            state.rip = ins.next_ip;
        }
        Ok(())
    }

    fn execute_jcc(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if condition_met(state, ins.condition) {
            if let Some(Operand::Relative(rel)) = ins.operands.first() {
                state.rip = ins.next_ip.wrapping_add(*rel as i64 as u64);
            } else {
                state.rip = ins.next_ip;
            }
        } else {
            state.rip = ins.next_ip;
        }
        let _ = mmu;
        Ok(())
    }

    fn execute_call(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        far: bool,
    ) -> Result<(), CpuError> {
        if let Some(op) = ins.operands.first() {
            match op {
                Operand::Far { offset, selector } => {
                    push_value(state, mmu, state.cs.selector as u64, 8)?;
                    push_value(state, mmu, ins.next_ip, 8)?;
                    move_segment(state, 1, *selector);
                    state.rip = *offset;
                }
                _ => {
                    let target = resolve_target(ins, state, mmu, op)?;
                    push_value(state, mmu, ins.next_ip, 8)?;
                    state.rip = target;
                }
            }
        } else {
            state.rip = ins.next_ip;
        }
        let _ = far;
        Ok(())
    }

    fn execute_ret(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        far: bool,
    ) -> Result<(), CpuError> {
        let new_rip = pop_value(state, mmu, 8)?;
        if far {
            let new_cs = pop_value(state, mmu, 8)? as u16;
            move_segment(state, 1, new_cs);
        }
        if let Some(Operand::Immediate(v)) = ins.operands.first() {
            state.rsp = state.rsp.wrapping_add(*v);
        }
        state.rip = new_rip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Interrupts
    // ------------------------------------------------------------------

    fn execute_int(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
        bios: &mut BiosContext,
    ) -> Result<(), CpuError> {
        let vector = if ins.mnemonic == "INT3" {
            3
        } else if let Some(Operand::Immediate(v)) = ins.operands.first() {
            *v as u8
        } else {
            0
        };

        // If the guest has installed an IDT, deliver through it.
        if intc.idt_entry_address(vector).is_some() && state.idtr.limit != 0 {
            state.rip = ins.next_ip;
            state.deliver(vector, None, false, mmu, intc)?;
            return Ok(());
        }

        // Otherwise fall back to BIOS services (real-mode boot path).
        state.rip = ins.next_ip;
        let _ = bios.call_int(vector, state, mmu);
        Ok(())
    }

    fn execute_iret(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        let new_rip = pop_value(state, mmu, 8)?;
        let new_cs = pop_value(state, mmu, 8)? as u16;
        let new_flags = pop_value(state, mmu, 8)?;
        move_segment(state, 1, new_cs);
        state.rip = new_rip;
        // Preserve the reserved bit.
        state.rflags = new_flags | 0x2;
        let _ = ins;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Descriptor table instructions
    // ------------------------------------------------------------------

    fn execute_lgdt(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if let Some(Operand::Memory(mem)) = ins.operands.first() {
            let addr = effective_address(ins, state, mem);
            let limit = mmu.read_from_addr(addr, 2).map_err(mem_err)? as u16;
            let base = if state.mode == CpuMode::Long64 {
                mmu.read_from_addr(addr + 2, 8).map_err(mem_err)?
            } else {
                mmu.read_from_addr(addr + 2, 4).map_err(mem_err)? as u64
            };
            state.gdtr = crate::cpu::DescriptorTableRegister { base, limit };
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_lidt(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
    ) -> Result<(), CpuError> {
        if let Some(Operand::Memory(mem)) = ins.operands.first() {
            let addr = effective_address(ins, state, mem);
            let limit = mmu.read_from_addr(addr, 2).map_err(mem_err)? as u16;
            let base = if state.mode == CpuMode::Long64 {
                mmu.read_from_addr(addr + 2, 8).map_err(mem_err)?
            } else {
                mmu.read_from_addr(addr + 2, 4).map_err(mem_err)? as u64
            };
            state.idtr = crate::cpu::DescriptorTableRegister { base, limit };
            intc.set_idt(base, limit);
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    fn write_dtr(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        dtr: crate::cpu::DescriptorTableRegister,
    ) -> Result<(), CpuError> {
        if let Some(Operand::Memory(mem)) = ins.operands.first() {
            let addr = effective_address(ins, state, mem);
            mmu.write_to_addr(addr, dtr.limit as u64, 2).map_err(mem_err)?;
            if state.mode == CpuMode::Long64 {
                mmu.write_to_addr(addr + 2, dtr.base, 8).map_err(mem_err)?;
            } else {
                mmu.write_to_addr(addr + 2, dtr.base & 0xFFFF_FFFF, 4).map_err(mem_err)?;
            }
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_sgdt(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        self.write_dtr(ins, state, mmu, state.gdtr)
    }

    fn execute_sidt(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        self.write_dtr(ins, state, mmu, state.idtr)
    }

    fn execute_smsw(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        let op = ins.operands[0].clone();
        write_operand(ins, state, mmu, &op, state.cr0)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_lmsw(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        let v = read_operand(ins, state, mmu, &ins.operands[0])?;
        state.cr0 = (state.cr0 & !0x0F) | (v & 0x0F);
        state.update_paging(mmu)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // MOVZX / MOVSX / MOVSXD
    // ------------------------------------------------------------------

    fn execute_movx(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        sign_extend: bool,
    ) -> Result<(), CpuError> {
        if ins.operands.len() < 2 {
            state.rip = ins.next_ip;
            return Ok(());
        }
        let dst = ins.operands[0].clone();
        let src = ins.operands[1].clone();

        let src_size = match ins.mnemonic {
            "MOVSXD" => 4,
            _ => match ins.op2 {
                Some(0xB7) | Some(0xBF) => 2,
                _ => 1,
            },
        };

        let raw = read_operand_sized(ins, state, mmu, &src, src_size)?;

        let v = if sign_extend {
            match src_size {
                1 => (raw as u8 as i8) as i64 as u64,
                2 => (raw as u16 as i16) as i64 as u64,
                4 => (raw as u32 as i32) as i64 as u64,
                _ => raw,
            }
        } else {
            match src_size {
                1 => raw & 0xFF,
                2 => raw & 0xFFFF,
                _ => raw,
            }
        };

        let dest_size = if ins.mnemonic == "MOVSXD" { 64 } else { ins.opsize };
        write_operand_sized(state, &dst, dest_size, v);
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Shifts / rotates
    // ------------------------------------------------------------------

    fn execute_shift(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        let dst = ins.operands[0].clone();
        let v = read_operand(ins, state, mmu, &dst)?;
        let count_raw = match ins.operands.get(1) {
            Some(Operand::Immediate(c)) => *c,
            Some(Operand::Register(1)) => state.rcx & 0xFF,
            _ => 0,
        };
        let mask = operand_mask(ins.opsize);
        let bits = bit_count(ins.opsize);
        let count = if bits == 64 {
            (count_raw & 0x3F) as u32
        } else {
            (count_raw & 0x1F) as u32
        };
        let count_u64 = count as u64;

        let (result, new_cf) = if count == 0 {
            (v, state.rflags & CF != 0)
        } else {
            let sign = sign_bit(ins.opsize);
            match ins.mnemonic {
                "SHL" => {
                    let cf = (v >> (bits - count_u64)) & 1 != 0;
                    ((v << count) & mask, cf)
                }
                "SHR" => {
                    let cf = (v >> (count_u64 - 1)) & 1 != 0;
                    ((v >> count) & mask, cf)
                }
                "SAR" => {
                    let cf = (v >> (count_u64 - 1)) & 1 != 0;
                    let shifted = (v as i64) >> count;
                    ((shifted as u64) & mask, cf)
                }
                "ROL" => {
                    let r = ((v << count) | (v >> (bits - count_u64))) & mask;
                    (r, r & 1 != 0)
                }
                "ROR" => {
                    let r = ((v >> count) | (v << (bits - count_u64))) & mask;
                    (r, r & sign != 0)
                }
                "RCL" => {
                    let carry_in = (state.rflags & CF != 0) as u64;
                    let top = (v >> (bits - count_u64 - 1)) & ((1u64 << count) - 1);
                    let cf = if count == 1 { (v & sign) != 0 } else { (v >> (bits - count_u64)) & 1 != 0 };
                    let r = ((v << count) | (top << 1) | carry_in) & mask;
                    let _ = cf;
                    (r, ((v >> (bits - count_u64)) & 1) != 0)
                }
                "RCR" => {
                    let r = ((v >> count) | ((v & ((1u64 << count) - 1)) << (bits - count_u64 + 1))) & mask;
                    (r, (v >> (count_u64 - 1)) & 1 != 0)
                }
                _ => (v, false),
            }
        };

        // OF is defined only for count == 1.
        if count == 1 {
            let sign = sign_bit(ins.opsize);
            let of = match ins.mnemonic {
                "SHL" => (result & sign) != (v & sign),
                "SHR" => (result & sign) != 0,
                "SAR" => false,
                "ROL" => ((result & sign) != 0) != (result & 1 != 0),
                "ROR" => ((result & sign) != 0) != (v & sign != 0),
                _ => false,
            };
            if of {
                state.rflags |= OF;
            } else {
                state.rflags &= !OF;
            }
        }

        // SF/ZF/PF from the result; AF is undefined.
        write_flags(
            state,
            new_cf,
            parity(result),
            false,
            result == 0 && count != 0,
            result & sign_bit(ins.opsize) != 0,
            if count == 1 { state.rflags & OF != 0 } else { false },
        );

        // Restore OF if count != 1 (write_flags cleared it).
        if count != 1 {
            state.rflags &= !OF;
        }

        write_operand(ins, state, mmu, &dst, result)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // String instructions (with optional REP)
    // ------------------------------------------------------------------

    fn execute_string(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        ports: &mut PortBus,
    ) -> Result<(), CpuError> {
        let step = match ins.mnemonic {
            "MOVSB" | "CMPSB" | "STOSB" | "LODSB" | "SCASB" | "INSB" | "OUTSB" => 1u64,
            "MOVSW" | "CMPSW" | "STOSW" | "LODSW" | "SCASW" | "INSW" | "OUTSW" => 2,
            "MOVSD" | "CMPSD" | "STOSD" | "LODSD" | "SCASD" | "INSD" | "OUTSD" => 4,
            _ => 8,
        };
        let dir: i64 = if state.rflags & DF != 0 { -1 } else { 1 };
        let delta = (step as i64) * dir;

        let rep = ins.rep_prefix;
        let mut rcx = if rep.is_some() { state.rcx } else { 1 };
        if rep.is_some() && rcx == 0 {
            state.rip = ins.next_ip;
            return Ok(());
        }

        let is_movs = ins.mnemonic.starts_with("MOVS");
        let is_cmps = ins.mnemonic.starts_with("CMPS");
        let is_stos = ins.mnemonic.starts_with("STOS");
        let is_lods = ins.mnemonic.starts_with("LODS");
        let is_scas = ins.mnemonic.starts_with("SCAS");
        let is_ins = ins.mnemonic.starts_with("INS");
        let is_outs = ins.mnemonic.starts_with("OUTS");

        let mut rsi = state.rsi;
        let mut rdi = state.rdi;
        let port = state.rdx as u16;

        while rcx > 0 {
            if is_movs {
                let v = mmu.read_from_addr(rsi, step as u8).map_err(mem_err)?;
                mmu.write_to_addr(rdi, v, step as u8).map_err(mem_err)?;
                rsi = (rsi as i64 + delta) as u64;
                rdi = (rdi as i64 + delta) as u64;
            } else if is_stos {
                let v = match step {
                    1 => state.rax & 0xFF,
                    2 => state.rax & 0xFFFF,
                    4 => state.rax & 0xFFFF_FFFF,
                    _ => state.rax,
                };
                mmu.write_to_addr(rdi, v, step as u8).map_err(mem_err)?;
                rdi = (rdi as i64 + delta) as u64;
            } else if is_lods {
                let v = mmu.read_from_addr(rsi, step as u8).map_err(mem_err)?;
                state.set_reg_size(0, step as u8, v);
                rsi = (rsi as i64 + delta) as u64;
            } else if is_cmps {
                let a = mmu.read_from_addr(rsi, step as u8).map_err(mem_err)?;
                let b = mmu.read_from_addr(rdi, step as u8).map_err(mem_err)?;
                let _ = alu_sub(state, a, b, (step * 8) as u8, 0);
                rsi = (rsi as i64 + delta) as u64;
                rdi = (rdi as i64 + delta) as u64;
                if rep == Some(true) && state.rflags & ZF == 0 {
                    break;
                }
                if rep == Some(false) && state.rflags & ZF != 0 {
                    break;
                }
            } else if is_scas {
                let a = match step {
                    1 => state.rax & 0xFF,
                    2 => state.rax & 0xFFFF,
                    4 => state.rax & 0xFFFF_FFFF,
                    _ => state.rax,
                };
                let b = mmu.read_from_addr(rdi, step as u8).map_err(mem_err)?;
                let _ = alu_sub(state, a, b, (step * 8) as u8, 0);
                rdi = (rdi as i64 + delta) as u64;
                if rep == Some(true) && state.rflags & ZF == 0 {
                    break;
                }
                if rep == Some(false) && state.rflags & ZF != 0 {
                    break;
                }
            } else if is_ins {
                let v = ports.read(port, step as u8).map_err(port_err)?;
                mmu.write_to_addr(rdi, v, step as u8).map_err(mem_err)?;
                rdi = (rdi as i64 + delta) as u64;
            } else if is_outs {
                let v = mmu.read_from_addr(rsi, step as u8).map_err(mem_err)?;
                ports.write(port, v, step as u8).map_err(port_err)?;
                rsi = (rsi as i64 + delta) as u64;
            }
            rcx -= 1;
        }

        state.rsi = rsi;
        state.rdi = rdi;
        if rep.is_some() {
            state.rcx = rcx;
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Port I/O
    // ------------------------------------------------------------------

    fn execute_io(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        ports: &mut PortBus,
    ) -> Result<(), CpuError> {
        let port_operand = match ins.mnemonic {
            "OUT" => ins.operands.first(),
            _ => ins.operands.get(1),
        };
        let port = match port_operand {
            Some(Operand::Immediate(v)) => *v as u16,
            Some(Operand::Register(2)) => state.rdx as u16,
            _ => 0,
        };
        let size = match ins.opsize {
            8 => 1,
            64 => 8,
            16 => 2,
            _ => 4,
        };

        match ins.mnemonic {
            "IN" => {
                let v = ports.read(port, size).map_err(port_err)?;
                let reg = match ins.operands.first() {
                    Some(Operand::Register(r)) => *r,
                    _ => 0,
                };
                state.set_reg_size(reg, size, v);
            }
            "OUT" => {
                let v = match ins.operands.get(1) {
                    Some(Operand::Register(0)) => state.reg_size(0, size),
                    Some(Operand::Immediate(v)) => *v,
                    _ => 0,
                };
                ports.write(port, v, size).map_err(port_err)?;
            }
            _ => {}
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // MSRs
    // ------------------------------------------------------------------

    fn execute_cpuid(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
    ) -> Result<(), CpuError> {
        let leaf = state.rax as u32;
        let (eax, ebx, ecx, edx): (u32, u32, u32, u32) = match leaf {
            0 => (1, 0x534F_6E53, 0x20204D56, 0x0000_0000), // "SynOSVM  "
            1 => (0x0000_0601, 0, 0, (1 << 4) | (1 << 25) | (1 << 26) | (1 << 29)),
            0x8000_0000 => (0x8000_0001, 0, 0, 0),
            0x8000_0001 => (0, 0, 0, (1 << 26) | (1 << 29)),
            _ => (0, 0, 0, 0),
        };
        state.rax = eax as u64;
        state.rbx = ebx as u64;
        state.rcx = ecx as u64;
        state.rdx = edx as u64;
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_msr(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        apic: Option<&mut LocalApic>,
    ) -> Result<(), CpuError> {
        let msr = state.rcx & 0xFFFF_FFFF;
        let value = ((state.rdx & 0xFFFF_FFFF) << 32) | (state.rax & 0xFFFF_FFFF);

        match ins.mnemonic {
            "WRMSR" => match msr {
                0xC000_0080 => {
                    // EFER
                    state.efer = value;
                    state.update_paging(mmu)?;
                }
                0xC000_0081 => state.star = value,
                0xC000_0082 => state.lstar = value & !0xFFF,
                0xC000_0083 => { /* FMASK - ignored */ }
                0xC000_0100 => state.fs_base = value,
                0xC000_0101 => state.gs_base = value,
                0xC000_0102 => { /* KERNEL_GS_BASE - ignored */ }
                // IA32_APIC_BASE routes to the shared local APIC so the MSR
                // view and the MMIO view stay in sync.
                IA32_APIC_BASE_MSR_U64 => {
                    if let Some(apic) = apic {
                        apic.set_apic_base_msr(value);
                    }
                }
                _ => { /* Unknown MSRs are accepted and ignored. */ }
            },
            "RDMSR" => {
                let v = if msr == IA32_APIC_BASE_MSR_U64 {
                    apic.map(|a| a.apic_base_msr()).unwrap_or(0)
                } else {
                    match msr {
                        0xC000_0080 => state.efer,
                        0xC000_0081 => state.star,
                        0xC000_0082 => state.lstar,
                        0xC000_0100 => state.fs_base,
                        0xC000_0101 => state.gs_base,
                        _ => 0,
                    }
                };
                state.rax = (state.rax & !0xFFFF_FFFF) | (v & 0xFFFF_FFFF);
                state.rdx = (state.rdx & !0xFFFF_FFFF) | ((v >> 32) & 0xFFFF_FFFF);
            }
            _ => {}
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // SYSCALL / SYSRET
    // ------------------------------------------------------------------

    fn execute_syscall(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if ins.mnemonic == "SYSCALL" {
            state.rcx = ins.next_ip;
            state.r11 = state.rflags;
            state.rip = state.lstar;
            state.privilege = PrivilegeLevel::Ring3;
            mmu.set_privilege(true);
        } else {
            // SYSRET returns to the caller.
            state.rip = state.rcx;
            state.rflags = state.r11 | 0x2;
            state.privilege = PrivilegeLevel::Ring0;
            mmu.set_privilege(false);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // XCHG / CMPXCHG / SETCC / BSF / BSR
    // ------------------------------------------------------------------

    fn execute_xchg(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if ins.operands.len() < 2 {
            state.rip = ins.next_ip;
            return Ok(());
        }
        let a = ins.operands[0].clone();
        let b = ins.operands[1].clone();
        let va = read_operand(ins, state, mmu, &a)?;
        let vb = read_operand(ins, state, mmu, &b)?;
        write_operand(ins, state, mmu, &a, vb)?;
        write_operand(ins, state, mmu, &b, va)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_cmpxchg(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if ins.operands.len() < 2 {
            state.rip = ins.next_ip;
            return Ok(());
        }
        let dst = ins.operands[0].clone();
        let src = ins.operands[1].clone();
        let mask = operand_mask(ins.opsize);
        let dst_val = read_operand(ins, state, mmu, &dst)? & mask;
        let acc = state.rax & mask;

        if acc == dst_val {
            let src_val = read_operand(ins, state, mmu, &src)?;
            write_operand(ins, state, mmu, &dst, src_val)?;
            state.rflags |= ZF;
        } else {
            state.rax = (state.rax & !mask) | dst_val;
            state.rflags &= !ZF;
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_xadd(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if ins.operands.len() < 2 {
            state.rip = ins.next_ip;
            return Ok(());
        }
        let dst = ins.operands[0].clone();
        let src = ins.operands[1].clone();
        let old_dst = read_operand(ins, state, mmu, &dst)?;
        let src_value = read_operand(ins, state, mmu, &src)?;
        let result = alu_add(state, old_dst, src_value, ins.opsize, 0);
        write_operand(ins, state, mmu, &dst, result)?;
        write_operand(ins, state, mmu, &src, old_dst)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_setcc(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        let op = ins.operands[0].clone();
        let v = if condition_met(state, ins.condition) { 1u64 } else { 0 };
        write_operand(ins, state, mmu, &op, v)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_cmovcc(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if ins.operands.len() < 2 {
            state.rip = ins.next_ip;
            return Ok(());
        }
        let value = read_operand(ins, state, mmu, &ins.operands[1])?;
        if condition_met(state, ins.condition) {
            let destination = ins.operands[0].clone();
            write_operand(ins, state, mmu, &destination, value)?;
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_bit_scan(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), CpuError> {
        if ins.operands.len() < 2 {
            state.rip = ins.next_ip;
            return Ok(());
        }
        let dst = ins.operands[0].clone();
        let src = ins.operands[1].clone();
        let v = read_operand(ins, state, mmu, &src)? & operand_mask(ins.opsize);

        if v == 0 {
            state.rflags |= ZF;
            state.rip = ins.next_ip;
            return Ok(());
        }
        state.rflags &= !ZF;
        let idx = if ins.mnemonic == "BSF" {
            v.trailing_zeros() as u64
        } else {
            (bit_count(ins.opsize) - 1) - (v.leading_zeros() as u64)
        };
        write_operand(ins, state, mmu, &dst, idx)?;
        state.rip = ins.next_ip;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Misc flag / sign operations
    // ------------------------------------------------------------------

    fn execute_sign_extend(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
    ) -> Result<(), CpuError> {
        match ins.mnemonic {
            "CBW" => {
                state.rax = (state.rax & !0xFFFF) | ((state.rax as u8 as i8) as i16 as u16 as u64);
            }
            "CWDE" => {
                state.rax = ((state.rax as u16 as i16) as i32 as u32) as u64;
            }
            "CDQE" => {
                state.rax = ((state.rax as u32 as i32) as i64) as u64;
            }
            "CWD" => {
                let v = if state.rax & 0x8000 != 0 { 0xFFFF } else { 0 };
                state.rdx = (state.rdx & !0xFFFF) | v;
            }
            "CDQ" => {
                let v = if state.rax & 0x8000_0000 != 0 {
                    0xFFFF_FFFF
                } else {
                    0
                };
                state.rdx = (state.rdx & !0xFFFF_FFFF) | v;
            }
            "CQO" => {
                state.rdx = if state.rax & (1 << 63) != 0 { u64::MAX } else { 0 };
            }
            _ => {}
        }
        state.rip = ins.next_ip;
        Ok(())
    }

    fn execute_sahf(&self, state: &mut CpuState) -> Result<(), CpuError> {
        let ah = (state.rax >> 8) & 0xFF;
        let mut f = state.rflags & !(CF | PF | AF | ZF | SF);
        if ah & 0x01 != 0 {
            f |= CF;
        }
        if ah & 0x04 != 0 {
            f |= PF;
        }
        if ah & 0x10 != 0 {
            f |= AF;
        }
        if ah & 0x40 != 0 {
            f |= ZF;
        }
        if ah & 0x80 != 0 {
            f |= SF;
        }
        state.rflags = f;
        Ok(())
    }

    fn execute_lahf(&self, state: &mut CpuState) -> Result<(), CpuError> {
        let f = state.rflags;
        let mut ah = 0u8;
        if f & CF != 0 {
            ah |= 0x01;
        }
        if f & PF != 0 {
            ah |= 0x04;
        }
        if f & AF != 0 {
            ah |= 0x10;
        }
        if f & ZF != 0 {
            ah |= 0x40;
        }
        if f & SF != 0 {
            ah |= 0x80;
        }
        state.rax = (state.rax & !0xFF00) | ((ah as u64) << 8);
        Ok(())
    }

    fn execute_loop(
        &self,
        ins: &DecodedInstruction,
        state: &mut CpuState,
    ) -> Result<(), CpuError> {
        let rel = match ins.operands.first() {
            Some(Operand::Relative(r)) => *r,
            _ => 0,
        };

        if ins.mnemonic == "JRCXZ" {
            if state.rcx == 0 {
                state.rip = ins.next_ip.wrapping_add(rel as i64 as u64);
            } else {
                state.rip = ins.next_ip;
            }
            return Ok(());
        }

        let old = state.rcx;
        state.rcx = state.rcx.wrapping_sub(1);
        let take = match ins.mnemonic {
            "LOOP" => state.rcx != 0,
            "LOOPE" => state.rcx != 0 && state.rflags & ZF != 0,
            "LOOPNE" => state.rcx != 0 && state.rflags & ZF == 0,
            _ => false,
        };
        let _ = old;
        if take {
            state.rip = ins.next_ip.wrapping_add(rel as i64 as u64);
        } else {
            state.rip = ins.next_ip;
        }
        Ok(())
    }
}

fn write_operand_sized(state: &mut CpuState, op: &Operand, size: u8, value: u64) {
    if let Operand::Register(r) = op {
        state.set_reg_size(*r, operand_bytes(size), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::Cpu;
    use crate::devices::{PortBus, Serial16550};

    fn cpu_with(bytes: &[u8]) -> (Cpu, Mmu) {
        let mut cpu = Cpu::new();
        let mut mmu = Mmu::new(1 << 20);
        // Enter long mode so execution uses the 64-bit path.
        cpu.state.efer |= 1 << 8; // LME
        mmu.write_phys(0x1000, bytes).unwrap();
        cpu.set_rip(0x1000);
        // Keep paging disabled with an identity map so the decoder can read
        // the instruction bytes at physical RIP.
        cpu.state.cr0 |= 1; // PE
        (cpu, mmu)
    }

    fn run_until(cpu: &mut Cpu, mmu: &mut Mmu, max_steps: usize) -> Result<(), CpuError> {
        let mut intc = InterruptController::new();
        let mut ports = PortBus::new();
        ports.attach(0x3F8, 8, Box::new(Serial16550::new(0x3F8)));
        let mut bios = BiosContext::new();
        for _ in 0..max_steps {
            cpu.step(mmu, &mut intc, &mut ports, &mut bios)?;
        }
        Ok(())
    }

    fn single_run(cpu: &mut Cpu, mmu: &mut Mmu) -> Result<(), CpuError> {
        run_until(cpu, mmu, 1)
    }

    #[test]
    fn mov_register_and_memory() {
        let (mut cpu, mut mmu) = cpu_with(&[0x48, 0xB8, 0x78, 0x56, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00]); // mov rax, 0x12345678
        single_run(&mut cpu, &mut mmu).unwrap();
        assert_eq!(cpu.state.rax, 0x1234_5678);
        assert_eq!(cpu.state.rip, 0x100A);

        // mov rbx, rax
        let (mut cpu, mut mmu) = cpu_with(&[0x48, 0x89, 0xC3]);
        cpu.state.rax = 0xABCD;
        single_run(&mut cpu, &mut mmu).unwrap();
        assert_eq!(cpu.state.rbx, 0xABCD);
    }

    #[test]
    fn add_with_flags() {
        // add rax, 1  (48 83 C0 01)
        let (mut cpu, mut mmu) = cpu_with(&[0x48, 0x83, 0xC0, 0x01]);
        cpu.state.rax = 0xFFFF;
        single_run(&mut cpu, &mut mmu).unwrap();
        assert_eq!(cpu.state.rax, 0x1_0000);
        assert_eq!(cpu.state.rip, 0x1004);
    }

    #[test]
    fn sub_sets_flags() {
        // sub rax, 1 (48 83 E8 01)
        let (mut cpu, mut mmu) = cpu_with(&[0x48, 0x83, 0xE8, 0x01]);
        cpu.state.rax = 1;
        single_run(&mut cpu, &mut mmu).unwrap();
        assert_eq!(cpu.state.rax, 0);
        assert_ne!(cpu.state.rflags & ZF, 0);
    }

    #[test]
    fn push_pop_roundtrip() {
        // push rax (50), pop rbx (58)
        let (mut cpu, mut mmu) = cpu_with(&[0x50, 0x5B]);
        cpu.state.rax = 0xDEAD_BEEF;
        cpu.state.rsp = 0x8000;
        run_until(&mut cpu, &mut mmu, 2).unwrap();
        assert_eq!(cpu.state.rbx, 0xDEAD_BEEF);
        assert_eq!(cpu.state.rsp, 0x8000);
        assert_eq!(cpu.state.rip, 0x1002);
    }

    #[test]
    fn jcc_taken_with_zero_flag() {
        // xor rax, rax (48 31 C0), test rax, rax (48 85 C0),
        // jz +1 near (0F 84 01 00 00 00), nop, nop
        let (mut cpu, mut mmu) = cpu_with(&[0x48, 0x31, 0xC0, 0x48, 0x85, 0xC0, 0x0F, 0x84, 0x01, 0x00, 0x00, 0x00, 0x90, 0x90]);
        run_until(&mut cpu, &mut mmu, 3).unwrap();
        assert_eq!(cpu.state.rax, 0);
        assert_ne!(cpu.state.rflags & ZF, 0);
        // jz next_ip = 0x100C + 1 = 0x100D (the second NOP).
        assert_eq!(cpu.state.rip, 0x100D);
    }

    #[test]
    fn call_and_ret() {
        // call +2 (e8 02 00 00 00) -> 0x1007, nop, nop,
        // mov rax, 42 (48 c7 c0 2a 00 00 00), ret (c3)
        let code = [
            0xE8, 0x02, 0x00, 0x00, 0x00, // call +2 -> 0x1007
            0x90,                         // 0x1005 nop
            0x90,                         // 0x1006 nop
            0x48, 0xC7, 0xC0, 0x2A, 0x00, 0x00, 0x00, // 0x1007 mov rax, 42
            0xC3,                         // 0x100E ret
        ];
        let (mut cpu, mut mmu) = cpu_with(&code);
        cpu.state.rsp = 0x9000;
        run_until(&mut cpu, &mut mmu, 5).unwrap();
        assert_eq!(cpu.state.rax, 42);
        assert_eq!(cpu.state.rsp, 0x9000);
    }

    #[test]
    fn mul_and_div() {
        // mov rax, 6 (b8 06 00 00 00); mov rbx, 3 (bb 03 00 00 00); mul rbx (48 f7 e3); div rbx (48 f7 f3)
        let code = [
            0x48, 0xB8, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x48, 0xBB, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x48, 0xF7, 0xE3, // mul rbx -> rax = 18
            0x48, 0xF7, 0xF3, // div rbx -> rax = 6, rdx = 0
        ];
        let (mut cpu, mut mmu) = cpu_with(&code);
        run_until(&mut cpu, &mut mmu, 4).unwrap();
        assert_eq!(cpu.state.rax, 6);
        assert_eq!(cpu.state.rdx, 0);
        // Let the earlier mul be verified: after 3rd step rax==18.
        let (mut cpu, mut mmu) = cpu_with(&code);
        run_until(&mut cpu, &mut mmu, 3).unwrap();
        assert_eq!(cpu.state.rax, 18);
        assert_eq!(cpu.state.rdx, 0);
    }

    #[test]
    fn string_movs_with_rep() {
        // Setup: rsi=0x2000, rdi=0x3000, rcx=4
        // rep movsb: f3 a4
        let (mut cpu, mut mmu) = cpu_with(&[0xF3, 0xA4]);
        cpu.state.rsi = 0x2000;
        cpu.state.rdi = 0x3000;
        cpu.state.rcx = 4;
        mmu.write_phys(0x2000, &[1, 2, 3, 4]).unwrap();
        single_run(&mut cpu, &mut mmu).unwrap();
        assert_eq!(mmu.read_phys(0x3000, 4).unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(cpu.state.rcx, 0);
        assert_eq!(cpu.state.rsi, 0x2004);
        assert_eq!(cpu.state.rdi, 0x3004);
    }

    #[test]
    fn port_io_serial() {
        // mov al, 'A' (b0 41); out 0x3F8, al (e6 f8 is imm8 port: out imm8, al -> E6 F8)
        let (mut cpu, mut mmu) = cpu_with(&[0xB0, 0x41, 0xE6, 0xF8]);
        run_until(&mut cpu, &mut mmu, 2).unwrap();
        assert_eq!(cpu.state.rax & 0xFF, b'A' as u64);
        assert_eq!(cpu.state.rip, 0x1004);
    }
}
