pub const PROTOCOL_VERSION: u8 = synos_abi::RPC_PROTOCOL_VERSION;
pub const FRAME_HEADER_BYTES: usize = synos_abi::RPC_FRAME_HEADER_BYTES;
pub const MAX_FRAME_BYTES: usize = synos_abi::RPC_MAX_FRAME_BYTES;
pub const PERFORMANCE_DIAGNOSTICS_BYTES: usize = 80;

pub(crate) const FLAG_CAPABILITY: u16 = synos_abi::RPC_CAPABILITY_FLAG;

pub use synos_abi::{RpcMethod as Method, RpcStatus};
use synos_ipc::{BufferError, BufferLease};
use synos_system_model::performance::{
    PerformanceBudget, PerformanceDiagnostics, PERFORMANCE_DIAGNOSTICS_VERSION,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameHeader {
    pub method: Method,
    pub flags: u16,
    pub request_id: u64,
    pub payload_bytes: u32,
    pub status: RpcStatus,
}

impl FrameHeader {
    pub fn encode(self, output: &mut [u8]) -> Result<(), ProtocolError> {
        synos_abi::RpcFrameHeader {
            method: self.method,
            flags: self.flags,
            request_id: self.request_id,
            payload_bytes: self.payload_bytes,
            status: self.status,
        }
        .encode(output)
        .map_err(ProtocolError::from)
    }

    pub fn decode(input: &[u8]) -> Result<Self, ProtocolError> {
        let header = synos_abi::RpcFrameHeader::decode(input).map_err(ProtocolError::from)?;
        Ok(Self {
            method: header.method,
            flags: header.flags,
            request_id: header.request_id,
            payload_bytes: header.payload_bytes,
            status: header.status,
        })
    }

    pub fn encode_guarded(
        self,
        output: &mut BufferLease<'_>,
    ) -> Result<(), ProtocolError> {
        let bytes = output
            .as_mut_slice()
            .map_err(buffer_error)?;
        self.encode(bytes)
    }

    pub fn decode_guarded(input: &BufferLease<'_>) -> Result<Self, ProtocolError> {
        Self::decode(input.as_slice().map_err(buffer_error)?)
    }
}

pub(crate) fn buffer_error(error: BufferError) -> ProtocolError {
    match error {
        BufferError::CapabilityDenied
        | BufferError::InvalidDescriptor
        | BufferError::OwnerMismatch => ProtocolError::InvalidFrame,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    AbiMismatch,
    BufferTooSmall,
    Capacity,
    DuplicateNode,
    InvalidFrame,
    InvalidStatus,
    InvalidUtf8,
    InvalidValue,
    MismatchedResponse,
    Transport(synos_protocol::ProtocolError),
    UnknownMethod,
}

impl From<synos_abi::FrameError> for ProtocolError {
    fn from(error: synos_abi::FrameError) -> Self {
        match error {
            synos_abi::FrameError::UnsupportedVersion
            | synos_abi::FrameError::SchemaMismatch => Self::AbiMismatch,
            synos_abi::FrameError::UnknownMethod => Self::UnknownMethod,
            synos_abi::FrameError::InvalidStatus => Self::InvalidStatus,
            synos_abi::FrameError::BufferTooSmall
            | synos_abi::FrameError::InvalidFlags
            | synos_abi::FrameError::InvalidFrame
            | synos_abi::FrameError::InvalidLength
            | synos_abi::FrameError::InvalidMagic => Self::InvalidFrame,
        }
    }
}

pub fn decode_frame_checked(
    guard: &mut synos_protocol::ProtocolGuard,
    sequence: u64,
    input: &[u8],
) -> Result<FrameHeader, ProtocolError> {
    guard
        .require_class(synos_protocol::TrafficClass::Sdk)
        .map_err(ProtocolError::Transport)?;
    guard
        .validate_message(input.len())
        .map_err(ProtocolError::Transport)?;
    let header = FrameHeader::decode(input)?;
    guard
        .accept_sequence(sequence)
        .map_err(ProtocolError::Transport)?;
    Ok(header)
}

pub(crate) fn write_u16(output: &mut [u8], offset: usize, value: u16) -> Result<(), ProtocolError> {
    write(output, offset, &value.to_be_bytes())
}

pub(crate) fn write_u32(output: &mut [u8], offset: usize, value: u32) -> Result<(), ProtocolError> {
    write(output, offset, &value.to_be_bytes())
}

pub(crate) fn write_u64(output: &mut [u8], offset: usize, value: u64) -> Result<(), ProtocolError> {
    write(output, offset, &value.to_be_bytes())
}

pub(crate) fn write(output: &mut [u8], offset: usize, value: &[u8]) -> Result<(), ProtocolError> {
    let end = offset
        .checked_add(value.len())
        .ok_or(ProtocolError::BufferTooSmall)?;
    let destination = output
        .get_mut(offset..end)
        .ok_or(ProtocolError::BufferTooSmall)?;
    destination.copy_from_slice(value);
    Ok(())
}

pub(crate) fn read_u16(input: &[u8], offset: usize) -> Result<u16, ProtocolError> {
    let bytes = read_array::<2>(input, offset)?;
    Ok(u16::from_be_bytes(bytes))
}

pub(crate) fn read_u32(input: &[u8], offset: usize) -> Result<u32, ProtocolError> {
    let bytes = read_array::<4>(input, offset)?;
    Ok(u32::from_be_bytes(bytes))
}

pub(crate) fn read_u64(input: &[u8], offset: usize) -> Result<u64, ProtocolError> {
    let bytes = read_array::<8>(input, offset)?;
    Ok(u64::from_be_bytes(bytes))
}

pub(crate) fn read_array<const N: usize>(
    input: &[u8],
    offset: usize,
) -> Result<[u8; N], ProtocolError> {
    input
        .get(offset..offset.saturating_add(N))
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(ProtocolError::InvalidFrame)
}

pub(crate) fn encode_performance_diagnostics(
    diagnostics: PerformanceDiagnostics,
    output: &mut [u8],
) -> Result<(), ProtocolError> {
    let output = output
        .get_mut(..PERFORMANCE_DIAGNOSTICS_BYTES)
        .ok_or(ProtocolError::BufferTooSmall)?;
    output.fill(0);
    write_u16(output, 0, PERFORMANCE_DIAGNOSTICS_VERSION)?;
    write_u16(output, 2, diagnostics.budget_exceeded as u16)?;
    write_u64(output, 4, diagnostics.queue_wait_us)?;
    write_u64(output, 12, diagnostics.service_time_us)?;
    write_u32(output, 20, diagnostics.retries)?;
    write_u32(output, 24, diagnostics.request_bytes)?;
    write_u32(output, 28, diagnostics.response_bytes)?;
    write_u64(output, 32, diagnostics.tail_latency_us)?;
    write_u64(output, 40, diagnostics.budget.queue_wait_us)?;
    write_u64(output, 48, diagnostics.budget.service_time_us)?;
    write_u64(output, 56, diagnostics.budget.tail_latency_us)?;
    write_u32(output, 64, diagnostics.budget.max_retries)?;
    write_u32(output, 68, diagnostics.budget.max_request_bytes)?;
    write_u32(output, 72, diagnostics.budget.max_response_bytes)?;
    Ok(())
}

pub(crate) fn decode_performance_diagnostics(
    input: &[u8],
) -> Result<PerformanceDiagnostics, ProtocolError> {
    if input.len() != PERFORMANCE_DIAGNOSTICS_BYTES
        || read_u16(input, 0)? != PERFORMANCE_DIAGNOSTICS_VERSION
    {
        return Err(ProtocolError::InvalidFrame)
    }
    let budget = PerformanceBudget {
        queue_wait_us: read_u64(input, 40)?,
        service_time_us: read_u64(input, 48)?,
        tail_latency_us: read_u64(input, 56)?,
        max_retries: read_u32(input, 64)?,
        max_request_bytes: read_u32(input, 68)?,
        max_response_bytes: read_u32(input, 72)?,
    };
    Ok(PerformanceDiagnostics::new(
        read_u64(input, 4)?,
        read_u64(input, 12)?,
        read_u32(input, 20)?,
        read_u32(input, 24)?,
        read_u32(input, 28)?,
        read_u64(input, 32)?,
        budget,
    ))
}
