use crate::memory::Mmu;

pub struct InstructionDecoder;

impl InstructionDecoder {
    pub fn new() -> Self {
        Self
    }

    pub fn decode(&self, ip: u64, mmu: &Mmu) -> Result<DecodedInstruction, InstructionDecodeError> {
        let opcode_byte = mmu.read_byte(ip).map_err(|_| InstructionDecodeError::InvalidOpcode)?;
        
        let mut instruction = DecodedInstruction::new(ip, opcode_byte);
        
        let opcode = opcode_byte as usize;
        
        match opcode {
            0x00..=0x3F => {
                instruction = self.decode_r_m(opcode, &mut instruction, ip, mmu)?;
            }
            0x40..=0x4F => {
                instruction = self.decode_rex(opcode, &mut instruction, ip, mmu)?;
            }
            0x50..=0x5F => {
                instruction = self.decode_push_pop(opcode, &mut instruction, ip, mmu)?;
            }
            0x60..=0x6F => {
                instruction = self.decode_pending(opcode, &mut instruction, ip, mmu)?;
            }
            0x70..=0x7F => {
                instruction = self.decode_conditional_jump(opcode, &mut instruction, ip, mmu)?;
            }
            0x80..=0x8F => {
                instruction = self.decode_extended(opcode, &mut instruction, ip, mmu)?;
            }
            0x90..=0x9F => {
                instruction = self.decode_xchg(opcode, &mut instruction, ip, mmu)?;
            }
            0xA0..=0xFF => {
                instruction = self.decode_load_store(opcode, &mut instruction, ip, mmu)?;
            }
            _ => {
                return Err(InstructionDecodeError::InvalidOpcode);
            }
        }

        Ok(instruction)
    }

    fn decode_r_m(&self, opcode: usize, instruction: &mut DecodedInstruction, ip: u64, _mmu: &Mmu) -> Result<DecodedInstruction, InstructionDecodeError> {
        let mnemonic = match opcode {
            0x00 => "ADD", 0x01 => "ADD", 0x02 => "ADD", 0x03 => "ADD",
            0x08 => "OR",  0x09 => "OR",  0x0A => "OR",  0x0B => "OR",
            0x10 => "ADC", 0x11 => "ADC", 0x12 => "ADC", 0x13 => "ADC",
            0x18 => "SBB", 0x19 => "SBB", 0x1A => "SBB", 0x1B => "SBB",
            0x20 => "AND", 0x21 => "AND", 0x22 => "AND", 0x23 => "AND",
            0x28 => "SUB", 0x29 => "SUB", 0x2A => "SUB", 0x2B => "SUB",
            0x30 => "XOR", 0x31 => "XOR", 0x32 => "XOR", 0x33 => "XOR",
            _ => return Err(InstructionDecodeError::InvalidOpcode),
        };
        
        instruction.mnemonic = mnemonic;
        instruction.next_ip = ip + 1;
        Ok(instruction.clone())
    }

    fn decode_rex(&self, _opcode: usize, instruction: &mut DecodedInstruction, ip: u64, _mmu: &Mmu) -> Result<DecodedInstruction, InstructionDecodeError> {
        instruction.next_ip = ip + 1;
        Ok(instruction.clone())
    }

    fn decode_push_pop(&self, opcode: usize, instruction: &mut DecodedInstruction, ip: u64, _mmu: &Mmu) -> Result<DecodedInstruction, InstructionDecodeError> {
        let mnemonic = if opcode < 0x50 { "PUSH" } else { "POP" };
        instruction.mnemonic = mnemonic;
        instruction.next_ip = ip + 1;
        Ok(instruction.clone())
    }

    fn decode_pending(&self, _opcode: usize, instruction: &mut DecodedInstruction, ip: u64, _mmu: &Mmu) -> Result<DecodedInstruction, InstructionDecodeError> {
        instruction.next_ip = ip + 1;
        Ok(instruction.clone())
    }

    fn decode_conditional_jump(&self, opcode: usize, instruction: &mut DecodedInstruction, ip: u64, _mmu: &Mmu) -> Result<DecodedInstruction, InstructionDecodeError> {
        let jump_type = if opcode < 0x80 { "JCC" } else { "JMP" };
        instruction.mnemonic = jump_type;
        instruction.next_ip = ip + 1;
        Ok(instruction.clone())
    }

    fn decode_extended(&self, _opcode: usize, instruction: &mut DecodedInstruction, ip: u64, _mmu: &Mmu) -> Result<DecodedInstruction, InstructionDecodeError> {
        instruction.next_ip = ip + 1;
        Ok(instruction.clone())
    }

    fn decode_xchg(&self, opcode: usize, instruction: &mut DecodedInstruction, ip: u64, _mmu: &Mmu) -> Result<DecodedInstruction, InstructionDecodeError> {
        let mnemonic = if opcode == 0x90 { "NOP" } else { "XCHG" };
        instruction.mnemonic = mnemonic;
        instruction.next_ip = ip + 1;
        Ok(instruction.clone())
    }

    fn decode_load_store(&self, opcode: usize, instruction: &mut DecodedInstruction, ip: u64, _mmu: &Mmu) -> Result<DecodedInstruction, InstructionDecodeError> {
        let mnemonic = match opcode {
            0xA0..=0xA3 => "MOV",
            0xA4..=0xA7 => "MOVS",
            0xA8..=0xAB => "CMPS",
            0xAC..=0xAF => "LODS",
            0xB0..=0xB7 => "MOV",
            0xB8..=0xBF => "MOV",
            _ => "UNKNOWN",
        };
        instruction.mnemonic = mnemonic;
        instruction.next_ip = ip + 1;
        Ok(instruction.clone())
    }
}

#[derive(Clone, Debug)]
pub struct DecodedInstruction {
    pub ip: u64,
    pub opcode: u8,
    pub mnemonic: &'static str,
    pub next_ip: u64,
    pub operands: Vec<Operand>,
}

impl DecodedInstruction {
    pub fn new(ip: u64, opcode: u8) -> Self {
        Self {
            ip,
            opcode,
            mnemonic: "UNKNOWN",
            next_ip: ip + 1,
            operands: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Operand {
    Register(u8),
    Memory(u64),
    Immediate(u64),
    Relative(i32),
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