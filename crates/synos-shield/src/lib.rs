#![no_std]
#![forbid(unsafe_code)]

use synos_status::{IntoStatus, Severity, Status, facility};
use synos_system_model::ContentId;

pub mod attestation;
pub mod fabric;
pub mod probes;

pub const DIGEST_BYTES: usize = 32;
pub(crate) const MAX_HMAC_MESSAGE_BYTES: usize = 2048;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    BufferTooSmall { required: usize },
    Capacity,
    Duplicate,
    Expired,
    FaultRateExceeded,
    InvalidConfiguration,
    InvalidInput,
    NotFound,
    Replay,
    SignatureMismatch,
    Unauthorized,
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::BufferTooSmall { .. } | Self::Capacity => Status::NO_SPACE,
            Self::Duplicate | Self::InvalidConfiguration | Self::InvalidInput => {
                Status::INVALID_ARGUMENT
            }
            Self::Expired | Self::FaultRateExceeded | Self::Replay | Self::Unauthorized => {
                Status::ACCESS_DENIED
            }
            Self::NotFound => Status::NOT_FOUND,
            Self::SignatureMismatch => Status::new(
                Severity::Error,
                facility::SECURITY,
                2,
                0,
            )
            .expect("valid shield status"),
        }
    }
}

pub(crate) fn constant_time_equal(left: &[u8; DIGEST_BYTES], right: &[u8; DIGEST_BYTES]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

pub(crate) fn hmac_sha256(
    key: &[u8; DIGEST_BYTES],
    message: &[u8],
) -> [u8; DIGEST_BYTES] {
    assert!(message.len() <= MAX_HMAC_MESSAGE_BYTES);

    let mut normalized = [0; 64];
    normalized[..key.len()].copy_from_slice(key);

    let mut inner = [0; 64 + MAX_HMAC_MESSAGE_BYTES];
    for (index, byte) in normalized.iter().enumerate() {
        inner[index] = byte ^ 0x36
    }
    inner[64..64 + message.len()].copy_from_slice(message);
    let inner_hash = ContentId::hash(&inner[..64 + message.len()]);

    let mut outer = [0; 96];
    for (index, byte) in normalized.iter().enumerate() {
        outer[index] = byte ^ 0x5c
    }
    outer[64..].copy_from_slice(inner_hash.as_bytes());
    *ContentId::hash(&outer).as_bytes()
}
