pub const PROTOCOL_VERSION: u8 = 1;
pub const FRAME_HEADER_BYTES: usize = 24;
pub const MAX_FRAME_BYTES: usize = 4096;

const MAGIC: &[u8; 4] = b"SYRP";
pub(crate) const FLAG_CAPABILITY: u16 = 1;
pub(crate) const KNOWN_FLAGS: u16 = FLAG_CAPABILITY;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Method {
    ClusterState = 1,
    SubmitJob = 2,
    DelegateCapability = 3,
    TopologyState = 4,
    ClusterSummary = 5,
    ClusterMembers = 6,
    ClusterInvitations = 7,
    ClusterJoinPlan = 8,
    ClusterLeavePlan = 9,
    ClusterHealth = 10,
    ClusterResources = 11,
    ClusterAudit = 12,
    ClusterCreate = 13,
    ClusterJoin = 14,
    ClusterLeave = 15,
    ClusterRemove = 16,
    Subscribe = 17,
    Poll = 18,
}

impl Method {
    pub(crate) const fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::ClusterState),
            2 => Ok(Self::SubmitJob),
            3 => Ok(Self::DelegateCapability),
            4 => Ok(Self::TopologyState),
            5 => Ok(Self::ClusterSummary),
            6 => Ok(Self::ClusterMembers),
            7 => Ok(Self::ClusterInvitations),
            8 => Ok(Self::ClusterJoinPlan),
            9 => Ok(Self::ClusterLeavePlan),
            10 => Ok(Self::ClusterHealth),
            11 => Ok(Self::ClusterResources),
            12 => Ok(Self::ClusterAudit),
            13 => Ok(Self::ClusterCreate),
            14 => Ok(Self::ClusterJoin),
            15 => Ok(Self::ClusterLeave),
            16 => Ok(Self::ClusterRemove),
            17 => Ok(Self::Subscribe),
            18 => Ok(Self::Poll),
            _ => Err(ProtocolError::UnknownMethod),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum RpcStatus {
    Ok = 0,
    InvalidRequest = 1,
    Unauthenticated = 2,
    AccessDenied = 3,
    NotFound = 4,
    Busy = 5,
    Capacity = 6,
    Internal = 7,
}

impl RpcStatus {
    pub(crate) const fn from_wire(value: u16) -> Result<Self, ProtocolError> {
        match value {
            0 => Ok(Self::Ok),
            1 => Ok(Self::InvalidRequest),
            2 => Ok(Self::Unauthenticated),
            3 => Ok(Self::AccessDenied),
            4 => Ok(Self::NotFound),
            5 => Ok(Self::Busy),
            6 => Ok(Self::Capacity),
            7 => Ok(Self::Internal),
            _ => Err(ProtocolError::InvalidStatus),
        }
    }
}

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
        if output.len() < FRAME_HEADER_BYTES
            || self.flags & !KNOWN_FLAGS != 0
            || self.payload_bytes as usize > MAX_FRAME_BYTES - FRAME_HEADER_BYTES
        {
            return Err(ProtocolError::InvalidFrame);
        }
        output[..FRAME_HEADER_BYTES].fill(0);
        output[0..4].copy_from_slice(MAGIC);
        output[4] = PROTOCOL_VERSION;
        output[5] = self.method as u8;
        output[6..8].copy_from_slice(&self.flags.to_be_bytes());
        output[8..16].copy_from_slice(&self.request_id.to_be_bytes());
        output[16..20].copy_from_slice(&self.payload_bytes.to_be_bytes());
        output[20..22].copy_from_slice(&(self.status as u16).to_be_bytes());
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<Self, ProtocolError> {
        if input.len() < FRAME_HEADER_BYTES || &input[0..4] != MAGIC || input[4] != PROTOCOL_VERSION
        {
            return Err(ProtocolError::InvalidFrame);
        }
        let flags = read_u16(input, 6)?;
        let payload_bytes = read_u32(input, 16)?;
        if flags & !KNOWN_FLAGS != 0
            || payload_bytes as usize > MAX_FRAME_BYTES - FRAME_HEADER_BYTES
            || input.len() < FRAME_HEADER_BYTES + payload_bytes as usize
        {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(Self {
            method: Method::from_wire(input[5])?,
            flags,
            request_id: read_u64(input, 8)?,
            payload_bytes,
            status: RpcStatus::from_wire(read_u16(input, 20)?)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    BufferTooSmall,
    Capacity,
    DuplicateNode,
    InvalidFrame,
    InvalidStatus,
    InvalidUtf8,
    InvalidValue,
    MismatchedResponse,
    UnknownMethod,
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
