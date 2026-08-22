#![no_std]
#![forbid(unsafe_code)]

use ghostos_status::{IntoStatus, Severity, Status, facility};

pub mod capability;
pub mod enclave;
pub mod fabric;

pub use capability::{
    CapabilityAuthority, CapabilityRights, ConfidentialCapability, ProtectedResource,
};
pub use enclave::{
    EnclaveBinding, EnclaveClass, EnclaveManager, EnclavePlatform, DEFAULT_ENCLAVE_CAPACITY,
    DEFAULT_MEMORY_RANGE_CAPACITY,
};
pub use fabric::{
    EncryptedDsmFrame, FabricTransport, MlKemCiphertext, MlKemKeypair, MlKemPublicKey,
    MlKemSecretKey, NonceReplayGuard, SharedSecret, MAX_PACKET_BYTES,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AuthenticationFailed,
    Attestation(ghostos_shield::Error),
    Capacity,
    Expired,
    Fabric(ghostos_fabric::Error),
    InvalidConfiguration,
    InvalidInput,
    NotAdmitted,
    NotFound,
    NotProtected,
    Replay,
    Unauthorized,
}

impl From<ghostos_shield::Error> for Error {
    fn from(error: ghostos_shield::Error) -> Self {
        Self::Attestation(error)
    }
}

impl From<ghostos_fabric::Error> for Error {
    fn from(error: ghostos_fabric::Error) -> Self {
        Self::Fabric(error)
    }
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::Attestation(error) => error.status(),
            Self::Fabric(error) => error.status(),
            Self::Capacity => Status::NO_SPACE,
            Self::Expired | Self::NotAdmitted | Self::NotProtected | Self::Unauthorized => {
                Status::ACCESS_DENIED
            }
            Self::AuthenticationFailed | Self::Replay => Status::new(
                Severity::Error,
                facility::SECURITY,
                30,
                0,
            )
            .expect("valid confidential status"),
            Self::InvalidConfiguration | Self::InvalidInput => Status::INVALID_ARGUMENT,
            Self::NotFound => Status::NOT_FOUND,
        }
    }
}
