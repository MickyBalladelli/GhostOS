use super::{Daemon, DaemonError, Flags, ProcessId, ProcessRights};
use synos_synfs::{FileType, SynFs};

type TestDaemon = Daemon<64, 4, 8, 4, 4, 4096>;

fn daemon() -> (TestDaemon, ProcessId, super::Capability) {
    let mut daemon = TestDaemon::new(SynFs::new()).expect("create filesystem daemon");
    let process = ProcessId::new(7).expect("valid process id");
    let rights = ProcessRights::from_bits(
        ProcessRights::READ.bits()
            | ProcessRights::WRITE.bits()
            | ProcessRights::DELETE.bits()
            | ProcessRights::ADMIN.bits(),
    );
    let authority = daemon
        .register_process(process, rights)
        .expect("register process");
    (daemon, process, authority)
}

fn file_flags() -> Flags {
    Flags::READ.union(Flags::WRITE).union(Flags::DELETE)
}

#[test]
fn link_keeps_data_alive_through_write_delete_and_rename() {
    let (mut daemon, process, authority) = daemon();
    let source = daemon
        .open(
            process,
            authority,
            "/data/source",
            file_flags().union(Flags::CREATE).union(Flags::EXCLUSIVE),
        )
        .expect("open source");
    daemon
        .write(process, source.capability, 0, b"old contents")
        .expect("write source");

    let linked = daemon
        .link(process, source.capability, "/data/alias")
        .expect("create hard link");
    assert_eq!(linked.file_type, FileType::Regular);
    assert_eq!(linked.link_count, 2);

    let alias = daemon
        .open(process, authority, "/data/alias", file_flags())
        .expect("open alias");
    let mut contents = [0; 12];
    assert_eq!(
        daemon.read(process, alias.capability, 0, &mut contents),
        Ok(12)
    );
    assert_eq!(&contents, b"old contents");

    daemon
        .write(process, source.capability, 0, b"new contents")
        .expect("create new source version");
    let mut alias_contents = [0; 12];
    assert_eq!(
        daemon.read(process, alias.capability, 0, &mut alias_contents),
        Ok(12)
    );
    assert_eq!(&alias_contents, b"old contents");
    assert_eq!(
        daemon
            .metadata(process, alias.capability)
            .unwrap()
            .link_count,
        2
    );

    daemon
        .delete(process, source.capability)
        .expect("delete source name");
    assert_eq!(
        daemon
            .metadata(process, alias.capability)
            .unwrap()
            .link_count,
        1
    );

    daemon
        .rename(process, alias.capability, "/data/renamed")
        .expect("rename remaining link");
    let mut links = [0; 128];
    let (_, bytes) = daemon
        .list_links(process, authority, "/data/renamed", &mut links)
        .expect("list remaining link");
    assert_eq!(&links[..bytes], b"/data/renamed\n/data/source\n");
    assert_eq!(
        daemon.filesystem().lookup("/data/source").unwrap().version,
        1
    );
    assert_eq!(
        daemon
            .filesystem()
            .lookup("/data/renamed")
            .unwrap()
            .link_count,
        2
    );
}

#[test]
fn link_requires_read_handle() {
    let (mut daemon, process, authority) = daemon();
    let source = daemon
        .open(
            process,
            authority,
            "/data/source",
            Flags::WRITE.union(Flags::CREATE).union(Flags::EXCLUSIVE),
        )
        .expect("open source");
    assert_eq!(
        daemon.link(process, source.capability, "/data/alias"),
        Err(DaemonError::AccessDenied)
    );
}

#[test]
fn rmdir_requires_empty_directory_and_parent_authority() {
    let (mut daemon, process, authority) = daemon();
    daemon
        .create_directory(process, authority, "/data/empty", false)
        .expect("create empty directory");
    let removed = daemon
        .remove_directory(process, authority, "/data/empty")
        .expect("remove empty directory");
    assert_eq!(removed.file.file_type, FileType::Directory);
    assert!(removed.storage_reclamation_pending);
    assert!(removed.removal_generation > 0);

    daemon
        .create_directory(process, authority, "/data/nonempty", false)
        .expect("create non-empty directory");
    daemon
        .open(
            process,
            authority,
            "/data/nonempty/file",
            file_flags().union(Flags::CREATE).union(Flags::EXCLUSIVE),
        )
        .expect("create child file");
    assert_eq!(
        daemon.remove_directory(process, authority, "/data/nonempty"),
        Err(DaemonError::File(synos_synfs::Error::DirectoryNotEmpty))
    );
}

#[test]
fn rmdir_rejects_mount_roots_and_missing_parent_rights() {
    let (mut daemon, process, authority) = daemon();
    assert_eq!(
        daemon.remove_directory(process, authority, "/data"),
        Err(DaemonError::AccessDenied)
    );

    let limited = daemon
        .register_process(
            ProcessId::new(8).expect("valid process id"),
            ProcessRights::DELETE,
        )
        .expect("register limited process");
    assert_eq!(
        daemon.remove_directory(ProcessId::new(8).unwrap(), limited, "/data/empty"),
        Err(DaemonError::AccessDenied)
    );
}
