use synos_storaged::{
    Endpoint, IoOperation, MountOptions, MountStateError, Protocol, StorageDaemon, StorageError,
    StoragePath, StorageRights,
};
use synos_synfs::SynFs;

#[test]
fn storage_paths_capabilities_and_io_boundaries_are_enforced() {
    assert_eq!(StoragePath::new("SYS$STORAGE:").unwrap(), StoragePath::ROOT);
    assert!(StoragePath::ROOT.contains(StoragePath::mount_root("DATA").unwrap()));
    assert!(StoragePath::new("SYS$STORAGE:DATA/../escape").is_err());
    assert!(StoragePath::new("SYS$STORAGE:DATA//file").is_err());

    let mut daemon = StorageDaemon::new(0x5a5a);
    let admin = daemon.bootstrap_capability(100).expect("bootstrap admin capability");
    let mount = daemon
        .mount(
            admin,
            Endpoint::new("storage.example", "/volume", 2049).unwrap(),
            Protocol::Pnfs(synos_storaged::NfsMinorVersion::V42),
            StoragePath::mount_root("DATA").unwrap(),
            MountOptions::DEFAULT,
            10,
        )
        .expect("mount storage endpoint");
    let scoped = daemon
        .capability(
            admin,
            mount.id,
            StorageRights::READ,
            StoragePath::new("SYS$STORAGE:DATA/files").unwrap(),
            90,
            10,
        )
        .expect("attenuate storage capability");
    assert_eq!(
        daemon.submit_io(
            scoped,
            IoOperation::Read,
            StoragePath::new("SYS$STORAGE:DATA/files/a").unwrap(),
            4,
            10,
        )
        .unwrap()
        .length,
        4
    );
    assert!(matches!(
        daemon.submit_io(
            scoped,
            IoOperation::Write,
            StoragePath::new("SYS$STORAGE:DATA/files/a").unwrap(),
            4,
            10,
        ),
        Err(StorageError::Capability(_))
    ));
    let completion = daemon.complete_next(Ok(()), 99).expect("complete pending IO");
    assert_eq!(completion.bytes, 4);
    assert_eq!(daemon.poll_completion(), Some(completion));
    assert!(matches!(daemon.bootstrap_capability(0), Err(_)));
}

#[test]
fn mount_catalog_round_trips_through_synfs_and_rejects_corruption() {
    let mut daemon = StorageDaemon::new(7);
    let admin = daemon.bootstrap_capability(100).unwrap();
    let mounted = daemon
        .mount(
            admin,
            Endpoint::new("host", "/export", 445).unwrap(),
            Protocol::Smb {
                dialect: synos_storaged::SmbDialect::Smb311,
                multichannel: true,
                direct: true,
            },
            StoragePath::mount_root("REMOTE").unwrap(),
            MountOptions::DEFAULT,
            1,
        )
        .unwrap();

    let catalog = daemon.mount_catalog();
    let mut encoded = vec![0; catalog.encoded_len()];
    let length = catalog.encode(&mut encoded).unwrap();
    assert_eq!(synos_storaged::MountCatalog::decode(&encoded).unwrap().version(), catalog.version());
    assert!(matches!(
        synos_storaged::MountCatalog::decode(&encoded[..length - 1]),
        Err(MountStateError::Corrupt)
    ));
    encoded[0] ^= 1;
    assert!(matches!(
        synos_storaged::MountCatalog::decode(&encoded),
        Err(MountStateError::Corrupt)
    ));

    let mut filesystem = SynFs::<128>::new();
    let mut staging = vec![0; catalog.encoded_len()];
    catalog.save_to_synfs(&mut filesystem, &mut staging).expect("persist mount catalog");
    let mut restored_bytes = vec![0; catalog.encoded_len()];
    filesystem.read("/system/mounts.dat", &mut restored_bytes).expect("read mount catalog");
    let restored = synos_storaged::MountCatalog::decode(&restored_bytes).unwrap();
    let mut restored_daemon = StorageDaemon::new(7);
    restored_daemon.restore_catalog(restored).expect("restore mount catalog");
    let mut mount_infos = [mounted; 1];
    assert_eq!(restored_daemon.list_mounts(&mut mount_infos), 1);
}
