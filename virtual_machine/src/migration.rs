//! Bounded decoding and authentication for migration checkpoint frames.

use crate::{snapshot_digest, SnapshotAuthKey, SnapshotError, SnapshotSchema, VmSnapshot};

const MIGRATION_AUTH_DOMAIN: &[u8] = b"SYNOS-MIGRATION-HMAC-SHA256-V3";
pub const MIGRATION_NONCE_BYTES: usize = 32;
pub const MAX_MIGRATION_ALLOCATION_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const FRAME_HEADER_BYTES: usize = 8 + 32 + 8;
const FRAME_TAG_BYTES: usize = 32;

#[derive(Debug)]
pub struct MigrationCheckpointFrame {
    pub issued_at: u64,
    pub checkpoint_id: [u8; 32],
    pub bytes: Vec<u8>,
    pub snapshot: VmSnapshot,
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationFrameError {
    #[error("migration frame is truncated or has trailing bytes")]
    InvalidLength,
    #[error("migration payload is too large: {0} bytes")]
    PayloadTooLarge(u64),
    #[error("migration authentication failed")]
    Authentication,
    #[error("migration checkpoint identity does not match its payload")]
    Identity,
    #[error("received invalid authenticated VM checkpoint: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("received checkpoint does not match negotiated schema")]
    Schema,
}

pub fn migration_checkpoint_tag(
    key: SnapshotAuthKey,
    schema: SnapshotSchema,
    sender_nonce: &[u8; MIGRATION_NONCE_BYTES],
    receiver_nonce: &[u8; MIGRATION_NONCE_BYTES],
    issued_at: u64,
    checkpoint_id: &[u8; 32],
    length: u64,
    payload: &[u8],
) -> [u8; 32] {
    let min_version = schema.min_version.to_le_bytes();
    let max_version = schema.max_version.to_le_bytes();
    let features = schema.features.bits().to_le_bytes();
    let issued_at = issued_at.to_le_bytes();
    let length = length.to_le_bytes();
    key.authenticate_parts(&[
        MIGRATION_AUTH_DOMAIN,
        b"frame",
        &key.key_id(),
        sender_nonce,
        receiver_nonce,
        &min_version,
        &max_version,
        &features,
        &issued_at,
        checkpoint_id,
        &length,
        payload,
    ])
}

pub fn validate_migration_checkpoint(
    key: SnapshotAuthKey,
    schema: SnapshotSchema,
    sender_nonce: &[u8; MIGRATION_NONCE_BYTES],
    receiver_nonce: &[u8; MIGRATION_NONCE_BYTES],
    issued_at: u64,
    checkpoint_id: [u8; 32],
    payload: Vec<u8>,
    received_tag: [u8; 32],
) -> Result<MigrationCheckpointFrame, MigrationFrameError> {
    let length = u64::try_from(payload.len()).map_err(|_| MigrationFrameError::InvalidLength)?;
    if length == 0 || length > MAX_MIGRATION_ALLOCATION_BYTES {
        return Err(MigrationFrameError::PayloadTooLarge(length))
    }
    let expected_tag = migration_checkpoint_tag(
        key,
        schema,
        sender_nonce,
        receiver_nonce,
        issued_at,
        &checkpoint_id,
        length,
        &payload,
    );
    key.verify_tag(&expected_tag, &received_tag)
        .map_err(|_| MigrationFrameError::Authentication)?;
    if snapshot_digest(&payload) != checkpoint_id {
        return Err(MigrationFrameError::Identity)
    }
    let snapshot = VmSnapshot::from_authenticated_bytes(&payload, key)?;
    if snapshot.format_version != schema.max_version
        || !schema.accepts(snapshot.format_version, snapshot.feature_flags)
    {
        return Err(MigrationFrameError::Schema)
    }
    Ok(MigrationCheckpointFrame {
        issued_at,
        checkpoint_id,
        bytes: payload,
        snapshot,
    })
}

pub fn decode_migration_checkpoint(
    frame: &[u8],
    key: SnapshotAuthKey,
    schema: SnapshotSchema,
    sender_nonce: &[u8; MIGRATION_NONCE_BYTES],
    receiver_nonce: &[u8; MIGRATION_NONCE_BYTES],
) -> Result<MigrationCheckpointFrame, MigrationFrameError> {
    if frame.len() < FRAME_HEADER_BYTES + FRAME_TAG_BYTES {
        return Err(MigrationFrameError::InvalidLength)
    }
    let issued_at = u64::from_le_bytes(
        frame[..8]
            .try_into()
            .map_err(|_| MigrationFrameError::InvalidLength)?,
    );
    let checkpoint_id = frame[8..40]
        .try_into()
        .map_err(|_| MigrationFrameError::InvalidLength)?;
    let length = u64::from_le_bytes(
        frame[40..48]
            .try_into()
            .map_err(|_| MigrationFrameError::InvalidLength)?,
    );
    if length == 0 || length > MAX_MIGRATION_ALLOCATION_BYTES {
        return Err(MigrationFrameError::PayloadTooLarge(length))
    }
    let payload_length = usize::try_from(length).map_err(|_| MigrationFrameError::InvalidLength)?;
    let expected_length = FRAME_HEADER_BYTES
        .checked_add(payload_length)
        .and_then(|length| length.checked_add(FRAME_TAG_BYTES))
        .ok_or(MigrationFrameError::InvalidLength)?;
    if frame.len() != expected_length {
        return Err(MigrationFrameError::InvalidLength)
    }
    let payload = frame[FRAME_HEADER_BYTES..FRAME_HEADER_BYTES + payload_length].to_vec();
    let received_tag = frame[FRAME_HEADER_BYTES + payload_length..]
        .try_into()
        .map_err(|_| MigrationFrameError::InvalidLength)?;
    validate_migration_checkpoint(
        key,
        schema,
        sender_nonce,
        receiver_nonce,
        issued_at,
        checkpoint_id,
        payload,
        received_tag,
    )
}
