//! Small, transport-neutral subset of the GDB remote serial protocol.
//!
//! The stub only speaks framed packets. Serial, Ethernet, and a capability
//! protected IPC channel can all feed the same `ingest` method.

use ghostos_fabric::{NodeId, PageFault};
use ghostos_init::ProcessId;

use crate::Error;

pub const MAX_PACKET_BYTES: usize = 512;
pub const MAX_REGISTERS: usize = 32;
pub const MAX_DSM_FAULTS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DebugToken(u64);

impl DebugToken {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugOperation {
    Read,
    Write,
    Control,
}

pub trait DebugAuthority {
    fn permits(&self, token: DebugToken, operation: DebugOperation) -> bool;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DenyAll;

impl DebugAuthority for DenyAll {
    fn permits(&self, _token: DebugToken, _operation: DebugOperation) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegisterFile {
    values: [u64; MAX_REGISTERS],
    count: u8,
}

impl RegisterFile {
    pub const fn empty() -> Self {
        Self {
            values: [0; MAX_REGISTERS],
            count: 0,
        }
    }

    pub fn from_slice(values: &[u64]) -> Result<Self, Error> {
        if values.is_empty() || values.len() > MAX_REGISTERS {
            return Err(Error::InvalidInput);
        }
        let mut registers = Self::empty();
        registers.values[..values.len()].copy_from_slice(values);
        registers.count = values.len() as u8;
        Ok(registers)
    }

    pub const fn count(self) -> usize {
        self.count as usize
    }

    pub fn get(&self, index: usize) -> Option<u64> {
        (index < self.count()).then_some(self.values[index])
    }

    pub fn values(&self) -> &[u64] {
        &self.values[..self.count()]
    }

