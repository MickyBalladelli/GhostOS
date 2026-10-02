//! Bounded decoding and authentication for migration checkpoint frames.

use crate::{snapshot_digest, SnapshotAuthKey, SnapshotError, SnapshotSchema, VmSnapshot};

use std::ffi::c_void;
pub const MIGRATION_NONCE_BYTES: usize = 32;
pub const MAX_MIGRATION_ALLOCATION_BYTES: u64 = 8 * 1024 * 1024 * 1024;
#[cfg(test)]
const FRAME_HEADER_BYTES: usize = 8 + 32 + 8;
#[cfg(test)]
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

#[repr(C)]
struct CPart { bytes: *const u8, length: usize }
#[repr(C)]
struct CSchema { min_version: u32, max_version: u32, features: u64 }
impl From<SnapshotSchema> for CSchema {
    fn from(schema: SnapshotSchema) -> Self {
        Self { min_version: schema.min_version, max_version: schema.max_version,
            features: schema.features.bits() }
    }
}
#[repr(C)]
#[derive(Default)]
struct CFrame {
    issued_at: u64,
    checkpoint_id: [u8; 32],
    payload: *const u8,
    payload_length: usize,
    tag: [u8; 32],
}
#[repr(C)]
struct CIo {
    authenticate: unsafe extern "C" fn(*mut c_void, *const CPart, usize, *mut u8),
    digest: unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut u8),
    snapshot: unsafe extern "C" fn(*mut c_void, *const u8, usize) -> u32,
    context: *mut c_void,
}
unsafe extern "C" {
    fn ghostos_vm_migration_decode(bytes: *const u8, length: usize,
        frame: *mut CFrame, declared_length: *mut u64) -> u32;
    fn ghostos_vm_migration_tag(frame: *const CFrame, schema: CSchema,
        key_id: *const u8, sender: *const u8, receiver: *const u8,
        length: u64, io: *const CIo, tag: *mut u8);
    fn ghostos_vm_migration_validate(frame: *const CFrame, schema: CSchema,
        key_id: *const u8, sender: *const u8, receiver: *const u8, io: *const CIo) -> u32;
}

struct Context {
    key: SnapshotAuthKey,
    schema: SnapshotSchema,
    snapshot: Option<VmSnapshot>,
    error: Option<SnapshotError>,
}

unsafe fn bytes<'a>(ptr: *const u8, length: usize) -> &'a [u8] {
    if length == 0 { &[] } else { unsafe { std::slice::from_raw_parts(ptr, length) } }
}

unsafe extern "C" fn authenticate(raw: *mut c_void, parts: *const CPart, count: usize, out: *mut u8) {
    let context = unsafe { &mut *raw.cast::<Context>() };
    let parts = unsafe { std::slice::from_raw_parts(parts, count) };
    let parts: Vec<&[u8]> = parts.iter().map(|part| unsafe { bytes(part.bytes, part.length) }).collect();
    let tag = context.key.authenticate_parts(&parts);
    unsafe { std::ptr::copy_nonoverlapping(tag.as_ptr(), out, tag.len()) };
}

unsafe extern "C" fn digest(_raw: *mut c_void, ptr: *const u8, length: usize, out: *mut u8) {
    let digest = snapshot_digest(unsafe { bytes(ptr, length) });
    unsafe { std::ptr::copy_nonoverlapping(digest.as_ptr(), out, digest.len()) };
}

unsafe extern "C" fn snapshot(raw: *mut c_void, ptr: *const u8, length: usize) -> u32 {
    let context = unsafe { &mut *raw.cast::<Context>() };
    match VmSnapshot::from_authenticated_bytes(unsafe { bytes(ptr, length) }, context.key) {
        Ok(snapshot) => {
            if snapshot.format_version != context.schema.max_version
                || !context.schema.accepts(snapshot.format_version, snapshot.feature_flags) {
                return 6
            }
            context.snapshot = Some(snapshot);
            0
        }
        Err(error) => { context.error = Some(error); 5 }
    }
}

