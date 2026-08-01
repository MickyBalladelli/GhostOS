//! x86_64 instruction decoder: legacy prefixes, REX, ModR/M, SIB,
//! displacements, immediates, and two-byte opcode groups.

use crate::memory::Mmu;

#[derive(Clone, Debug)]
pub struct MemoryOperand {
    pub base: Option<u8>,
    pub index: Option<u8>,
    pub scale: u8,
    pub displacement: i32,
    /// RIP-relative addressing (64-bit mode, ModRM mod=00 r/m=101).
    pub rip_relative: bool,
    /// Segment override for the access: 0=default, 1=CS, 2=DS, 3=ES, 4=FS,
    /// 5=GS, 6=SS.
    pub segment: u8,
}

#[derive(Clone, Debug)]
pub enum Operand {
    Register(u8),
    Memory(MemoryOperand),
    Immediate(u64),
    /// Relative displacement to be added to `next_ip` by the executor.
    Relative(i32),
    Far { offset: u64, selector: u16 },
    Segment(u8),
    ControlRegister(u8),
    DebugRegister(u8),
}

#[derive(Clone, Debug)]
pub struct DecodedInstruction {
    /// Address of the first prefix/opcode byte.
    pub ip: u64,
    pub opcode: u8,
    /// Second opcode byte when the stream begins with 0x0F.
    pub op2: Option<u8>,
    pub mnemonic: &'static str,
    /// Address of the byte following the last operand.
    pub next_ip: u64,
    /// Operand size in bits: 8, 16, 32, or 64.
    pub opsize: u8,
    /// Address size in bits: 32 or 64.
    pub addrsize: u8,
    /// Whether any REX prefix preceded the opcode.
    pub has_rex: bool,
    /// Condition code for Jcc/SETcc (0x0..0xF).
    pub condition: u8,
    /// REP prefix state: None = none, Some(true) = REPE/REPZ,
    /// Some(false) = REPNE/REPNZ.
    pub rep_prefix: Option<bool>,
    pub operands: Vec<Operand>,
}

