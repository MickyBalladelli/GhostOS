use ghostos_ghostfs::{
    BackgroundIoLimit, Error, FormatMigrationPhase, SynFs, VOLUME_FORMAT_VERSION, BLOCK_SIZE,
};

const BLOCKS: usize = 8;
const OLD_FORMAT: u16 = 3;

fn checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3)
    }
    hash
}

fn mark_bank_old(image: &mut [u8], bank: usize) {
    let start = bank * (BLOCKS + 2) * BLOCK_SIZE;
    image[start + 8..start + 10].copy_from_slice(&OLD_FORMAT.to_le_bytes());
    let header_checksum = checksum(&image[start..start + BLOCK_SIZE - 8]);
    image[start + BLOCK_SIZE - 8..start + BLOCK_SIZE]
        .copy_from_slice(&header_checksum.to_le_bytes());
}

#[test]
fn migration_copies_in_bounded_steps_and_commits_one_complete_format() {
    let mut image = vec![0; SynFs::<BLOCKS>::volume_bytes()];
    SynFs::<BLOCKS>::format(&mut image).unwrap();
    mark_bank_old(&mut image, 0);
    let mut filesystem = SynFs::<BLOCKS>::load(&image).unwrap();
    filesystem.write("/state", b"durable").unwrap();
    filesystem.flush(&mut image).unwrap();

    let mut migration = SynFs::<BLOCKS>::begin_format_migration(
        &mut image,
        VOLUME_FORMAT_VERSION,
    )
    .unwrap();
    while migration.progress().phase != FormatMigrationPhase::Committed {
        let progress = migration.step(&mut image, BackgroundIoLimit::new(1)).unwrap();
        assert!(progress.io_blocks <= 1);
    }

    let recovered = SynFs::<BLOCKS>::load(&image).unwrap();
    assert_eq!(recovered.format_version(), VOLUME_FORMAT_VERSION);
    let mut contents = [0; 7];
    recovered.read("/state", &mut contents).unwrap();
    assert_eq!(&contents, b"durable");
}

#[test]
fn migration_refuses_downgrade_and_can_roll_back_after_commit() {
    let mut image = vec![0; SynFs::<BLOCKS>::volume_bytes()];
    SynFs::<BLOCKS>::format(&mut image).unwrap();
    assert!(matches!(
        SynFs::<BLOCKS>::begin_format_migration(&mut image, OLD_FORMAT),
        Err(Error::DowngradeRefused)
    ));

    mark_bank_old(&mut image, 0);
    let mut migration = SynFs::<BLOCKS>::begin_format_migration(
        &mut image,
        VOLUME_FORMAT_VERSION,
    )
    .unwrap();
    while migration.progress().phase != FormatMigrationPhase::Committed {
        migration.step(&mut image, BackgroundIoLimit::new(BLOCKS)).unwrap();
    }
    migration.rollback(&mut image).unwrap();
    assert_eq!(SynFs::<BLOCKS>::load(&image).unwrap().format_version(), OLD_FORMAT);
}
