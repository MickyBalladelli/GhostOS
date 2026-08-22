#![no_main]

use libfuzzer_sys::fuzz_target;
use ghostos_vm::{
    decode_migration_checkpoint, migration_checkpoint_tag, snapshot_digest, SnapshotAuthKey,
    SnapshotSchema,
};

const KEY: SnapshotAuthKey = SnapshotAuthKey::new([0x4d; 32]);
const SENDER_NONCE: [u8; 32] = [0x11; 32];
const RECEIVER_NONCE: [u8; 32] = [0x22; 32];

fuzz_target!(|data: &[u8]| {
    let data = &data[..data.len().min(1024 * 1024)];
    let schema = SnapshotSchema::local();
    let _ = decode_migration_checkpoint(data, KEY, schema, &SENDER_NONCE, &RECEIVER_NONCE);

    if data.is_empty() {
        return;
    }
    let issued_at = u64::from(data[0]);
    let checkpoint_id = snapshot_digest(data);
    let length = data.len() as u64;
    let tag = migration_checkpoint_tag(
        KEY,
        schema,
        &SENDER_NONCE,
        &RECEIVER_NONCE,
        issued_at,
        &checkpoint_id,
        length,
        data,
    );
    let mut frame = Vec::with_capacity(48 + data.len() + tag.len());
    frame.extend_from_slice(&issued_at.to_le_bytes());
    frame.extend_from_slice(&checkpoint_id);
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(data);
    frame.extend_from_slice(&tag);
    let _ = decode_migration_checkpoint(&frame, KEY, schema, &SENDER_NONCE, &RECEIVER_NONCE);
});