impl DecodedInstruction {
    pub fn new(ip: u64, opcode: u8) -> Self {
        Self {
            ip,
            opcode,
            op2: None,
            mnemonic: "UNKNOWN",
            next_ip: ip,
            opsize: 32,
            addrsize: 64,
            has_rex: false,
            condition: 0,
            rep_prefix: None,
            operands: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum InstructionDecodeError {
    InvalidOpcode,
    UnsupportedMode,
    InvalidPrefix,
    InvalidModRm,
    InvalidSib,
    InvalidDisplacement,
    InvalidImmediate,
    OutOfMemory,
}

pub struct InstructionDecoder;

#[derive(Clone, Copy, Default)]
struct Rex {
    w: bool,
    r: bool,
    x: bool,
    b: bool,
}

const MAX_INSN_LEN: usize = 15;

impl InstructionDecoder {
    pub fn new() -> Self {
        Self
    }

    pub fn decode(
        &self,
        ip: u64,
        mmu: &Mmu,
    ) -> Result<DecodedInstruction, InstructionDecodeError> {
        self.decode_inner(ip, mmu)
    }

    pub(crate) fn rd(mmu: &Mmu, pos: &mut u64) -> Result<u8, InstructionDecodeError> {
        let b = mmu
            .read_byte(*pos)
            .map_err(|_| InstructionDecodeError::OutOfMemory)?;
        *pos += 1;
        Ok(b)
    }

    pub(crate) fn rd16(mmu: &Mmu, pos: &mut u64) -> Result<u16, InstructionDecodeError> {
        let lo = Self::rd(mmu, pos)? as u16;
        let hi = Self::rd(mmu, pos)? as u16;
        Ok(lo | (hi << 8))
    }

    pub(crate) fn rd32(mmu: &Mmu, pos: &mut u64) -> Result<u32, InstructionDecodeError> {
        let a = Self::rd16(mmu, pos)? as u32;
        let b = Self::rd16(mmu, pos)? as u32;
        Ok(a | (b << 16))
    }

    pub(crate) fn rd64(mmu: &Mmu, pos: &mut u64) -> Result<u64, InstructionDecodeError> {
        let mut v = 0u64;
        for i in 0..8 {
            let b = Self::rd(mmu, pos)? as u64;
            v |= b << (i * 8);
        }
        Ok(v)
    }

    fn read_modrm(mmu: &Mmu, pos: &mut u64) -> Result<(u8, u8, u8), InstructionDecodeError> {
        let b = Self::rd(mmu, pos)?;
        Ok(((b >> 6) & 0x03, (b >> 3) & 0x07, b & 0x07))
    }

    fn read_imm(
        mmu: &Mmu,
        pos: &mut u64,
        size: u8,
        sign_extend: bool,
    ) -> Result<u64, InstructionDecodeError> {
        let raw = match size {
            1 => Self::rd(mmu, pos)? as u64,
            2 => Self::rd16(mmu, pos)? as u64,
            4 => Self::rd32(mmu, pos)? as u64,
            8 => Self::rd64(mmu, pos)?,
            _ => return Err(InstructionDecodeError::InvalidImmediate),
        };
        Ok(if sign_extend {
            match size {
                1 => (raw as i8) as i64 as u64,
                2 => (raw as i16) as i64 as u64,
                4 => (raw as i32) as i64 as u64,
                _ => raw,
            }
        } else {
            raw
        })
    }

    fn decode_rm(
        &self,
        mmu: &Mmu,
        pos: &mut u64,
        mod_: u8,
        rm: u8,
        rex: Rex,
        addrsize: u8,
        segment: u8,
    ) -> Result<Operand, InstructionDecodeError> {
        if mod_ == 0b11 {
            let high = if rex.b { 8 } else { 0 };
            return Ok(Operand::Register(rm + high));
        }

        let is_64 = addrsize == 64;
        let mut base = Some(rm);
        let mut index: Option<u8> = None;
        let mut scale: u8 = 1;
        let mut displacement: i32 = 0;
        let rip_relative = false;

        if mod_ == 0b00 && rm == 0b101 {
            displacement = Self::rd32(mmu, pos)? as i32;
            return Ok(Operand::Memory(MemoryOperand {
                base: None,
                index: None,
                scale: 1,
                displacement,
                rip_relative: is_64,
                segment,
            }));
        }

        // The SIB byte (if any) precedes the displacement in the encoding.
        if rm == 0b100 {
            let sib = Self::rd(mmu, pos)?;
            scale = 1u8 << ((sib >> 6) & 0x03);
            let idx = (sib >> 3) & 0x07;
            let bas = sib & 0x07;

            if idx != 0b100 {
                let high = if rex.x { 8 } else { 0 };
                index = Some(idx + high);
            }
            if index == Some(4) || index == Some(12) {
                index = None;
            }

            if bas == 0b101 && mod_ == 0b00 {
                // No base register; disp32 follows.
                displacement = Self::rd32(mmu, pos)? as i32;
                base = None;
            } else {
                base = Some(bas);
                if rex.b {
                    base = base.map(|base| base + 8);
                }
            }
        } else if rex.b {
            base = base.map(|base| base + 8);
        }

        if mod_ == 0b01 {
            displacement = Self::rd(mmu, pos)? as i8 as i32;
        } else if mod_ == 0b10 {
            displacement = Self::rd32(mmu, pos)? as i32;
        }

        Ok(Operand::Memory(MemoryOperand {
            base,
            index,
            scale,
            displacement,
            rip_relative,
            segment,
        }))
    }

    fn decode_modrm_operands(
        &self,
        mmu: &Mmu,
        pos: &mut u64,
        rex: Rex,
        _opsize: u8,
        addrsize: u8,
        segment: u8,
        use_digit: bool,
    ) -> Result<(u8, Operand), InstructionDecodeError> {
        let (mod_, reg, rm) = Self::read_modrm(mmu, pos)?;
        let high = if rex.r { 8 } else { 0 };
        let reg_op = if use_digit { reg } else { reg + high };
        let rm_op = self.decode_rm(mmu, pos, mod_, rm, rex, addrsize, segment)?;
        Ok((reg_op, rm_op))
    }

    fn decode_inner(
        &self,
        ip: u64,
        mmu: &Mmu,
    ) -> Result<DecodedInstruction, InstructionDecodeError> {
        let mut pos = ip;
        let mut opsize: u8 = 32;
        let mut addrsize: u8 = 64;
        let mut segment: u8 = 0;
        let mut has_rex = false;
        let mut rex = Rex::default();
        let mut rep_prefix: Option<bool> = None;

        loop {
            if pos - ip >= MAX_INSN_LEN as u64 {
                return Err(InstructionDecodeError::InvalidPrefix);
            }
            let b = Self::rd(mmu, &mut pos)?;
            match b {
                0x66 => opsize = if opsize == 16 { 32 } else { 16 },
                0x67 => addrsize = if addrsize == 64 { 32 } else { 64 },
                0x26 => segment = 3,
                0x2E => segment = 1,
                0x36 => segment = 6,
                0x3E => segment = 2,
                0x64 => segment = 4,
                0x65 => segment = 5,
                0xF0 => {}
                0xF2 => rep_prefix = Some(false),
                0xF3 => rep_prefix = Some(true),
                0x40..=0x4F => {
                    has_rex = true;
                    rex = Rex {
                        w: b & 0x08 != 0,
                        r: b & 0x04 != 0,
                        x: b & 0x02 != 0,
                        b: b & 0x01 != 0,
                    };
                    if rex.w {
                        opsize = 64;
                    }
                }
                _ => {
                    pos -= 1;
                    break;
                }
            }
        }

        let opcode = Self::rd(mmu, &mut pos)?;
        let mut ins = DecodedInstruction::new(ip, opcode);
        ins.opsize = opsize;
        ins.addrsize = addrsize;
        ins.has_rex = has_rex;
        ins.rep_prefix = rep_prefix;

        if opcode == 0x0F {
            let op2 = Self::rd(mmu, &mut pos)?;
            ins.op2 = Some(op2);
            self.decode_two_byte(mmu, &mut pos, &mut ins, rex, segment)?;
        } else {
            self.decode_one_byte(mmu, &mut pos, &mut ins, rex, segment)?;
        }
        ins.next_ip = pos;
        Ok(ins)
    }

    // ------------------------------------------------------------------
    // Single-byte opcode map
    // ------------------------------------------------------------------

    fn decode_one_byte(
        &self,
        mmu: &Mmu,
        pos: &mut u64,
        ins: &mut DecodedInstruction,
        rex: Rex,
        segment: u8,
    ) -> Result<(), InstructionDecodeError> {
        let op = ins.opcode as usize;
        let opsize = ins.opsize;
        let addrsize = ins.addrsize;

        macro_rules! reg_rm {
            ($mnem:expr, $src_first:expr, $size8:expr) => {{
                ins.mnemonic = $mnem;
                if $size8 {
                    ins.opsize = 8;
                }
                let (reg, rm) = self.decode_modrm_operands(
                    mmu, pos, rex, ins.opsize, addrsize, segment, false,
                )?;
                if $src_first {
                    ins.operands = vec![Operand::Register(reg), rm];
                } else {
                    ins.operands = vec![rm, Operand::Register(reg)];
                }
                return Ok(());
            }};
        }

        macro_rules! alu_imm {
            ($mnem:expr, $size8:expr) => {{
                ins.mnemonic = $mnem;
                if $size8 {
                    ins.opsize = 8;
                }
                let imm_size = match ins.opsize {
                    8 => 1,
                    16 => 2,
                    _ => 4,
                };
                ins.operands = vec![
                    Operand::Register(0),
                    Operand::Immediate(Self::read_imm(
                        mmu,
                        pos,
                        imm_size,
                        opsize == 64 && imm_size == 4,
                    )?),
                ];
                return Ok(());
            }};
        }

        match op {
            0x00 => reg_rm!("ADD", false, true),
            0x01 => reg_rm!("ADD", false, false),
            0x02 => reg_rm!("ADD", true, true),
            0x03 => reg_rm!("ADD", true, false),
            0x04 => alu_imm!("ADD", true),
            0x05 => alu_imm!("ADD", false),
            0x08 => reg_rm!("OR", false, true),
            0x09 => reg_rm!("OR", false, false),
            0x0A => reg_rm!("OR", true, true),
            0x0B => reg_rm!("OR", true, false),
            0x0C => alu_imm!("OR", true),
            0x0D => alu_imm!("OR", false),
            0x10 => reg_rm!("ADC", false, true),
            0x11 => reg_rm!("ADC", false, false),
            0x12 => reg_rm!("ADC", true, true),
            0x13 => reg_rm!("ADC", true, false),
            0x14 => alu_imm!("ADC", true),
            0x15 => alu_imm!("ADC", false),
            0x18 => reg_rm!("SBB", false, true),
            0x19 => reg_rm!("SBB", false, false),
            0x1A => reg_rm!("SBB", true, true),
            0x1B => reg_rm!("SBB", true, false),
            0x1C => alu_imm!("SBB", true),
            0x1D => alu_imm!("SBB", false),
            0x20 => reg_rm!("AND", false, true),
            0x21 => reg_rm!("AND", false, false),
            0x22 => reg_rm!("AND", true, true),
            0x23 => reg_rm!("AND", true, false),
            0x24 => alu_imm!("AND", true),
            0x25 => alu_imm!("AND", false),
            0x28 => reg_rm!("SUB", false, true),
            0x29 => reg_rm!("SUB", false, false),
            0x2A => reg_rm!("SUB", true, true),
            0x2B => reg_rm!("SUB", true, false),
            0x2C => alu_imm!("SUB", true),
            0x2D => alu_imm!("SUB", false),
            0x30 => reg_rm!("XOR", false, true),
            0x31 => reg_rm!("XOR", false, false),
            0x32 => reg_rm!("XOR", true, true),
            0x33 => reg_rm!("XOR", true, false),
            0x34 => alu_imm!("XOR", true),
            0x35 => alu_imm!("XOR", false),
            0x38 => reg_rm!("CMP", false, true),
            0x39 => reg_rm!("CMP", false, false),
            0x3A => reg_rm!("CMP", true, true),
            0x3B => reg_rm!("CMP", true, false),
            0x3C => alu_imm!("CMP", true),
            0x3D => alu_imm!("CMP", false),

            0x50..=0x57 => {
                ins.mnemonic = "PUSH";
                ins.opsize = 64;
                let reg = (op - 0x50) as u8 + if rex.b { 8 } else { 0 };
                ins.operands = vec![Operand::Register(reg)];
                return Ok(());
            }
            0x58..=0x5F => {
                ins.mnemonic = "POP";
                ins.opsize = 64;
                let reg = (op - 0x58) as u8 + if rex.b { 8 } else { 0 };
                ins.operands = vec![Operand::Register(reg)];
                return Ok(());
            }

            0x6C => {
                ins.mnemonic = "INSB";
                ins.opsize = 8;
                return Ok(());
            }
            0x6D => {
                ins.mnemonic = match ins.opsize {
                    16 => "INSW",
                    64 => "INSQ",
                    _ => "INSD",
                };
                return Ok(());
            }
            0x6E => {
                ins.mnemonic = "OUTSB";
                ins.opsize = 8;
                return Ok(());
            }
            0x6F => {
                ins.mnemonic = match ins.opsize {
                    16 => "OUTSW",
                    64 => "OUTSQ",
                    _ => "OUTSD",
                };
                return Ok(());
            }

            0x68 => {
                ins.mnemonic = "PUSH";
                ins.operands = vec![Operand::Immediate(Self::read_imm(mmu, pos, 4, true)?)];
                return Ok(());
            }
            0x6A => {
                ins.mnemonic = "PUSH";
                ins.operands = vec![Operand::Immediate(Self::read_imm(mmu, pos, 1, true)?)];
                return Ok(());
            }
            0x69 => {
                ins.mnemonic = "IMUL";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                let imm = Self::read_imm(mmu, pos, if opsize == 16 { 2 } else { 4 }, true)?;
                ins.operands = vec![Operand::Register(reg), rm, Operand::Immediate(imm)];
                return Ok(());
            }
            0x6B => {
                ins.mnemonic = "IMUL";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                ins.operands = vec![
                    Operand::Register(reg),
                    rm,
                    Operand::Immediate(Self::read_imm(mmu, pos, 1, true)?),
                ];
                return Ok(());
            }

            0x70..=0x7F => {
                ins.mnemonic = "JCC";
                ins.condition = (op - 0x70) as u8;
                let rel = Self::rd(mmu, pos)? as i8 as i32;
                ins.operands = vec![Operand::Relative(rel)];
                return Ok(());
            }

            0x80 | 0x81 | 0x82 | 0x83 => {
                let (digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, true)?;
                let imm_size: u8 = if op == 0x80 || op == 0x82 {
                    ins.opsize = 8;
                    1
                } else if op == 0x83 {
                    1
                } else if opsize == 16 {
                    2
                } else {
                    4
                };
                let imm = Self::read_imm(
                    mmu,
                    pos,
                    imm_size,
                    op == 0x83 || (op == 0x81 && opsize == 64),
                )?;
                ins.mnemonic = group1_mnemonic(digit);
                ins.operands = vec![rm, Operand::Immediate(imm)];
                return Ok(());
            }

            0x84 | 0x85 => {
                ins.mnemonic = "TEST";
                if op == 0x84 {
                    ins.opsize = 8;
                }
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, ins.opsize, addrsize, segment, false)?;
                ins.operands = vec![rm, Operand::Register(reg)];
                return Ok(());
            }

            0x86 | 0x87 => {
                ins.mnemonic = "XCHG";
                if op == 0x86 {
                    ins.opsize = 8;
                }
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, ins.opsize, addrsize, segment, false)?;
                ins.operands = vec![rm, Operand::Register(reg)];
                return Ok(());
            }

            0x88 => reg_rm!("MOV", false, true),
            0x89 => reg_rm!("MOV", false, false),
            0x8A => reg_rm!("MOV", true, true),
            0x8B => reg_rm!("MOV", true, false),
            0x8C => {
                ins.mnemonic = "MOV";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                ins.operands = vec![rm, Operand::Segment(seg_index(reg))];
                return Ok(());
            }
            0x8D => {
                ins.mnemonic = "LEA";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                ins.operands = vec![Operand::Register(reg), rm];
                return Ok(());
            }
            0x8E => {
                ins.mnemonic = "MOV";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                ins.operands = vec![Operand::Segment(seg_index(reg)), rm];
                return Ok(());
            }
            0x8F => {
                ins.mnemonic = "POP";
                let (_digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, true)?;
                ins.operands = vec![rm];
                return Ok(());
            }

            0x90 => {
                ins.mnemonic = "NOP";
                return Ok(());
            }
            0x91..=0x97 => {
                ins.mnemonic = "XCHG";
                let reg = (op - 0x91) as u8 + if rex.b { 8 } else { 0 };
                ins.operands = vec![Operand::Register(0), Operand::Register(reg)];
                return Ok(());
            }

            0x98 => {
                ins.mnemonic = if opsize == 64 {
                    "CDQE"
                } else if opsize == 16 {
                    "CBW"
                } else {
                    "CWDE"
                };
                return Ok(());
            }
            0x99 => {
                ins.mnemonic = if opsize == 64 {
                    "CQO"
                } else if opsize == 16 {
                    "CWD"
                } else {
                    "CDQ"
                };
                return Ok(());
            }
            0x9C => {
                ins.mnemonic = "PUSHF";
                return Ok(());
            }
            0x9D => {
                ins.mnemonic = "POPF";
                return Ok(());
            }
            0x9E => {
                ins.mnemonic = "SAHF";
                return Ok(());
            }
            0x9F => {
                ins.mnemonic = "LAHF";
                return Ok(());
            }

            0xA0 => {
                ins.mnemonic = "MOV";
                ins.opsize = 8;
                let offset = moffs_addr(mmu, pos, addrsize)?;
                ins.operands = vec![
                    Operand::Register(0),
                    Operand::Memory(moffs_mem(offset, segment)),
                ];
                return Ok(());
            }
            0xA1 => {
                ins.mnemonic = "MOV";
                let offset = moffs_addr(mmu, pos, addrsize)?;
                ins.operands = vec![
                    Operand::Register(0),
                    Operand::Memory(moffs_mem(offset, segment)),
                ];
                return Ok(());
            }
            0xA2 => {
                ins.mnemonic = "MOV";
                ins.opsize = 8;
                let offset = moffs_addr(mmu, pos, addrsize)?;
                ins.operands = vec![
                    Operand::Memory(moffs_mem(offset, segment)),
                    Operand::Register(0),
                ];
                return Ok(());
            }
            0xA3 => {
                ins.mnemonic = "MOV";
                let offset = moffs_addr(mmu, pos, addrsize)?;
                ins.operands = vec![
                    Operand::Memory(moffs_mem(offset, segment)),
                    Operand::Register(0),
                ];
                return Ok(());
            }
            0xA8 => {
                ins.mnemonic = "TEST";
                ins.opsize = 8;
                let immediate = Self::read_imm(mmu, pos, 1, false)?;
                ins.operands = vec![Operand::Register(0), Operand::Immediate(immediate)];
                return Ok(());
            }
            0xA9 => {
                ins.mnemonic = "TEST";
                let immediate_size = if opsize == 16 { 2 } else { 4 };
                let immediate = Self::read_imm(mmu, pos, immediate_size, opsize == 64)?;
                ins.operands = vec![Operand::Register(0), Operand::Immediate(immediate)];
                return Ok(());
            }

            0xA4 => {
                ins.mnemonic = "MOVSB";
                ins.opsize = 8;
                return Ok(());
            }
            0xA5 => {
                ins.mnemonic = match opsize {
                    16 => "MOVSW",
                    64 => "MOVSQ",
                    _ => "MOVSD",
                };
                return Ok(());
            }
            0xA6 => {
                ins.mnemonic = "CMPSB";
                ins.opsize = 8;
                return Ok(());
            }
            0xA7 => {
                ins.mnemonic = match opsize {
                    16 => "CMPSW",
                    64 => "CMPSQ",
                    _ => "CMPSD",
                };
                return Ok(());
            }
            0xAA => {
                ins.mnemonic = "STOSB";
                ins.opsize = 8;
                return Ok(());
            }
            0xAB => {
                ins.mnemonic = match opsize {
                    16 => "STOSW",
                    64 => "STOSQ",
                    _ => "STOSD",
                };
                return Ok(());
            }
            0xAC => {
                ins.mnemonic = "LODSB";
                ins.opsize = 8;
                return Ok(());
            }
            0xAD => {
                ins.mnemonic = match opsize {
                    16 => "LODSW",
                    64 => "LODSQ",
                    _ => "LODSD",
                };
                return Ok(());
            }
            0xAE => {
                ins.mnemonic = "SCASB";
                ins.opsize = 8;
                return Ok(());
            }
            0xAF => {
                ins.mnemonic = match opsize {
                    16 => "SCASW",
                    64 => "SCASQ",
                    _ => "SCASD",
                };
                return Ok(());
            }

            0xB0..=0xB7 => {
                ins.mnemonic = "MOV";
                ins.opsize = 8;
                let reg = (op - 0xB0) as u8 + if rex.b { 8 } else { 0 };
                ins.operands = vec![
                    Operand::Register(reg),
                    Operand::Immediate(Self::read_imm(mmu, pos, 1, false)?),
                ];
                return Ok(());
            }
            0xB8..=0xBF => {
                ins.mnemonic = "MOV";
                let reg = (op - 0xB8) as u8 + if rex.b { 8 } else { 0 };
                let imm = if opsize == 64 {
                    Self::read_imm(mmu, pos, 8, false)?
                } else if opsize == 16 {
                    Self::read_imm(mmu, pos, 2, false)?
                } else {
                    Self::read_imm(mmu, pos, 4, false)?
                };
                ins.operands = vec![Operand::Register(reg), Operand::Immediate(imm)];
                return Ok(());
            }

            0xC6 | 0xC7 => {
                ins.mnemonic = "MOV";
                let (_digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, true)?;
                let imm = if op == 0xC6 {
                    ins.opsize = 8;
                    Self::read_imm(mmu, pos, 1, false)?
                } else if opsize == 16 {
                    Self::read_imm(mmu, pos, 2, false)?
                } else {
                    Self::read_imm(mmu, pos, 4, opsize == 64)?
                };
                ins.operands = vec![rm, Operand::Immediate(imm)];
                return Ok(());
            }

            0xC0 | 0xC1 => {
                let (digit, rm) = self.decode_modrm_operands(
                    mmu,
                    pos,
                    rex,
                    if op == 0xC0 { 8 } else { opsize },
                    addrsize,
                    segment,
                    true,
                )?;
                if op == 0xC0 {
                    ins.opsize = 8;
                }
                ins.mnemonic = shift_mnemonic(digit);
                let imm = Self::read_imm(mmu, pos, 1, false)?;
                ins.operands = vec![rm, Operand::Immediate(imm)];
                return Ok(());
            }

            0xC2 => {
                ins.mnemonic = "RET";
                let imm = Self::read_imm(mmu, pos, 2, false)?;
                ins.operands = vec![Operand::Immediate(imm)];
                return Ok(());
            }
            0xC3 => {
                ins.mnemonic = "RET";
                return Ok(());
            }
            0xCA => {
                ins.mnemonic = "RETF";
                let imm = Self::read_imm(mmu, pos, 2, false)?;
                ins.operands = vec![Operand::Immediate(imm)];
                return Ok(());
            }
            0xCB => {
                ins.mnemonic = "RETF";
                return Ok(());
            }
            0xCC => {
                ins.mnemonic = "INT3";
                ins.operands = vec![Operand::Immediate(3)];
                return Ok(());
            }
            0xCD => {
                ins.mnemonic = "INT";
                ins.operands = vec![Operand::Immediate(Self::read_imm(mmu, pos, 1, false)?)];
                return Ok(());
            }
            0xCF => {
                ins.mnemonic = "IRET";
                return Ok(());
            }

            0xD0 => {
                let (digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, 8, addrsize, segment, true)?;
                ins.mnemonic = shift_mnemonic(digit);
                ins.opsize = 8;
                ins.operands = vec![rm, Operand::Immediate(1)];
                return Ok(());
            }
            0xD1 => {
                let (digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, true)?;
                ins.mnemonic = shift_mnemonic(digit);
                ins.operands = vec![rm, Operand::Immediate(1)];
                return Ok(());
            }
            0xD2 => {
                let (digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, 8, addrsize, segment, true)?;
                ins.mnemonic = shift_mnemonic(digit);
                ins.opsize = 8;
                ins.operands = vec![rm, Operand::Register(1)];
                return Ok(());
            }
            0xD3 => {
                let (digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, true)?;
                ins.mnemonic = shift_mnemonic(digit);
                ins.operands = vec![rm, Operand::Register(1)];
                return Ok(());
            }

            0xE0 => {
                ins.mnemonic = "LOOPNE";
                let rel = Self::rd(mmu, pos)? as i8 as i32;
                ins.operands = vec![Operand::Relative(rel)];
                return Ok(());
            }
            0xE1 => {
                ins.mnemonic = "LOOPE";
                let rel = Self::rd(mmu, pos)? as i8 as i32;
                ins.operands = vec![Operand::Relative(rel)];
                return Ok(());
            }
            0xE2 => {
                ins.mnemonic = "LOOP";
                let rel = Self::rd(mmu, pos)? as i8 as i32;
                ins.operands = vec![Operand::Relative(rel)];
                return Ok(());
            }
            0xE3 => {
                ins.mnemonic = "JRCXZ";
                let rel = Self::rd(mmu, pos)? as i8 as i32;
                ins.operands = vec![Operand::Relative(rel)];
                return Ok(());
            }

            0xE4 => {
                ins.mnemonic = "IN";
                ins.opsize = 8;
                ins.operands = vec![
                    Operand::Register(0),
                    Operand::Immediate(Self::read_imm(mmu, pos, 1, false)?),
                ];
                return Ok(());
            }
            0xE5 => {
                ins.mnemonic = "IN";
                ins.operands = vec![
                    Operand::Register(0),
                    Operand::Immediate(Self::read_imm(mmu, pos, 1, false)?),
                ];
                return Ok(());
            }
            0xE6 => {
                ins.mnemonic = "OUT";
                ins.opsize = 8;
                ins.operands = vec![
                    Operand::Immediate(Self::read_imm(mmu, pos, 1, false)?),
                    Operand::Register(0),
                ];
                return Ok(());
            }
            0xE7 => {
                ins.mnemonic = "OUT";
                ins.operands = vec![
                    Operand::Immediate(Self::read_imm(mmu, pos, 1, false)?),
                    Operand::Register(0),
                ];
                return Ok(());
            }

            0xEC => {
                ins.mnemonic = "IN";
                ins.opsize = 8;
                ins.operands = vec![Operand::Register(0), Operand::Register(2)];
                return Ok(());
            }
            0xED => {
                ins.mnemonic = "IN";
                ins.operands = vec![Operand::Register(0), Operand::Register(2)];
                return Ok(());
            }
            0xEE => {
                ins.mnemonic = "OUT";
                ins.opsize = 8;
                ins.operands = vec![Operand::Register(2), Operand::Register(0)];
                return Ok(());
            }
            0xEF => {
                ins.mnemonic = "OUT";
                ins.operands = vec![Operand::Register(2), Operand::Register(0)];
                return Ok(());
            }

            0xE8 => {
                ins.mnemonic = "CALL";
                let rel = Self::rd32(mmu, pos)? as i32;
                ins.operands = vec![Operand::Relative(rel)];
                return Ok(());
            }
            0xE9 => {
                ins.mnemonic = "JMP";
                let rel = Self::rd32(mmu, pos)? as i32;
                ins.operands = vec![Operand::Relative(rel)];
                return Ok(());
            }
            0xEA => {
                ins.mnemonic = "JMP";
                let off = if opsize == 32 {
                    Self::rd32(mmu, pos)? as u64
                } else {
                    Self::rd16(mmu, pos)? as u64
                };
                let sel = Self::rd16(mmu, pos)?;
                ins.operands = vec![Operand::Far { offset: off, selector: sel }];
                return Ok(());
            }
            0xEB => {
                ins.mnemonic = "JMP";
                let rel = Self::rd(mmu, pos)? as i8 as i32;
                ins.operands = vec![Operand::Relative(rel)];
                return Ok(());
            }

            0xF4 => {
                ins.mnemonic = "HLT";
                return Ok(());
            }
            0xF5 => {
                ins.mnemonic = "CMC";
                return Ok(());
            }

            0xF6 => {
                let (digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, 8, addrsize, segment, true)?;
                ins.opsize = 8;
                ins.mnemonic = group3_mnemonic(digit);
                if digit == 0 {
                    ins.operands = vec![
                        rm,
                        Operand::Immediate(Self::read_imm(mmu, pos, 1, false)?),
                    ];
                } else {
                    ins.operands = vec![rm];
                }
                return Ok(());
            }
            0xF7 => {
                let (digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, true)?;
                ins.mnemonic = group3_mnemonic(digit);
                if digit == 0 {
                    let imm = if opsize == 16 {
                        Self::read_imm(mmu, pos, 2, false)?
                    } else {
                        Self::read_imm(mmu, pos, 4, false)?
                    };
                    ins.operands = vec![rm, Operand::Immediate(imm)];
                } else {
                    ins.operands = vec![rm];
                }
                return Ok(());
            }

            0xF8 => {
                ins.mnemonic = "CLC";
                return Ok(());
            }
            0xF9 => {
                ins.mnemonic = "STC";
                return Ok(());
            }
            0xFA => {
                ins.mnemonic = "CLI";
                return Ok(());
            }
            0xFB => {
                ins.mnemonic = "STI";
                return Ok(());
            }
            0xFC => {
                ins.mnemonic = "CLD";
                return Ok(());
            }
            0xFD => {
                ins.mnemonic = "STD";
                return Ok(());
            }

            0xFE => {
                let (digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, 8, addrsize, segment, true)?;
                ins.opsize = 8;
                ins.mnemonic = if digit == 0 { "INC" } else { "DEC" };
                ins.operands = vec![rm];
                return Ok(());
            }
            0xFF => {
                let (digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, true)?;
                ins.mnemonic = group5_mnemonic(digit);
                ins.operands = vec![rm];
                return Ok(());
            }

            _ => {
                return Err(InstructionDecodeError::InvalidOpcode);
            }
        }
    }

    // ------------------------------------------------------------------
    // Two-byte opcode map (0x0F ...)
    // ------------------------------------------------------------------

    fn decode_two_byte(
        &self,
        mmu: &Mmu,
        pos: &mut u64,
        ins: &mut DecodedInstruction,
        rex: Rex,
        segment: u8,
    ) -> Result<(), InstructionDecodeError> {
        // The 0x0F was consumed by decode_inner which stores it in ins.op2.
        let op2 = ins.op2.unwrap_or(0) as usize;
        let opsize = ins.opsize;
        let addrsize = ins.addrsize;

        match op2 {
            0x1F => {
                // Multi-byte NOP: consume the ModR/M and optional address
                // bytes, but do not touch the referenced memory.
                let _ = self.decode_modrm_operands(
                    mmu, pos, rex, opsize, addrsize, segment, true,
                )?;
                ins.mnemonic = "NOP";
                ins.operands = vec![];
                return Ok(());
            }
            0x80..=0x8F => {
                ins.mnemonic = "JCC";
                ins.condition = (op2 - 0x80) as u8;
                ins.opsize = if opsize == 16 { 16 } else { 32 };
                let rel = Self::rd32(mmu, pos)? as i32;
                ins.operands = vec![Operand::Relative(rel)];
                return Ok(());
            }

            0x05 => {
                ins.mnemonic = "SYSCALL";
                return Ok(());
            }
            0x07 => {
                ins.mnemonic = "SYSRET";
                return Ok(());
            }
            0x30 => {
                ins.mnemonic = "WRMSR";
                return Ok(());
            }
            0x32 => {
                ins.mnemonic = "RDMSR";
                return Ok(());
            }
            0x34 => {
                ins.mnemonic = "SYSENTER";
                return Ok(());
            }
            0x35 => {
                ins.mnemonic = "SYSEXIT";
                return Ok(());
            }
            0xA2 => {
                ins.mnemonic = "CPUID";
                return Ok(());
            }

            0x20 => {
                let (mod_, reg, rm) = Self::read_modrm(mmu, pos)?;
                if mod_ != 0b11 {
                    return Err(InstructionDecodeError::InvalidModRm);
                }
                ins.mnemonic = "MOV";
                let high = if rex.b { 8 } else { 0 };
                ins.operands = vec![Operand::Register(rm + high), Operand::ControlRegister(reg)];
                return Ok(());
            }
            0x22 => {
                let (mod_, reg, rm) = Self::read_modrm(mmu, pos)?;
                if mod_ != 0b11 {
                    return Err(InstructionDecodeError::InvalidModRm);
                }
                ins.mnemonic = "MOV";
                let high = if rex.b { 8 } else { 0 };
                ins.operands = vec![Operand::ControlRegister(reg), Operand::Register(rm + high)];
                return Ok(());
            }

            0x01 => {
                let (digit, rm) = self.decode_modrm_operands(
                    mmu, pos, Rex::default(), opsize, addrsize, 0, true,
                )?;
                match digit {
                    0 => ins.mnemonic = "SGDT",
                    1 => ins.mnemonic = "SIDT",
                    2 => ins.mnemonic = "LGDT",
                    3 => ins.mnemonic = "LIDT",
                    4 => ins.mnemonic = "SMSW",
                    6 => ins.mnemonic = "LMSW",
                    7 => ins.mnemonic = "INVLPG",
                    _ => return Err(InstructionDecodeError::InvalidOpcode),
                }
                ins.operands = vec![rm];
                return Ok(());
            }

            0xAF => {
                ins.mnemonic = "IMUL";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                ins.operands = vec![Operand::Register(reg), rm];
                return Ok(());
            }

            0xB6 => {
                ins.mnemonic = "MOVZX";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                ins.operands = vec![Operand::Register(reg), rm];
                return Ok(());
            }
            0xB7 => {
                ins.mnemonic = "MOVZX";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, 16, addrsize, segment, false)?;
                ins.operands = vec![Operand::Register(reg), rm];
                return Ok(());
            }
            0xBE => {
                ins.mnemonic = "MOVSX";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                ins.operands = vec![Operand::Register(reg), rm];
                return Ok(());
            }
            0xBF => {
                ins.mnemonic = "MOVSX";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, 16, addrsize, segment, false)?;
                ins.operands = vec![Operand::Register(reg), rm];
                return Ok(());
            }

            0x63 => {
                ins.mnemonic = "MOVSXD";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, 32, addrsize, segment, false)?;
                ins.operands = vec![Operand::Register(reg), rm];
                return Ok(());
            }

