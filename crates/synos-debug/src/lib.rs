#![no_std]
#![forbid(unsafe_code)]

//! Developer-facing debugging primitives that keep Ring 0 observation bounded
//! and let Ring 3 services continue while a debugger or crash collector runs.

pub mod coredump;
pub mod gdb;
pub mod probes;

use synos_status::{IntoStatus, Severity, Status, facility};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AccessDenied,
    BufferTooSmall { required: usize },
    Capacity,
    Corrupt,
    InvalidInput,
    InvalidProgram,
    NotFound,
    Runtime,
    Snapshot,
    Storage(synos_synfs::Error),
    Unsupported,
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::AccessDenied => Status::ACCESS_DENIED,
            Self::BufferTooSmall { .. } | Self::Capacity => Status::NO_SPACE,
            Self::Corrupt => Status::CORRUPT,
            Self::InvalidInput | Self::InvalidProgram => Status::INVALID_ARGUMENT,
            Self::NotFound => Status::NOT_FOUND,
            Self::Runtime | Self::Snapshot => {
                Status::new(Severity::Error, facility::KERNEL, 1, 0).expect("valid debug status")
            }
            Self::Storage(error) => error.status(),
            Self::Unsupported => {
                Status::new(Severity::Error, facility::KERNEL, 2, 0).expect("valid debug status")
            }
        }
    }
}
