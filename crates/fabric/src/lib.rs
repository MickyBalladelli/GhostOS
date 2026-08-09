#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

use synos_status::{IntoStatus, Severity, Status, facility};

pub mod cluster;
pub mod cxl;
pub mod dsm;
pub mod memory;
pub mod rcu;

pub use cxl::{
    CxlBandwidthDecision, CxlBandwidthPolicy, CxlBandwidthQos, CxlChannel,
};

pub const PAGE_SIZE: u64 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct NodeId(u32);

impl NodeId {
    pub const LOCAL: Self = Self(1);

    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn from_valid_raw(raw: u32) -> Self {
        Self(raw)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AddressConflict,
    Alignment,
    BandwidthThrottled,
    Busy,
    Capacity,
    CorruptPacket,
    DecoderCommitFailed,
    DeviceNotFound,
    ExpiredLease,
    InvalidAddress,
    InvalidDevice,
    InvalidRange,
    InvalidQosPolicy,
    LeaseNotFound,
    NodeFailed,
    NodeNotFenced,
    NotOwner,
    Unsupported,
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::Busy | Self::BandwidthThrottled => Status::BUSY,
            Self::Capacity => Status::NO_SPACE,
            Self::DeviceNotFound | Self::LeaseNotFound => Status::NOT_FOUND,
            Self::NotOwner => Status::ACCESS_DENIED,
            Self::CorruptPacket => Status::CORRUPT,
            Self::ExpiredLease => {
                Status::new(Severity::Warning, facility::FABRIC, 1, 0)
                    .expect("valid fabric status")
            }
            Self::NodeFailed => {
                Status::new(Severity::Error, facility::FABRIC, 2, 0)
                    .expect("valid fabric status")
            }
            Self::NodeNotFenced => {
                Status::new(Severity::Error, facility::FABRIC, 4, 0)
                    .expect("valid fabric status")
            }
            Self::DecoderCommitFailed => {
                Status::new(Severity::Error, facility::FABRIC, 3, 0)
                    .expect("valid fabric status")
            }
            Self::AddressConflict
            | Self::Alignment
            | Self::InvalidAddress
            | Self::InvalidDevice
            | Self::InvalidRange
            | Self::InvalidQosPolicy
            | Self::Unsupported => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressRange {
    pub start: u64,
    pub length: u64,
}

impl AddressRange {
    pub const fn new(start: u64, length: u64) -> Result<Self, Error> {
        if length == 0 || start.checked_add(length).is_none() {
            Err(Error::InvalidRange)
        } else {
            Ok(Self { start, length })
        }
    }

    pub const fn end(self) -> u64 {
        self.start + self.length
    }

    pub const fn contains(self, address: u64) -> bool {
        address >= self.start && address < self.end()
    }

    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.end() && other.start < self.end()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    Read,
    Write,
    Execute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageFault {
    pub virtual_address: u64,
    pub access: Access,
    pub user: bool,
    pub present: bool,
    pub reserved_bit: bool,
    pub instruction_fetch: bool,
}

impl PageFault {
    pub const fn from_x86_error(virtual_address: u64, error: u64) -> Self {
        let instruction_fetch = error & (1 << 4) != 0;
        Self {
            virtual_address,
            access: if error & (1 << 1) != 0 {
                Access::Write
            } else {
                Access::Read
            },
            user: error & (1 << 2) != 0,
            present: error & 1 != 0,
            reserved_bit: error & (1 << 3) != 0,
            instruction_fetch,
        }
    }

    pub const fn page_address(self) -> u64 {
        self.virtual_address & !(PAGE_SIZE - 1)
    }
}