            0x90..=0x9F => {
                ins.mnemonic = "SETCC";
                ins.condition = (op2 - 0x90) as u8;
                let (_digit, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, 8, addrsize, segment, true)?;
                ins.operands = vec![rm];
                return Ok(());
            }

            0xBC => {
                ins.mnemonic = "BSF";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                ins.operands = vec![Operand::Register(reg), rm];
                return Ok(());
            }
            0xBD => {
                ins.mnemonic = "BSR";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                ins.operands = vec![Operand::Register(reg), rm];
                return Ok(());
            }

            0xB0 => {
                ins.mnemonic = "CMPXCHG";
                ins.opsize = 8;
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, 8, addrsize, segment, false)?;
                ins.operands = vec![rm, Operand::Register(reg)];
                return Ok(());
            }
            0xB1 => {
                ins.mnemonic = "CMPXCHG";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                ins.operands = vec![rm, Operand::Register(reg)];
                return Ok(());
            }

            0xA3 => {
                ins.mnemonic = "BT";
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, opsize, addrsize, segment, false)?;
                ins.operands = vec![rm, Operand::Register(reg)];
                return Ok(());
            }

            0xC0 | 0xC1 => {
                ins.mnemonic = "XADD";
                let width = if op2 == 0xC0 { 8 } else { opsize };
                if op2 == 0xC0 {
                    ins.opsize = 8;
                }
                let (reg, rm) =
                    self.decode_modrm_operands(mmu, pos, rex, width, addrsize, segment, false)?;
                ins.operands = vec![rm, Operand::Register(reg)];
                return Ok(());
            }

            0xAE => {
                let (digit, rm) =
                    self.decode_modrm_operands(mmu, pos, Rex::default(), opsize, addrsize, 0, true)?;
                match digit {
                    0 => ins.mnemonic = "FXRSTOR",
                    1 => ins.mnemonic = "FXSAVE",
                    5 => ins.mnemonic = "LFENCE",
                    6 => ins.mnemonic = "MFENCE",
                    7 => ins.mnemonic = "SFENCE",
                    _ => return Err(InstructionDecodeError::InvalidOpcode),
                }
                if digit >= 5 {
                    ins.operands = vec![];
                } else {
                    ins.operands = vec![rm];
                }
                return Ok(());
            }

            0x40..=0x4F => {
                ins.mnemonic = "CMOVCC";
                ins.condition = (op2 - 0x40) as u8;
                let (reg, rm) = self.decode_modrm_operands(
                    mmu, pos, rex, opsize, addrsize, segment, false,
                )?;
                ins.operands = vec![Operand::Register(reg), rm];
                return Ok(());
            }

            0x38 | 0x3A => {
                return Err(InstructionDecodeError::InvalidOpcode);
            }

            _ => {
                return Err(InstructionDecodeError::InvalidOpcode);
            }
        }
    }
}

