#![no_std]
#![forbid(unsafe_code)]

//! Heap-free, pure-Rust compute contracts for SynOS.
//!
//! Frameworks import tensors from capability-mapped IPC pages. GPU and NPU
//! drivers stay in Ring 3 and consume bounded asynchronous command queues.

use synos_status::{IntoStatus, Severity, Status, facility};

pub mod accelerator;
pub mod framework;
pub mod tensor;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AccessDenied,
    BufferTooSmall,
    DuplicateBinding,
    InvalidCapability,
    InvalidDevice,
    InvalidDispatch,
    InvalidKernel,
    InvalidModel,
    InvalidShape,
    InvalidStride,
    InvalidTensor,
    QueueFull,
    ReadOnly,
    RegionMismatch,
    RuntimeFailure,
    UnsupportedDevice,
}

impl From<synos_platform_io::Error> for Error {
    fn from(error: synos_platform_io::Error) -> Self {
        match error {
            synos_platform_io::Error::QueueFull => Self::QueueFull,
            synos_platform_io::Error::InvalidBufferAccess => Self::AccessDenied,
            synos_platform_io::Error::InvalidDevice => Self::InvalidDevice,
            _ => Self::InvalidDispatch,
        }
    }
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::AccessDenied | Self::ReadOnly => Status::ACCESS_DENIED,
            Self::BufferTooSmall | Self::QueueFull => Status::NO_SPACE,
            Self::InvalidDevice => Status::NOT_FOUND,
            Self::RuntimeFailure => {
                Status::new(Severity::Error, facility::COMPUTE, 1, 0).expect("valid compute status")
            }
            Self::UnsupportedDevice => Status::new(Severity::Warning, facility::COMPUTE, 2, 0)
                .expect("valid compute status"),
            Self::DuplicateBinding
            | Self::InvalidCapability
            | Self::InvalidDispatch
            | Self::InvalidKernel
            | Self::InvalidModel
            | Self::InvalidShape
            | Self::InvalidStride
            | Self::InvalidTensor
            | Self::RegionMismatch => Status::INVALID_ARGUMENT,
        }
    }
}
