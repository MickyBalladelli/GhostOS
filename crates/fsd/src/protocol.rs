use synos_ipc::{Envelope, SharedBuffer};
use synos_status::Status;

pub const REQUEST_LABEL: u64 = 0x5346_5300_0000_0000;
pub const RESPONSE_LABEL: u64 = 0x5346_5301_0000_0000;
pub const MAX_IPC_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Operation {
    Open = 1,
    Close = 2,
    Read = 3,
    Write = 4,
    Metadata = 5,
    Delete = 6,
    Rename = 7,
    List = 8,
    SnapshotCreate = 9,
    SnapshotRelease = 10,
    SnapshotList = 11,
    Mount = 12,
    Unmount = 13,
    MountList = 14,
    GarbageCollect = 15,
}

impl Operation {
    pub const fn from_raw(raw: u16) -> Option<Self> {
        match raw {
            1 => Some(Self::Open),
            2 => Some(Self::Close),
            3 => Some(Self::Read),
            4 => Some(Self::Write),
            5 => Some(Self::Metadata),
            6 => Some(Self::Delete),
            7 => Some(Self::Rename),
            8 => Some(Self::List),
            9 => Some(Self::SnapshotCreate),
            10 => Some(Self::SnapshotRelease),
            11 => Some(Self::SnapshotList),
            12 => Some(Self::Mount),
            13 => Some(Self::Unmount),
            14 => Some(Self::MountList),
            15 => Some(Self::GarbageCollect),
            _ => None,
        }
    }

    pub const fn raw(self) -> u16 {
        self as u16
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Flags(u16);

impl Flags {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const CREATE: Self = Self(1 << 2);
    pub const TRUNCATE: Self = Self(1 << 3);
    pub const APPEND: Self = Self(1 << 4);
    pub const DELETE: Self = Self(1 << 5);
    pub const ADMIN: Self = Self(1 << 6);
    pub const READ_ONLY: Self = Self(1 << 7);

    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ProcessId(u64);

impl ProcessId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Capability(u64);

impl Capability {
    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw >> 32 == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Request {
    pub operation: Operation,
    pub flags: Flags,
    pub process: ProcessId,
    pub capability: Option<Capability>,
    pub offset: u64,
}

impl Request {
    pub const fn new(operation: Operation, process: ProcessId) -> Self {
        Self {
            operation,
            flags: Flags(0),
            process,
            capability: None,
            offset: 0,
        }
    }

    pub const fn with_flags(mut self, flags: Flags) -> Self {
        self.flags = flags;
        self
    }

    pub const fn with_capability(mut self, capability: Capability) -> Self {
        self.capability = Some(capability);
        self
    }

    pub const fn with_offset(mut self, offset: u64) -> Self {
        self.offset = offset;
        self
    }

    pub fn to_envelope(self, buffer: Option<SharedBuffer>) -> Envelope {
        Envelope {
            correlation: 0,
            label: REQUEST_LABEL | self.operation.raw() as u64,
            buffer,
            words: [
                self.process.raw(),
                self.capability.map_or(0, Capability::raw),
                self.offset,
                self.flags.bits() as u64,
            ],
        }
    }

    pub fn from_envelope(
        envelope: Envelope,
    ) -> Result<(Self, Option<SharedBuffer>), ProtocolError> {
        if envelope.label & !0xffff != REQUEST_LABEL {
            return Err(ProtocolError::WrongLabel);
        }
        let operation = Operation::from_raw((envelope.label & 0xffff) as u16)
            .ok_or(ProtocolError::UnknownOperation)?;
        let process = ProcessId::new(envelope.words[0]).ok_or(ProtocolError::InvalidProcess)?;
        let capability = if envelope.words[1] == 0 {
            None
        } else {
            Some(Capability::from_raw(envelope.words[1]).ok_or(ProtocolError::InvalidCapability)?)
        };
        let flags = Flags::from_bits(
            u16::try_from(envelope.words[3]).map_err(|_| ProtocolError::InvalidFlags)?,
        );
        Ok((
            Self {
                operation,
                flags,
                process,
                capability,
                offset: envelope.words[2],
            },
            envelope.buffer,
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Response {
    pub status: Status,
    pub values: [u64; 4],
}

impl Response {
    pub const fn success() -> Self {
        Self {
            status: Status::NORMAL,
            values: [0; 4],
        }
    }

    pub const fn with_value(mut self, index: usize, value: u64) -> Self {
        if index < self.values.len() {
            self.values[index] = value;
        }
        self
    }

    pub const fn error(status: Status) -> Self {
        Self {
            status,
            values: [0; 4],
        }
    }

    pub fn to_envelope(self, correlation: u128) -> Envelope {
        Envelope {
            correlation,
            label: RESPONSE_LABEL,
            buffer: None,
            words: [
                self.status.raw() as u64,
                self.values[0],
                self.values[1],
                self.values[2],
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    WrongLabel,
    UnknownOperation,
    InvalidProcess,
    InvalidCapability,
    InvalidFlags,
    MissingBuffer,
    InvalidBuffer,
    BufferTooLarge,
}