fn moffs_addr(mmu: &Mmu, pos: &mut u64, addrsize: u8) -> Result<u64, InstructionDecodeError> {
    if addrsize == 64 {
        InstructionDecoder::rd64(mmu, pos)
    } else {
        Ok(InstructionDecoder::rd32(mmu, pos)? as u64)
    }
}

fn moffs_mem(offset: u64, segment: u8) -> MemoryOperand {
    MemoryOperand {
        base: None,
        index: None,
        scale: 1,
        displacement: offset as i32,
        rip_relative: false,
        segment,
    }
}

fn seg_index(n: u8) -> u8 {
    match n {
        0 => 3, // ES
        1 => 1, // CS
        2 => 6, // SS
        3 => 2, // DS
        4 => 4, // FS
        5 => 5, // GS
        _ => 2,
    }
}

fn group1_mnemonic(digit: u8) -> &'static str {
    match digit {
        0 => "ADD",
        1 => "OR",
        2 => "ADC",
        3 => "SBB",
        4 => "AND",
        5 => "SUB",
        6 => "XOR",
        7 => "CMP",
        _ => "UNKNOWN",
    }
}

fn shift_mnemonic(digit: u8) -> &'static str {
    match digit {
        0 => "ROL",
        1 => "ROR",
        2 => "RCL",
        3 => "RCR",
        4 => "SHL",
        5 => "SHR",
        6 => "SHL",
        7 => "SAR",
        _ => "UNKNOWN",
    }
}

