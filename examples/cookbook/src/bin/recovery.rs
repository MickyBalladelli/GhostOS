use synos_backup::{
    BackupCatalog, BackupId, BackupManifest, Digest, EncryptionKey, ManifestFile, RetentionPolicy,
};
use synos_synfs::{FileName, FileType, FileVersion};

fn main() {
    let id = BackupId::from_raw(7).expect("nonzero backup id");
    let file = FileVersion {
        file: FileName::new("system/kernel").expect("valid backup path"),
        version: 1,
        size: 0,
        checksum: 0,
        created_at: 1,
        file_type: FileType::Regular,
        link_count: 1,
        mode: 0o644,
    };
    let manifest = BackupManifest::<2, 2> {
        id,
        generation: 10,
        base: None,
        file_count: 1,
        chunk_count: 0,
        complete: true,
        files: [
            Some(ManifestFile {
                file,
                first_chunk: 0,
                chunk_count: 0,
            }),
            None,
        ],
        chunks: [None, None],
    };

    let mut catalog = BackupCatalog::<4, 2, 2>::new(RetentionPolicy::new(2, 5));
    catalog.record(manifest).expect("complete manifest");
    catalog.set_legal_hold(id, true).expect("backup exists");

    let key = EncryptionKey::new([3; 32]);
    let digest = Digest::hash(b"recovery payload");
    let mut encrypted = [0; 4096 + 44];
    let length = key
        .encrypt(digest, b"recovery payload", &mut encrypted)
        .expect("payload fits");
    let mut restored = [0; 64];
    let copied = key
        .decrypt(key.object_id(digest), &encrypted[..length], digest, &mut restored)
        .expect("authenticated restore");
    assert_eq!(&restored[..copied], b"recovery payload");
    println!("backup {} is held and payload restored", id.raw());
}
