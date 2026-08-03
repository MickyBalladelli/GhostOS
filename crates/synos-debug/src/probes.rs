//! Safe, bounded bytecode probes for live Ring 3 telemetry.
//!
//! Programs can inspect scalar samples only. They cannot dereference memory,
//! call into the kernel, loop backwards, or allocate. This keeps probe cost
//! and authority reviewable before a program is attached in production.

use synos_fabric::NodeId;

use crate::Error;

pub const MAX_PROBE_INSTRUCTIONS: usize = 64;
pub const PROBE_REGISTERS: usize = 8;
pub const MAX_PROBE_STEPS: usize = MAX_PROBE_INSTRUCTIONS;
pub const DEFAULT_PROBE_EVENTS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SampleKind {
    IpcStream = 1,
    RingBuffer = 2,
    CxlMemory = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeSample {
    pub timestamp_us: u64,
    pub node: NodeId,
    pub kind: SampleKind,
    pub subject: u64,
    pub value0: u64,
    pub value1: u64,
    pub value2: u64,
    pub value3: u64,
}

impl ProbeSample {
    pub const fn ipc_stream(
        timestamp_us: u64,
        node: NodeId,
        channel: u64,
        bytes: u64,
        messages: u64,
        dropped: u64,
    ) -> Self {
        Self {
            timestamp_us,
            node,
            kind: SampleKind::IpcStream,
            subject: channel,
            value0: bytes,
            value1: messages,
            value2: dropped,
            value3: 0,
        }
    }

    pub const fn ring_buffer(
        timestamp_us: u64,
        node: NodeId,
        ring: u64,
        capacity: u64,
        used: u64,
        dropped: u64,
    ) -> Self {
        Self {
            timestamp_us,
            node,
            kind: SampleKind::RingBuffer,
            subject: ring,
            value0: capacity,
            value1: used,
            value2: dropped,
            value3: 0,
        }
    }

    pub const fn cxl_latency(
        timestamp_us: u64,
        node: NodeId,
        device: u64,
        latency_ns: u64,
        bytes: u64,
    ) -> Self {
        Self {
            timestamp_us,
            node,
            kind: SampleKind::CxlMemory,
            subject: device,
            value0: latency_ns,
            value1: bytes,
            value2: 0,
            value3: 0,
        }
    }

    fn field(self, field: SampleField) -> u64 {
        match field {
            SampleField::Timestamp => self.timestamp_us,
            SampleField::Node => self.node.raw() as u64,
            SampleField::Subject => self.subject,
            SampleField::Value0 => self.value0,
            SampleField::Value1 => self.value1,
            SampleField::Value2 => self.value2,
            SampleField::Value3 => self.value3,
            SampleField::Kind => self.kind as u64,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SampleField {
    Timestamp = 1,
    Node = 2,
    Subject = 3,
    Value0 = 4,
    Value1 = 5,
    Value2 = 6,
    Value3 = 7,
    Kind = 8,
}

impl SampleField {
    fn from_raw(raw: i64) -> Option<Self> {
        match raw {
            1 => Some(Self::Timestamp),
            2 => Some(Self::Node),
            3 => Some(Self::Subject),
            4 => Some(Self::Value0),
            5 => Some(Self::Value1),
            6 => Some(Self::Value2),
            7 => Some(Self::Value3),
            8 => Some(Self::Kind),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Opcode {
    LoadField = 1,
    LoadImmediate = 2,
    Add = 3,
    Subtract = 4,
    And = 5,
    Or = 6,
    Xor = 7,
    Equal = 8,
    GreaterThan = 9,
    JumpIfZero = 10,
    Emit = 11,
    Halt = 12,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub opcode: Opcode,
    pub destination: u8,
    pub source: u8,
    pub immediate: i64,
}

impl Instruction {
    pub const fn load_field(destination: u8, field: SampleField) -> Self {
        Self {
            opcode: Opcode::LoadField,
            destination,
            source: 0,
            immediate: field as i64,
        }
    }

    pub const fn load_immediate(destination: u8, value: u64) -> Self {
        Self {
            opcode: Opcode::LoadImmediate,
            destination,
            source: 0,
            immediate: value as i64,
        }
    }

    pub const fn binary(opcode: Opcode, destination: u8, source: u8) -> Self {
        Self {
            opcode,
            destination,
            source,
            immediate: 0,
        }
    }

    pub const fn jump_if_zero(register: u8, target: u8) -> Self {
        Self {
            opcode: Opcode::JumpIfZero,
            destination: register,
            source: 0,
            immediate: target as i64,
        }
    }

    pub const EMIT: Self = Self {
        opcode: Opcode::Emit,
        destination: 0,
        source: 0,
        immediate: 0,
    };

    pub const HALT: Self = Self {
        opcode: Opcode::Halt,
        destination: 0,
        source: 0,
        immediate: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeProgram {
    instructions: [Instruction; MAX_PROBE_INSTRUCTIONS],
    len: u8,
}

impl ProbeProgram {
    pub const fn new() -> Self {
        Self {
            instructions: [Instruction::HALT; MAX_PROBE_INSTRUCTIONS],
            len: 0,
        }
    }

    pub fn push(&mut self, instruction: Instruction) -> Result<(), Error> {
        if self.len as usize == MAX_PROBE_INSTRUCTIONS {
            return Err(Error::Capacity);
        }
        self.instructions[self.len as usize] = instruction;
        self.len += 1;
        Ok(())
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions[..self.len()]
    }

    pub fn verify(&self) -> Result<(), Error> {
        if self.len == 0 || self.instructions()[self.len() - 1].opcode != Opcode::Halt {
            return Err(Error::InvalidProgram);
        }
        for (index, instruction) in self.instructions().iter().enumerate() {
            if instruction.destination as usize >= PROBE_REGISTERS {
                return Err(Error::InvalidProgram);
            }
            if matches!(
                instruction.opcode,
                Opcode::Add
                    | Opcode::Subtract
                    | Opcode::And
                    | Opcode::Or
                    | Opcode::Xor
                    | Opcode::Equal
                    | Opcode::GreaterThan
            ) && instruction.source as usize >= PROBE_REGISTERS
            {
                return Err(Error::InvalidProgram);
            }
            if instruction.opcode == Opcode::LoadField
                && SampleField::from_raw(instruction.immediate).is_none()
            {
                return Err(Error::InvalidProgram);
            }
            if instruction.opcode == Opcode::JumpIfZero {
                let target =
                    usize::try_from(instruction.immediate).map_err(|_| Error::InvalidProgram)?;
                if target <= index || target >= self.len() {
                    return Err(Error::InvalidProgram);
                }
            }
        }
        Ok(())
    }
}

impl Default for ProbeProgram {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeRecord {
    pub sample: ProbeSample,
    pub value: u64,
}

pub trait ProbeSink {
    fn record(&mut self, record: ProbeRecord) -> Result<(), Error>;
}

pub struct ProbeRecorder<const CAPACITY: usize = DEFAULT_PROBE_EVENTS> {
    records: [Option<ProbeRecord>; CAPACITY],
    next: usize,
    dropped: u64,
}

impl<const CAPACITY: usize> ProbeRecorder<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            records: [None; CAPACITY],
            next: 0,
            dropped: 0,
        }
    }

    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn records(&self) -> impl Iterator<Item = ProbeRecord> + '_ {
        self.records.iter().flatten().copied()
    }
}

impl<const CAPACITY: usize> Default for ProbeRecorder<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAPACITY: usize> ProbeSink for ProbeRecorder<CAPACITY> {
    fn record(&mut self, record: ProbeRecord) -> Result<(), Error> {
        if CAPACITY == 0 {
            return Err(Error::Capacity);
        }
        if self.records[self.next].is_some() {
            self.dropped = self.dropped.saturating_add(1);
        }
        self.records[self.next] = Some(record);
        self.next = (self.next + 1) % CAPACITY;
        Ok(())
    }
}

pub struct ProbeVm<S> {
    program: ProbeProgram,
    sink: S,
}

impl<S> ProbeVm<S>
where
    S: ProbeSink,
{
    pub fn new(program: ProbeProgram, sink: S) -> Result<Self, Error> {
        program.verify()?;
        Ok(Self { program, sink })
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    pub fn run(&mut self, sample: ProbeSample) -> Result<(), Error> {
        let mut registers = [0_u64; PROBE_REGISTERS];
        let mut program_counter = 0_usize;
        let mut steps = 0_usize;
        while program_counter < self.program.len() && steps < MAX_PROBE_STEPS {
            steps += 1;
            let instruction = self.program.instructions()[program_counter];
            match instruction.opcode {
                Opcode::LoadField => {
                    let field = SampleField::from_raw(instruction.immediate)
                        .ok_or(Error::InvalidProgram)?;
                    registers[instruction.destination as usize] = sample.field(field);
                }
                Opcode::LoadImmediate => {
                    registers[instruction.destination as usize] = instruction.immediate as u64;
                }
                Opcode::Add => {
                    let destination = instruction.destination as usize;
                    registers[destination] =
                        registers[destination].wrapping_add(registers[instruction.source as usize]);
                }
                Opcode::Subtract => {
                    let destination = instruction.destination as usize;
                    registers[destination] =
                        registers[destination].wrapping_sub(registers[instruction.source as usize]);
                }
                Opcode::And => {
                    let destination = instruction.destination as usize;
                    registers[destination] &= registers[instruction.source as usize];
                }
                Opcode::Or => {
                    let destination = instruction.destination as usize;
                    registers[destination] |= registers[instruction.source as usize];
                }
                Opcode::Xor => {
                    let destination = instruction.destination as usize;
                    registers[destination] ^= registers[instruction.source as usize];
                }
                Opcode::Equal => {
                    let destination = instruction.destination as usize;
                    registers[destination] =
                        (registers[destination] == registers[instruction.source as usize]) as u64;
                }
                Opcode::GreaterThan => {
                    let destination = instruction.destination as usize;
                    registers[destination] =
                        (registers[destination] > registers[instruction.source as usize]) as u64;
                }
                Opcode::JumpIfZero => {
                    if registers[instruction.destination as usize] == 0 {
                        program_counter = instruction.immediate as usize;
                        continue;
                    }
                }
                Opcode::Emit => self.sink.record(ProbeRecord {
                    sample,
                    value: registers[0],
                })?,
                Opcode::Halt => return Ok(()),
            }
            program_counter += 1;
        }
        Err(Error::InvalidProgram)
    }
}