fn io(context: &mut Context) -> CIo {
    CIo { authenticate, digest, snapshot, context: (context as *mut Context).cast() }
}

fn result(code: u32, length: u64, context: &mut Context) -> Result<(), MigrationFrameError> {
    match code {
        0 => Ok(()),
        1 => Err(MigrationFrameError::InvalidLength),
        2 => Err(MigrationFrameError::PayloadTooLarge(length)),
        3 => Err(MigrationFrameError::Authentication),
        4 => Err(MigrationFrameError::Identity),
        5 => Err(MigrationFrameError::Snapshot(context.error.take().expect("snapshot callback error"))),
        6 => Err(MigrationFrameError::Schema),
        _ => unreachable!("invalid C migration result"),
    }
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
    let frame = CFrame { issued_at, checkpoint_id: *checkpoint_id,
        payload: payload.as_ptr(), payload_length: payload.len(), tag: [0; 32] };
    let mut context = Context { key, schema, snapshot: None, error: None };
    let io = io(&mut context);
    let mut tag = [0; 32];
    unsafe { ghostos_vm_migration_tag(&frame, schema.into(), key.key_id().as_ptr(),
        sender_nonce.as_ptr(), receiver_nonce.as_ptr(), length, &io, tag.as_mut_ptr()) };
    tag
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
    let frame = CFrame { issued_at, checkpoint_id, payload: payload.as_ptr(),
        payload_length: payload.len(), tag: received_tag };
    let mut context = Context { key, schema, snapshot: None, error: None };
    let io = io(&mut context);
    let code = unsafe { ghostos_vm_migration_validate(&frame, schema.into(), key.key_id().as_ptr(),
        sender_nonce.as_ptr(), receiver_nonce.as_ptr(), &io) };
    result(code, payload.len() as u64, &mut context)?;
    Ok(MigrationCheckpointFrame { issued_at, checkpoint_id, bytes: payload,
        snapshot: context.snapshot.expect("validated C migration snapshot") })
}

pub fn decode_migration_checkpoint(
    frame: &[u8],
    key: SnapshotAuthKey,
    schema: SnapshotSchema,
    sender_nonce: &[u8; MIGRATION_NONCE_BYTES],
    receiver_nonce: &[u8; MIGRATION_NONCE_BYTES],
) -> Result<MigrationCheckpointFrame, MigrationFrameError> {
    let mut decoded = CFrame::default();
    let mut length = 0;
    let code = unsafe { ghostos_vm_migration_decode(frame.as_ptr(), frame.len(), &mut decoded, &mut length) };
    let mut context = Context { key, schema, snapshot: None, error: None };
    result(code, length, &mut context)?;
    let payload = unsafe { bytes(decoded.payload, decoded.payload_length) }.to_vec();
    validate_migration_checkpoint(key, schema, sender_nonce, receiver_nonce,
        decoded.issued_at, decoded.checkpoint_id, payload, decoded.tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_migration_stream_does_not_echo_payload_or_authentication_data() {
        let secret = b"migration-stream-secret";
        let mut frame = vec![0; FRAME_HEADER_BYTES + secret.len() + FRAME_TAG_BYTES];
        frame[40..48].copy_from_slice(&(secret.len() as u64).to_le_bytes());
        frame[FRAME_HEADER_BYTES..FRAME_HEADER_BYTES + secret.len()]
            .copy_from_slice(secret);

        let error = decode_migration_checkpoint(
            &frame,
            SnapshotAuthKey::new([0x42; 32]),
            SnapshotSchema::local(),
            &[0x11; MIGRATION_NONCE_BYTES],
            &[0x22; MIGRATION_NONCE_BYTES],
        )
        .expect_err("unauthenticated migration stream must fail");

        assert!(matches!(error, MigrationFrameError::Authentication));
        assert!(!error.to_string().contains("migration-stream-secret"));
    }
}
