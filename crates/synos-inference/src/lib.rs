#![no_std]
#![forbid(unsafe_code)]

use synos_status::{IntoStatus, Severity, Status, facility};

pub mod agent_state;
pub mod gateway;
pub mod protocol;
pub mod service;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AgentNotFound,
    BufferTooSmall { required: usize },
    Capacity,
    CorruptState,
    DuplicateAgent,
    DuplicateModel,
    Filesystem(synos_synfs::Error),
    InvalidRequest,
    Llm(synos_llm::Error),
    ModelNotFound,
    RequestNotFound,
    SnapshotNotDue,
    StaleSequence,
    UnsupportedEndpoint,
    UnsupportedProtocol,
}

impl From<synos_llm::Error> for Error {
    fn from(error: synos_llm::Error) -> Self {
        Self::Llm(error)
    }
}

impl From<synos_synfs::Error> for Error {
    fn from(error: synos_synfs::Error) -> Self {
        Self::Filesystem(error)
    }
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::Filesystem(error) => error.status(),
            Self::Llm(error) => error.status(),
            Self::AgentNotFound | Self::ModelNotFound | Self::RequestNotFound => Status::NOT_FOUND,
            Self::BufferTooSmall { .. } | Self::Capacity => Status::NO_SPACE,
            Self::CorruptState => Status::CORRUPT,
            Self::SnapshotNotDue => Status::BUSY,
            Self::DuplicateAgent | Self::DuplicateModel => {
                Status::new(Severity::Error, facility::LLM, 3, 0).expect("valid inference status")
            }
            Self::StaleSequence => {
                Status::new(Severity::Warning, facility::LLM, 4, 0).expect("valid inference status")
            }
            Self::InvalidRequest | Self::UnsupportedEndpoint | Self::UnsupportedProtocol => {
                Status::INVALID_ARGUMENT
            }
        }
    }
}
