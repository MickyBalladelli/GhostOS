use synos_ipc::{Envelope, SharedBuffer};
use synos_status::Status;

use crate::{SocketCapability, SocketRights};

pub const SOCKET_PROTOCOL_VERSION: u32 = 1;
pub const SOCKET_REQUEST_SCHEMA: u64 = 0x5359_4e4e_4554_5251;
pub const SOCKET_RESPONSE_SCHEMA: u64 = 0x5359_4e4e_4554_5253;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SocketOperation {
    OpenTcp = 1,
    Listen = 2,
    ConnectIpv4 = 3,
    Send = 4,
    Receive = 5,
    Close = 6,
    State = 7,
}

impl SocketOperation {
    const fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(Self::OpenTcp),
            2 => Some(Self::Listen),
            3 => Some(Self::ConnectIpv4),
            4 => Some(Self::Send),
            5 => Some(Self::Receive),
            6 => Some(Self::Close),
            7 => Some(Self::State),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    InvalidSchema,
    InvalidVersion,
    InvalidOperation,
    InvalidCapability,
    InvalidRights,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SocketRequest {
    pub correlation: u128,
    pub operation: SocketOperation,
    pub capability: Option<SocketCapability>,
    pub argument0: u64,
    pub argument1: u64,
    pub buffer: Option<SharedBuffer>,
}

impl SocketRequest {
    pub fn decode(envelope: Envelope) -> Result<Self, ProtocolError> {
        if envelope.label != SOCKET_REQUEST_SCHEMA {
            return Err(ProtocolError::InvalidSchema)
        }
        let version = (envelope.words[0] >> 32) as u32;
        if version != SOCKET_PROTOCOL_VERSION {
            return Err(ProtocolError::InvalidVersion)
        }
        let operation = SocketOperation::from_raw(envelope.words[0] as u32)
            .ok_or(ProtocolError::InvalidOperation)?;
        let capability = if envelope.words[1] == 0 {
            None
        } else {
            Some(
                SocketCapability::from_raw(envelope.words[1])
                    .ok_or(ProtocolError::InvalidCapability)?,
            )
        };
        Ok(Self {
            correlation: envelope.correlation,
            operation,
            capability,
            argument0: envelope.words[2],
            argument1: envelope.words[3],
            buffer: envelope.buffer,
        })
    }

    pub const fn open(
        correlation: u128,
        rights: SocketRights,
    ) -> Envelope {
        Self::encode(
            correlation,
            SocketOperation::OpenTcp,
            None,
            rights.bits() as u64,
            0,
            None,
        )
    }

    pub const fn encode(
        correlation: u128,
        operation: SocketOperation,
        capability: Option<SocketCapability>,
        argument0: u64,
        argument1: u64,
        buffer: Option<SharedBuffer>,
    ) -> Envelope {
        Envelope {
            correlation,
            label: SOCKET_REQUEST_SCHEMA,
            buffer,
            words: [
                ((SOCKET_PROTOCOL_VERSION as u64) << 32) | operation as u64,
                match capability {
                    Some(capability) => capability.raw(),
                    None => 0,
                },
                argument0,
                argument1,
            ],
        }
    }
}

pub const fn socket_response(
    correlation: u128,
    status: Status,
    capability: Option<SocketCapability>,
    value: u64,
    buffer: Option<SharedBuffer>,
) -> Envelope {
    Envelope {
        correlation,
        label: SOCKET_RESPONSE_SCHEMA,
        buffer,
        words: [
            SOCKET_PROTOCOL_VERSION as u64,
            status.raw() as u64,
            match capability {
                Some(capability) => capability.raw(),
                None => 0,
            },
            value,
        ],
    }
}
