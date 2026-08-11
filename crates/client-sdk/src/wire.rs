pub const PROTOCOL_VERSION: u8 = synos_abi::RPC_PROTOCOL_VERSION;
pub const FRAME_HEADER_BYTES: usize = synos_abi::RPC_FRAME_HEADER_BYTES;
pub const MAX_FRAME_BYTES: usize = synos_abi::RPC_MAX_FRAME_BYTES;

pub(crate) const FLAG_CAPABILITY: u16 = synos_abi::RPC_CAPABILITY_FLAG;

pub use synos_abi::{RpcMethod as Method, RpcStatus};
use synos_ipc::{BufferError, BufferLease};

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