    fn set_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() != self.count() * 16 {
            return Err(Error::InvalidInput);
        }
        let count = self.count();
        for (index, value) in self.values[..count].iter_mut().enumerate() {
            let start = index * 16;
            let mut encoded = [0; 8];
            for (offset, byte) in encoded.iter_mut().enumerate() {
                *byte = decode_hex_pair(bytes[start + offset * 2], bytes[start + offset * 2 + 1])?;
            }
            *value = u64::from_le_bytes(encoded);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReason {
    Signal(u8),
    Exited(u8),
}

impl StopReason {
    fn encode(self, output: &mut [u8]) -> Result<usize, Error> {
        if output.len() < 3 {
            return Err(Error::BufferTooSmall { required: 3 });
        }
        match self {
            Self::Signal(signal) => {
                output[0] = b'S';
                output[1] = hex(signal >> 4);
                output[2] = hex(signal);
            }
            Self::Exited(code) => {
                output[0] = b'W';
                output[1] = hex(code >> 4);
                output[2] = hex(code);
            }
        }
        Ok(3)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DsmFault {
    pub process: ProcessId,
    pub node: NodeId,
    pub fault: PageFault,
    pub timestamp_us: u64,
    pub latency_ns: u32,
}

pub trait DebugRuntime {
    fn stop_reason(&mut self) -> StopReason;
    fn registers(&mut self) -> Result<RegisterFile, Error>;
    fn write_registers(&mut self, registers: RegisterFile) -> Result<(), Error>;
    fn read_memory(&mut self, address: u64, destination: &mut [u8]) -> Result<usize, Error>;
    fn write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), Error>;
    fn continue_execution(&mut self) -> Result<(), Error>;
    fn step_execution(&mut self) -> Result<(), Error>;
    fn reverse_continue_execution(&mut self) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    fn reverse_step_execution(&mut self) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    fn dsm_faults(&mut self, destination: &mut [DsmFault]) -> usize;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputState {
    Idle,
    Payload,
    ChecksumHigh,
    ChecksumLow,
}

pub struct GdbStub<R, A = DenyAll> {
    runtime: R,
    authority: A,
    token: DebugToken,
    payload: [u8; MAX_PACKET_BYTES],
    payload_len: usize,
    checksum: u8,
    received_checksum: u8,
    input_state: InputState,
    detached: bool,
}

impl<R, A> GdbStub<R, A>
where
    R: DebugRuntime,
    A: DebugAuthority,
{
    pub fn new(runtime: R, authority: A, token: DebugToken) -> Self {
        Self {
            runtime,
            authority,
            token,
            payload: [0; MAX_PACKET_BYTES],
            payload_len: 0,
            checksum: 0,
            received_checksum: 0,
            input_state: InputState::Idle,
            detached: false,
        }
    }

    pub fn runtime(&self) -> &R {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut R {
        &mut self.runtime
    }

    pub const fn is_detached(&self) -> bool {
        self.detached
    }

    /// Feed serial/network bytes and write at most one complete response.
    /// Partial packets stay buffered, so the caller can use this from either
    /// an interrupt-driven serial reader or a network receive loop.
    pub fn ingest(&mut self, input: &[u8], output: &mut [u8]) -> Result<Option<usize>, Error> {
        for byte in input {
            match self.input_state {
                InputState::Idle => {
                    if *byte == b'$' {
                        self.payload_len = 0;
                        self.checksum = 0;
                        self.input_state = InputState::Payload;
                    }
                }
                InputState::Payload => {
                    if *byte == b'#' {
                        self.input_state = InputState::ChecksumHigh;
                    } else if self.payload_len == self.payload.len() {
                        self.reset_input();
                        return self.write_ack(output, false);
                    } else {
                        self.payload[self.payload_len] = *byte;
                        self.payload_len += 1;
                        self.checksum = self.checksum.wrapping_add(*byte);
                    }
                }
                InputState::ChecksumHigh => {
                    self.received_checksum = decode_hex(*byte)? << 4;
                    self.input_state = InputState::ChecksumLow;
                }
                InputState::ChecksumLow => {
                    self.received_checksum |= decode_hex(*byte)?;
                    let valid = self.received_checksum == self.checksum;
                    self.input_state = InputState::Idle;
                    if !valid {
                        self.payload_len = 0;
                        return self.write_ack(output, false);
                    }
                    let length = self.handle_packet(output)?;
                    self.payload_len = 0;
                    return Ok(Some(length));
                }
            }
        }
        Ok(None)
    }

    fn handle_packet(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        let mut packet = [0; MAX_PACKET_BYTES];
        packet[..self.payload_len].copy_from_slice(&self.payload[..self.payload_len]);
        let payload = &packet[..self.payload_len];
        if payload.is_empty() {
            return self.write_reply(b"", output);
        }
        let required_operation = match payload[0] {
            b'G' | b'M' => DebugOperation::Write,
            b'c' | b's' | b'b' | b'k' => DebugOperation::Control,
            _ => DebugOperation::Read,
        };
        if !self.authority.permits(self.token, required_operation) {
            return self.write_error(3, output);
        }

        match payload[0] {
            b'?' => self.write_stop_reason(output),
            b'g' => self.write_registers(output),
            b'G' => self.receive_registers(&payload[1..], output),
            b'p' => self.read_register(&payload[1..], output),
            b'm' => self.read_memory(&payload[1..], output),
            b'M' => self.receive_memory(&payload[1..], output),
            b'c' => self.control(false, output),
            b's' => self.control(true, output),
            b'b' if payload == b"bc" => self.reverse_control(false, output),
            b'b' if payload == b"bs" => self.reverse_control(true, output),
            b'D' => {
                self.detached = true;
                self.write_reply(b"OK", output)
            }
            b'k' => self.write_reply(b"OK", output),
            b'q' if payload == b"qSupported" => {
                self.write_reply(b"PacketSize=200;qGhostOS:state+;qGhostOS:replay+", output)
            }
            b'q' if payload == b"qAttached" => self.write_reply(b"1", output),
            b'q' if payload == b"qGhostOS:state" => self.write_state(output),
            _ => self.write_error(1, output),
        }
    }

    fn write_stop_reason(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        let mut payload = [0; 3];
        let length = self.runtime.stop_reason().encode(&mut payload)?;
        self.write_reply(&payload[..length], output)
    }

    fn write_registers(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        let registers = self.runtime.registers()?;
        let mut payload = [0; MAX_REGISTERS * 16];
        let length = encode_registers(registers, &mut payload)?;
        self.write_reply(&payload[..length], output)
    }

    fn receive_registers(&mut self, bytes: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let mut registers = self.runtime.registers()?;
        registers.set_bytes(bytes)?;
        self.runtime.write_registers(registers)?;
        self.write_reply(b"OK", output)
    }

    fn read_register(&mut self, bytes: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let index = parse_hex(bytes).ok_or(Error::InvalidInput)? as usize;
        let value = self
            .runtime
            .registers()?
            .get(index)
            .ok_or(Error::NotFound)?;
        let mut payload = [0; 16];
        encode_u64(value, &mut payload);
        self.write_reply(&payload, output)
    }

    fn read_memory(&mut self, bytes: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let (address, length) = parse_range(bytes).ok_or(Error::InvalidInput)?;
        let max_bytes = (MAX_PACKET_BYTES - 1) / 2;
        if length > max_bytes {
            return self.write_error(2, output);
        }
        let mut memory = [0; (MAX_PACKET_BYTES - 1) / 2];
        let read = self.runtime.read_memory(address, &mut memory[..length])?;
        if read > length {
            return Err(Error::Runtime);
        }
        let mut payload = [0; MAX_PACKET_BYTES];
        let encoded = encode_bytes(&memory[..read], &mut payload)?;
        self.write_reply_with_checksum(&payload[..encoded], &memory[..read], output)
    }

    fn receive_memory(&mut self, bytes: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let Some(separator) = bytes.iter().position(|byte| *byte == b':') else {
            return Err(Error::InvalidInput);
        };
        let (address, length) = parse_range(&bytes[..separator]).ok_or(Error::InvalidInput)?;
        let encoded_length = length.checked_mul(2).ok_or(Error::InvalidInput)?;
        if length > (MAX_PACKET_BYTES - 1) / 2 || bytes.len() != separator + 1 + encoded_length {
            return Err(Error::InvalidInput);
        }
        let mut memory = [0; (MAX_PACKET_BYTES - 1) / 2];
        for (index, byte) in memory[..length].iter_mut().enumerate() {
            *byte = decode_hex_pair(
                bytes[separator + 1 + index * 2],
                bytes[separator + 2 + index * 2],
            )?;
        }
        self.runtime.write_memory(address, &memory[..length])?;
        self.write_reply(b"OK", output)
    }

    fn control(&mut self, step: bool, output: &mut [u8]) -> Result<usize, Error> {
        if step {
            self.runtime.step_execution()?;
        } else {
            self.runtime.continue_execution()?;
        }
        self.write_reply(b"OK", output)
    }

    fn reverse_control(&mut self, step: bool, output: &mut [u8]) -> Result<usize, Error> {
        if step {
            self.runtime.reverse_step_execution()?;
        } else {
            self.runtime.reverse_continue_execution()?;
        }
        self.write_reply(b"OK", output)
    }

    fn write_state(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        let mut faults = [DsmFault {
            process: ProcessId::new(1).expect("valid process"),
            node: NodeId::LOCAL,
            fault: PageFault::from_x86_error(0, 0),
            timestamp_us: 0,
            latency_ns: 0,
        }; MAX_DSM_FAULTS];
        let count = self.runtime.dsm_faults(&mut faults).min(15);
        let mut payload = [0; 64];
        payload[0] = b'F';
        payload[1] = hex(count as u8 >> 4);
        payload[2] = hex(count as u8);
        let mut offset = 3;
        for fault in faults.iter().take(count) {
            let value = fault.latency_ns as u64;
            if offset + 16 > payload.len() {
                break;
            }
            encode_u64(value, &mut payload[offset..offset + 16]);
            offset += 16;
        }
        self.write_reply(&payload[..offset], output)
    }

    fn write_error(&self, code: u8, output: &mut [u8]) -> Result<usize, Error> {
        let payload = [b'E', hex(code >> 4), hex(code)];
        self.write_reply(&payload, output)
    }

    fn write_ack(&self, output: &mut [u8], valid: bool) -> Result<Option<usize>, Error> {
        if output.is_empty() {
            return Err(Error::BufferTooSmall { required: 1 });
        }
        output[0] = if valid { b'+' } else { b'-' };
        Ok(Some(1))
    }

    fn write_reply(&self, payload: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        self.write_reply_with_checksum(payload, payload, output)
    }

    fn write_reply_with_checksum(
        &self,
        payload: &[u8],
        checksum_payload: &[u8],
        output: &mut [u8],
    ) -> Result<usize, Error> {
        let required = payload.len() + 5;
        if output.len() < required {
            return Err(Error::BufferTooSmall { required });
        }
        output[0] = b'+';
        output[1] = b'$';
        output[2..2 + payload.len()].copy_from_slice(payload);
        output[2 + payload.len()] = b'#';
        let checksum = checksum_payload
            .iter()
            .fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
        output[3 + payload.len()] = hex(checksum >> 4);
        output[4 + payload.len()] = hex(checksum);
        Ok(required)
    }

    fn reset_input(&mut self) {
        self.payload_len = 0;
        self.input_state = InputState::Idle;
    }
}

fn encode_registers(registers: RegisterFile, output: &mut [u8]) -> Result<usize, Error> {
    let required = registers.count() * 16;
    if output.len() < required {
        return Err(Error::BufferTooSmall { required });
    }
    for (index, value) in registers.values().iter().copied().enumerate() {
        encode_u64(value, &mut output[index * 16..index * 16 + 16]);
    }
    Ok(required)
}

fn encode_u64(value: u64, output: &mut [u8]) {
    for (index, byte) in value.to_le_bytes().iter().copied().enumerate() {
        output[index * 2] = hex(byte >> 4);
        output[index * 2 + 1] = hex(byte);
    }
}

fn encode_bytes(bytes: &[u8], output: &mut [u8]) -> Result<usize, Error> {
    if output.len() < bytes.len() * 2 {
        return Err(Error::BufferTooSmall {
            required: bytes.len() * 2,
        });
    }
    for (index, byte) in bytes.iter().copied().enumerate() {
        output[index * 2] = hex(byte >> 4);
        output[index * 2 + 1] = hex(byte);
    }
    Ok(bytes.len() * 2)
}

fn parse_range(bytes: &[u8]) -> Option<(u64, usize)> {
    let separator = bytes.iter().position(|byte| *byte == b',')?;
    let address = parse_hex(&bytes[..separator])?;
    let length = parse_hex(&bytes[separator + 1..])? as usize;
    Some((address, length))
}

fn parse_hex(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    bytes.iter().try_fold(0_u64, |value, byte| {
        value
            .checked_mul(16)?
            .checked_add(decode_hex(*byte).ok()? as u64)
    })
}

fn decode_hex(byte: u8) -> Result<u8, Error> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(Error::Corrupt),
    }
}

fn decode_hex_pair(high: u8, low: u8) -> Result<u8, Error> {
    Ok(decode_hex(high)? << 4 | decode_hex(low)?)
}

fn hex(value: u8) -> u8 {
    match value & 0xf {
        0..=9 => b'0' + (value & 0xf),
        value => b'a' + value - 10,
    }
}