fn group3_mnemonic(digit: u8) -> &'static str {
    match digit {
        0 => "TEST",
        1 => "TEST",
        2 => "NOT",
        3 => "NEG",
        4 => "MUL",
        5 => "IMUL",
        6 => "DIV",
        7 => "IDIV",
        _ => "UNKNOWN",
    }
}

fn group5_mnemonic(digit: u8) -> &'static str {
    match digit {
        0 => "INC",
        1 => "DEC",
        2 => "CALL",
        3 => "CALLF",
        4 => "JMP",
        5 => "JMPF",
        6 => "PUSH",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(bytes: &[u8]) -> Result<DecodedInstruction, InstructionDecodeError> {
        let mut mmu = crate::memory::Mmu::new(1 << 20);
        mmu.write_phys(0x1000, bytes).unwrap();
        InstructionDecoder::new().decode(0x1000, &mmu)
    }

    #[test]
    fn mov_imm_reg() {
        let i = dec(&[0xB8, 0x34, 0x12, 0x00, 0x00]).unwrap();
        assert_eq!(i.mnemonic, "MOV");
        assert_eq!(i.opsize, 32);
        assert_eq!(i.next_ip, 0x1005);
        match &i.operands[..] {
            [Operand::Register(0), Operand::Immediate(v)] => assert_eq!(*v, 0x1234),
            _ => panic!("bad operands"),
        }
    }

    #[test]
    fn mov_imm_rax_rex_w() {
        let i = dec(&[0x48, 0xB8, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]).unwrap();
        assert_eq!(i.opsize, 64);
        match &i.operands[..] {
            [Operand::Register(0), Operand::Immediate(v)] => {
                assert_eq!(*v, 0x1122_3344_5566_7788)
            }
            _ => panic!("bad operands"),
        }
    }

    #[test]
    fn mov_reg_mem_sib() {
        let i = dec(&[0x48, 0x8B, 0x44, 0xB8, 0x10]).unwrap();
        assert_eq!(i.opsize, 64);
        match &i.operands[..] {
            [Operand::Register(0), Operand::Memory(m)] => {
                assert_eq!(m.base, Some(0));
                assert_eq!(m.index, Some(7));
                assert_eq!(m.scale, 4);
                assert_eq!(m.displacement, 0x10);
            }
            _ => panic!("bad operands"),
        }
    }

    #[test]
    fn rip_relative() {
        let i = dec(&[0x8B, 0x05, 0x00, 0x10, 0x00, 0x00]).unwrap();
        match &i.operands[..] {
            [Operand::Register(0), Operand::Memory(m)] => {
                assert!(m.rip_relative);
                assert_eq!(m.displacement, 0x1000);
            }
            _ => panic!("bad operands"),
        }
    }

    #[test]
    fn push_pop() {
        assert_eq!(dec(&[0x50]).unwrap().mnemonic, "PUSH");
        let pop = dec(&[0x58]).unwrap();
        assert_eq!(pop.mnemonic, "POP");
    }

    #[test]
    fn jcc_rel() {
        let i = dec(&[0x75, 0x02]).unwrap();
        assert_eq!(i.condition, 0x5);
        match &i.operands[..] {
            [Operand::Relative(r)] => assert_eq!(*r, 2),
            _ => panic!("bad operands"),
        }
    }

    #[test]
    fn call_rel32() {
        let i = dec(&[0xE8, 0x01, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!(i.mnemonic, "CALL");
        assert_eq!(i.next_ip, 0x1005);
    }

    #[test]
    fn group1_imm() {
        let i = dec(&[0x83, 0xC0, 0x05]).unwrap();
        assert_eq!(i.mnemonic, "ADD");
        match &i.operands[..] {
            [Operand::Register(0), Operand::Immediate(v)] => assert_eq!(*v, 5),
            _ => panic!("bad operands"),
        }
    }

    #[test]
    fn lea_and_movzx() {
        assert_eq!(dec(&[0x8D, 0x04, 0x07]).unwrap().mnemonic, "LEA");
        assert_eq!(dec(&[0x0F, 0xB6, 0xC1]).unwrap().mnemonic, "MOVZX");
    }

    #[test]
    fn syscall() {
        let i = dec(&[0x0F, 0x05]).unwrap();
        assert_eq!(i.mnemonic, "SYSCALL");
        assert_eq!(i.next_ip, 0x1002);
    }

    #[test]
    fn invalid_opcode() {
        assert!(matches!(
            dec(&[0x0F, 0x38]),
            Err(InstructionDecodeError::InvalidOpcode)
        ));
    }

    #[test]
    fn int_vector() {
        let i = dec(&[0xCD, 0x80]).unwrap();
        match &i.operands[..] {
            [Operand::Immediate(v)] => assert_eq!(*v, 0x80),
            _ => panic!("bad operands"),
        }
    }

    #[test]
    fn rep_prefix_tracked() {
        let i = dec(&[0xF3, 0xA4]).unwrap();
        assert_eq!(i.mnemonic, "MOVSB");
        assert_eq!(i.rep_prefix, Some(true));
    }

    #[test]
    fn in_out_dx() {
        assert_eq!(dec(&[0xEC]).unwrap().mnemonic, "IN");
        assert_eq!(dec(&[0xEE]).unwrap().mnemonic, "OUT");
    }

    #[test]
    fn segment_move() {
        let i = dec(&[0x8C, 0xD8]).unwrap();
        match &i.operands[..] {
            [Operand::Register(0), Operand::Segment(s)] => assert_eq!(*s, 2),
            _ => panic!("bad operands"),
        }
    }
}
