#![no_std]
#![forbid(unsafe_code)]

use synos_status::{IntoStatus, Severity, Status, facility};

pub mod allocator;
pub mod inference;
pub mod kv_cache;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AllocationNotFound,
    CacheNotFound,
    Capacity,
    CorruptRecoveryRecord,
    Fabric(synos_fabric::Error),
    InvalidHandle,
    InvalidRange,
    NoFailoverReplica,
    RequestNotFound,
    StaleCheckpoint,
}

impl From<synos_fabric::Error> for Error {
    fn from(error: synos_fabric::Error) -> Self {
        Self::Fabric(error)
    }
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::Fabric(error) => error.status(),
            Self::AllocationNotFound
            | Self::CacheNotFound
            | Self::RequestNotFound => Status::NOT_FOUND,
            Self::Capacity => Status::NO_SPACE,
            Self::CorruptRecoveryRecord => Status::CORRUPT,
            Self::NoFailoverReplica => {
                Status::new(Severity::Fatal, facility::LLM, 1, 0)
                    .expect("valid LLM status")
            }
            Self::StaleCheckpoint => {
                Status::new(Severity::Warning, facility::LLM, 2, 0)
                    .expect("valid LLM status")
            }
            Self::InvalidHandle | Self::InvalidRange => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(transparent)]
pub struct RequestId(u64);

impl RequestId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}
