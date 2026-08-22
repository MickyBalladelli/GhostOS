use super::{
    Daemon, DaemonError, Flags, Operation, ProcessId, ProcessRights, Request,
    MAX_IPC_BUFFER_BYTES,
};
use alloc::{boxed::Box, vec};
use ghostos_ghostfs::{FileType, SynFs};
use ghostos_status::Status;

type TestDaemon = Daemon<16, 4, 8, 4, 8, 4096>;

fn daemon() -> (TestDaemon, ProcessId, super::Capability) {
    daemon_with_capacity::<16>()
}

fn daemon_with_capacity<const BLOCKS: usize>(
) -> (Daemon<BLOCKS, 4, 8, 4, 8, 4096>, ProcessId, super::Capability) {
    let mut daemon = Daemon::<BLOCKS, 4, 8, 4, 8, 4096>::new(SynFs::new())
        .expect("create filesystem daemon");
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

fn boxed_daemon_with_capacity<const BLOCKS: usize>(
) -> (Box<Daemon<BLOCKS, 4, 8, 4, 8, 4096>>, ProcessId, super::Capability) {
    let mut daemon = Box::new(
        Daemon::<BLOCKS, 4, 8, 4, 8, 4096>::new(SynFs::new())
            .expect("create filesystem daemon"),
    );
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
fn whole_file_lock_blocks_other_process_io_until_unlock() {
    let (mut daemon, process, authority) = daemon();
    let file = daemon
        .open(
            process,
            authority,
            "/data/locked",
            file_flags().union(Flags::CREATE).union(Flags::EXCLUSIVE),
        )
        .expect("open locked file");
    daemon
        .write(process, file.capability, 0, b"locked")
        .expect("write locked file");

    let other = ProcessId::new(8).expect("valid process id");
    let other_authority = daemon
        .register_process(
            other,
            ProcessRights::from_bits(ProcessRights::READ.bits() | ProcessRights::WRITE.bits()),
        )
        .expect("register other process");
    let other_file = daemon
        .open(other, other_authority, "/data/locked", Flags::READ)
        .expect("open from other process");
    let lock = daemon
        .lock(
            process,
            file.capability,
            super::LockRange::WholeFile,
            super::LockMode::Exclusive,
        )
        .expect("lock file");
    let mut output = [0; 1];
    assert_eq!(
        daemon.read(other, other_file.capability, 0, &mut output),
        Err(DaemonError::LockBusy)
    );
    daemon.unlock(process, lock.capability).expect("unlock file");
    assert_eq!(daemon.read(other, other_file.capability, 0, &mut output), Ok(1));
}

#[test]
fn record_lock_only_blocks_the_same_record_position() {
    let (mut daemon, process, authority) = daemon();
    let file = daemon
        .open(
            process,
            authority,
            "/data/records",
            file_flags().union(Flags::CREATE).union(Flags::EXCLUSIVE),
        )
        .expect("open record file");
    daemon
        .write(process, file.capability, 0, b"records")
        .expect("write record file");
    let other = ProcessId::new(8).expect("valid process id");
    let other_authority = daemon
        .register_process(other, ProcessRights::READ)
        .expect("register reader");
    let other_file = daemon
        .open(other, other_authority, "/data/records", Flags::READ)
        .expect("open record file for reader");
    let lock = daemon
        .lock(
            process,
            file.capability,
            super::LockRange::Record(4),
            super::LockMode::Exclusive,
        )
        .expect("lock record");
    let mut output = [0; 1];
    assert_eq!(daemon.read(other, other_file.capability, 0, &mut output), Ok(1));
    assert_eq!(
        daemon.read(other, other_file.capability, 4, &mut output),
        Err(DaemonError::LockBusy)
    );
    daemon.unlock(process, lock.capability).expect("unlock record");
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
fn wildcard_delete_and_links_preserve_exact_version_selection() {
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
        .write(process, source.capability, 0, b"version one")
        .expect("write source v1");
    let linked = daemon
        .link(process, source.capability, "/data/alias")
        .expect("link selected source version");
    assert_eq!(linked.version, 1);
    daemon
        .write(process, source.capability, 0, b"version two")
        .expect("write source v2");

    let mut links = [0; 256];
    let (_, bytes) = daemon
        .list_links(process, authority, "/data/source*;1", &mut links)
        .expect("list links for retained version");
    assert_eq!(&links[..bytes], b"/data/alias\n/data/source\n");

    let deleted = daemon
        .delete_path(process, authority, "/data/source*;1")
        .expect("delete only selected retained version");
    assert_eq!(deleted.file.version, 1);
    assert_eq!(daemon.filesystem().lookup("/data/source").unwrap().version, 2);
    assert_eq!(
        daemon.delete_path(process, authority, "/data/source*;1"),
        Err(DaemonError::NotFound)
    );
    assert_eq!(
        daemon.delete_path(process, authority, "/data/source*;wat"),
        Err(DaemonError::InvalidPath)
    );
}

#[test]
fn wildcard_operations_require_authority_and_bound_shared_buffers() {
    let (mut daemon, process, authority) = daemon();
    daemon
        .open(
            process,
            authority,
            "/data/first",
            file_flags().union(Flags::CREATE).union(Flags::EXCLUSIVE),
        )
        .expect("create first file");
    daemon
        .open(
            process,
            authority,
            "/data/second",
            file_flags().union(Flags::CREATE).union(Flags::EXCLUSIVE),
        )
        .expect("create second file");

    let readonly_process = ProcessId::new(8).expect("valid read-only process");
    let readonly = daemon
        .register_process(readonly_process, ProcessRights::READ)
        .expect("register read-only process");
    assert_eq!(
        daemon.delete_path(readonly_process, readonly, "/data/*"),
        Err(DaemonError::AccessDenied)
    );

    let mut output = [0; 1];
    assert!(matches!(
        daemon.list_links(process, authority, "/data/*", &mut output),
        Err(DaemonError::BufferTooSmall { .. })
    ));
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
        Err(DaemonError::File(ghostos_ghostfs::Error::DirectoryNotEmpty))
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

#[test]
fn dispatch_covers_shell_workflow_capabilities_buffers_pagination_and_statuses() {
    let (mut daemon, process, authority) = daemon();
    for path in ["/data/first", "/data/second"] {
        let file = daemon
            .open(
                process,
                authority,
                path,
                file_flags().union(Flags::CREATE).union(Flags::EXCLUSIVE),
            )
            .expect("create workflow file");
        daemon.close(process, file.capability).expect("close workflow file");
    }

    let mut listing = [0; 28];
    listing[..5].copy_from_slice(b"/data");
    let response = daemon.dispatch(
        Request::new(Operation::List, process)
            .with_capability(authority)
            .with_offset(0),
        Some(&mut listing),
    );
    assert_eq!(response.status, Status::NORMAL);
    assert!(response.values[0] > 0);
    assert_eq!(response.values[1], 1);
    let first_name_length = u16::from_le_bytes([listing[0], listing[1]]) as usize;
    assert_eq!(&listing[22..22 + first_name_length], b"first");

    listing.fill(0);
    listing[..5].copy_from_slice(b"/data");
    let response = daemon.dispatch(
        Request::new(Operation::List, process)
            .with_capability(authority)
            .with_offset(1),
        Some(&mut listing),
    );
    assert_eq!(response.status, Status::NORMAL);
    assert_eq!(response.values[1], 0);
    let second_name_length = u16::from_le_bytes([listing[0], listing[1]]) as usize;
    assert_eq!(&listing[22..22 + second_name_length], b"second");

    let limited = daemon
        .register_process(ProcessId::new(8).unwrap(), ProcessRights::WRITE)
        .expect("register limited process");
    let mut denied_listing = [0; 64];
    denied_listing[..5].copy_from_slice(b"/data");
    let response = daemon.dispatch(
        Request::new(Operation::List, ProcessId::new(8).unwrap()).with_capability(limited),
        Some(&mut denied_listing),
    );
    assert_eq!(response.status, Status::ACCESS_DENIED);

    let mut oversized = vec![0; MAX_IPC_BUFFER_BYTES + 1];
    let response = daemon.dispatch(
        Request::new(Operation::List, process).with_capability(authority),
        Some(&mut oversized),
    );
    assert_ne!(response.status, Status::NORMAL);

    let mut invalid_path = *b"/data/../bad";
    let response = daemon.dispatch(
        Request::new(Operation::Mkdir, process)
            .with_capability(authority)
            .with_flags(Flags::RECURSIVE),
        Some(&mut invalid_path),
    );
    assert_eq!(response.status, Status::INVALID_PATH);

    let response = daemon.dispatch(
        Request::new(Operation::List, process).with_capability(authority),
        None,
    );
    assert_eq!(response.status, Status::INVALID_ARGUMENT);

    let mut delete_path = *b"/data/first";
    let response = daemon.dispatch(
        Request::new(Operation::Delete, process).with_capability(authority),
        Some(&mut delete_path),
    );
    assert_eq!(response.status, Status::NORMAL);
    assert_eq!(response.values[0], 1);
    assert_eq!(response.values[1], FileType::Regular as u64);
    assert_eq!(response.values[2], 0);
    assert_eq!(response.values[3], 0);
    assert_eq!(daemon.filesystem().lookup("/data/first"), Err(ghostos_ghostfs::Error::NotFound));

    let response = daemon.dispatch(
        Request::new(Operation::Delete, process).with_capability(authority),
        Some(&mut delete_path),
    );
    assert_eq!(response.status, Status::NOT_FOUND);
}

#[test]
fn long_run_gc_bounds_work_and_releases_orphaned_capabilities() {
    let (mut daemon, process, authority) = boxed_daemon_with_capacity::<64>();
    let mut stale_file = None;

    for cycle in 0..32 {
        let file = if cycle == 0 {
            daemon
                .open(
                    process,
                    authority,
                    "/data/long-run",
                    file_flags().union(Flags::CREATE).union(Flags::EXCLUSIVE),
                )
                .expect("open retained file")
        } else {
            daemon
                .open(process, authority, "/data/long-run", file_flags())
                .expect("reopen retained file")
        };
        daemon
            .write(process, file.capability, 0, &[cycle as u8; 32])
            .expect("write retained version");
        daemon
            .close(process, file.capability)
            .expect("close retained file");
        stale_file = Some(file.capability);

        let report = daemon
            .garbage_collect(process, authority, 1)
            .expect("collect after write");
        assert!(report.freed_blocks <= 1);

        daemon
            .filesystem_mut()
            .purge("/data/long-run", 1, 1)
            .expect("bounded purge");

        if cycle % 4 == 0 {
            let snapshot = daemon
                .snapshot_create(process, authority)
                .expect("create snapshot capability");
            let report = daemon
                .garbage_collect(process, authority, 1)
                .expect("bounded garbage collection");
            assert!(report.freed_blocks <= 1);
            daemon
                .snapshot_release(process, snapshot.capability)
                .expect("release snapshot capability");
            assert_eq!(
                daemon.snapshot_release(process, snapshot.capability),
                Err(DaemonError::AccessDenied)
            );
        } else {
            let report = daemon
                .garbage_collect(process, authority, 1)
                .expect("collect after purge");
            assert!(report.freed_blocks <= 1);
        }

        let diagnostics = daemon.diagnostics().expect("daemon diagnostics");
        assert!(diagnostics.allocated_blocks <= diagnostics.capacity_blocks);
        assert!(diagnostics.live_blocks <= diagnostics.allocated_blocks);
        assert!(diagnostics.checkpoints <= 1);
    }

    let report = daemon
        .garbage_collect(process, authority, usize::MAX)
        .expect("final garbage collection");
    assert!(report.freed_blocks <= daemon.diagnostics().unwrap().capacity_blocks);
    let diagnostics = daemon.diagnostics().expect("pre-unregister diagnostics");
    assert_eq!(diagnostics.allocated_blocks, diagnostics.live_blocks);

    let stale_file = stale_file.expect("stale file capability");
    assert_eq!(daemon.close(process, stale_file), Err(DaemonError::AccessDenied));
    daemon
        .unregister_process(process)
        .expect("unregister process and release handles");
    assert_eq!(daemon.diagnostics().unwrap().checkpoints, 0);
    assert_eq!(
        daemon.garbage_collect(process, authority, usize::MAX),
        Err(DaemonError::ProcessNotRegistered)
    );

    let _read_only_authority = daemon
        .register_process(process, ProcessRights::from_bits(ProcessRights::READ.bits()))
        .expect("register process after cleanup");
    assert_eq!(
        daemon.read(process, stale_file, 0, &mut [0; 32]),
        Err(DaemonError::AccessDenied)
    );
    let diagnostics = daemon.diagnostics().expect("post-restart diagnostics");
    assert_eq!(diagnostics.allocated_blocks, diagnostics.live_blocks);
}
