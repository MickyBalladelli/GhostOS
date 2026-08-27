// Inventory: coverage_59_5.rs (legacy roadmap section 59).
use ghostos_ghostfs::{
    BlockDevice, BlockIoError, BlockIoQueue, BlockIoResult, BlockOperation, BlockRequest,
    DeviceHealth, Error, PoolLayout, StorageClass, StorageDeviceId,
    StoragePoolError, StoragePoolId, StoragePoolIo, SynFs, VersionSelector, VersionedPath,
    SynfsPurged, VolumeLimits, BLOCK_SIZE, MAX_PATH_BYTES,
};
use ghostos_test_support::crash::{CrashBoundary, CrashDomain, CrashHarness, CrashPoint};

const BLOCKS: usize = 64;
const FORMAT_BLOCKS: usize = 16;

fn corrupt_first_allocated_block<const BLOCKS: usize>(image: &mut [u8], bank: usize) {
    let bank_offset = bank * (BLOCKS + 2) * BLOCK_SIZE;
    for index in 0..BLOCKS {
        let map_byte = image[bank_offset + BLOCK_SIZE + 8 + index / 4];
        let kind = (map_byte >> ((index % 4) * 2)) & 0x03;
        if kind != 0 {
            image[bank_offset + (index + 2) * BLOCK_SIZE] ^= 1;
            return
        }
    }
    panic!("test volume has no allocated blocks")
}

#[test]
fn recovery_uses_newest_complete_generation_and_rejects_partial_objects() {
    let mut image = vec![0; SynFs::<FORMAT_BLOCKS>::volume_bytes()];
    SynFs::<FORMAT_BLOCKS>::format(&mut image).expect("format volume");
    let mut filesystem = SynFs::<FORMAT_BLOCKS>::load(&image).expect("load formatted volume");
    filesystem.write("/state", b"old").expect("write old state");
    filesystem.flush(&mut image).expect("commit old state");
    filesystem.write("/state", b"new").expect("write new state");
    filesystem.flush(&mut image).expect("commit new state");

    corrupt_first_allocated_block::<FORMAT_BLOCKS>(&mut image, 0);
    let recovered = SynFs::<FORMAT_BLOCKS>::recover(&image).expect("recover old complete generation");
    let mut contents = [0; 3];
    recovered.read("/state", &mut contents).expect("read recovered state");
    assert_eq!(&contents, b"old");
    let loaded = SynFs::<FORMAT_BLOCKS>::load(&image).expect("load complete generation");
    loaded.check_consistency().expect("complete generation is consistent");
}

#[test]
fn recovery_reports_corruption_when_no_complete_generation_remains() {
    let mut image = vec![0; SynFs::<FORMAT_BLOCKS>::volume_bytes()];
    SynFs::<FORMAT_BLOCKS>::format(&mut image).expect("format volume");
    let mut filesystem = SynFs::<FORMAT_BLOCKS>::load(&image).expect("load formatted volume");
    filesystem.write("/state", b"old").expect("write old state");
    filesystem.flush(&mut image).expect("commit old state");
    filesystem.write("/state", b"new").expect("write new state");
    filesystem.flush(&mut image).expect("commit new state");

    corrupt_first_allocated_block::<FORMAT_BLOCKS>(&mut image, 0);
    corrupt_first_allocated_block::<FORMAT_BLOCKS>(&mut image, 1);
    assert!(matches!(SynFs::<FORMAT_BLOCKS>::recover(&image), Err(Error::Corrupt)));
}

#[test]
fn interruption_hook_runs_after_flush_and_rename() {
    let mut image = vec![0; SynFs::<FORMAT_BLOCKS>::volume_bytes()];
    let mut filesystem = SynFs::<FORMAT_BLOCKS>::new();
    filesystem.write("/state", b"durable").expect("write state");
    let point = CrashPoint::new(CrashDomain::SynFs, CrashBoundary::Flush, 1);
    let mut harness = CrashHarness::new(Some(point));
    assert_eq!(
        filesystem.flush_with_interruption(&mut image, &mut harness),
        Err(Error::Interrupted)
    );

    let mut filesystem = SynFs::<FORMAT_BLOCKS>::new();
    filesystem.write("/old", b"value").expect("write old path");
    let point = CrashPoint::new(CrashDomain::SynFs, CrashBoundary::Rename, 1);
    let mut harness = CrashHarness::new(Some(point));
    assert_eq!(
        filesystem.rename_with_interruption("/old", "/new", &mut harness),
        Err(Error::Interrupted)
    );
}

#[test]
fn abandoned_rename_transaction_keeps_the_old_root() {
    let mut filesystem = SynFs::<FORMAT_BLOCKS>::new();
    filesystem.write("/old", b"value").expect("write old file");

    {
        let mut transaction = filesystem.transaction();
        transaction
            .rename("/old", "/new")
            .expect("stage rename");
        assert!(transaction.lookup("/new").is_ok());
    }

    assert!(filesystem.lookup("/old").is_ok());
    assert_eq!(filesystem.lookup("/new"), Err(Error::NotFound));
}

#[test]
fn format_generation_selection_and_checksum_validation() {
    let geometry = SynFs::<FORMAT_BLOCKS>::volume_geometry();
    assert_eq!(geometry.block_size, BLOCK_SIZE);
    assert_eq!(geometry.total_bytes, SynFs::<FORMAT_BLOCKS>::volume_bytes());
    assert_eq!(geometry.total_blocks, SynFs::<FORMAT_BLOCKS>::volume_blocks());

    let mut image = vec![0; SynFs::<FORMAT_BLOCKS>::volume_bytes()];
    SynFs::<FORMAT_BLOCKS>::format(&mut image).expect("format volume");
    let mut filesystem = SynFs::<FORMAT_BLOCKS>::load(&image).expect("load formatted volume");
    filesystem.write("/state", b"generation one").expect("write first version");
    let first = filesystem.flush(&mut image).expect("flush first generation");
    filesystem.write("/state", b"generation two").expect("write second version");
    let second = filesystem.flush(&mut image).expect("flush second generation");
    assert!(second.sequence > first.sequence);

    let recovered = SynFs::<FORMAT_BLOCKS>::load(&image).expect("select newest valid generation");
    assert_eq!(recovered.lookup("/state").expect("latest file").version, 2);

    let mut corrupt = image;
    corrupt[BLOCK_SIZE - 1] ^= 1;
    corrupt[(FORMAT_BLOCKS + 2) * BLOCK_SIZE + BLOCK_SIZE - 1] ^= 1;
    assert!(matches!(SynFs::<FORMAT_BLOCKS>::load(&corrupt), Err(Error::Corrupt)));
}

#[test]
fn versions_directories_links_snapshots_and_retention_are_consistent() {
    let mut filesystem = SynFs::<BLOCKS>::new();
    filesystem
        .create_directory("/data/archive", true)
        .expect("create directories");
    filesystem
        .write("/data/archive/log", b"old")
        .expect("write version one");
    let checkpoint = filesystem.create_checkpoint().expect("pin snapshot");
    filesystem
        .write("/data/archive/log", b"new")
        .expect("write version two");
    filesystem
        .link("/data/archive/log", "/data/archive/alias")
        .expect("create hard link");

    let mut old = [0; 3];
    filesystem
        .read_version("/data/archive/log", 1, &mut old)
        .expect("read exact old version");
    assert_eq!(&old, b"old");
    let mut latest = [0; 3];
    filesystem.read("/data/archive/alias", &mut latest).expect("read link");
    assert_eq!(&latest, b"new");
    assert_eq!(filesystem.lookup("/data/archive/alias").unwrap().link_count, 2);

    let snapshot = filesystem
        .checkpoint_snapshot(checkpoint.id, ghostos_ghostfs::RmsMapHandle::from_capability((1 << 32) | 1).unwrap())
        .expect("open checkpoint snapshot");
    let mut snapshot_contents = [0; 3];
    let mut copied = 0;
    snapshot
        .visit_file_pages("/data/archive/log", |page| {
            snapshot_contents[copied..copied + page.bytes.len()].copy_from_slice(page.bytes);
            copied += page.bytes.len();
        })
        .expect("read pinned old root");
    assert_eq!(&snapshot_contents, b"old");

    assert_eq!(filesystem.purge("/data/archive/log", 1, 8).unwrap(), 1);
    assert_eq!(filesystem.lookup("/data/archive/log;1"), Err(Error::NotFound));
    filesystem.release_checkpoint(checkpoint.id).expect("release snapshot");
    let report = filesystem.collect_garbage();
    assert!(report.freed_blocks > 0);
    filesystem.check_consistency().expect("consistent retained tree");
}

#[test]
fn long_run_retention_is_bounded_across_restart_and_releases_checkpoint_blocks() {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(long_run_retention_is_bounded_across_restart_and_releases_checkpoint_blocks_body)
        .expect("spawn large-stack GhostFS retention test")
        .join()
        .expect("large-stack GhostFS retention test panicked");
}

fn long_run_retention_is_bounded_across_restart_and_releases_checkpoint_blocks_body() {
    const CYCLES: usize = 48;
    const CHECKPOINT_INTERVAL: usize = 8;
    const CHECKPOINT_HOLD: usize = 3;
    const KEEP_LATEST: u32 = 2;

    let mut image = vec![0; SynFs::<BLOCKS>::volume_bytes()];
    SynFs::<BLOCKS>::format(&mut image).expect("format volume");
    // SynFs keeps its fixed arena inline. Heap the long-run fixture so the
    // restart path does not stack two half-megabyte filesystem values.
    let mut filesystem = Box::new(SynFs::<BLOCKS>::load(&image).expect("load volume"));
    filesystem
        .create_directory("/data", true)
        .expect("create data directory");

    let mut purged = SynfsPurged::new();
    purged
        .add_rule("/data/long-run", KEEP_LATEST)
        .expect("add retention rule");
    let mut checkpoint = None;

    for cycle in 0..CYCLES {
        let mut contents = [0; 32];
        contents.fill(cycle as u8);
        filesystem
            .write("/data/long-run", &contents)
            .expect("write retained version");

        if cycle % CHECKPOINT_INTERVAL == 0 {
            let info = filesystem.create_checkpoint().expect("create checkpoint");
            checkpoint = Some((info, cycle as u8));
        }

        let report = purged
            .poll(&mut filesystem, 1)
            .expect("bounded retention poll");
        assert!(report.versions_purged <= 1);
        let (retained, _) = filesystem
            .retained_version_span("/data/long-run")
            .expect("retained version span");
        assert!(retained <= KEEP_LATEST);
        let diagnostics = filesystem.diagnostics().expect("retention diagnostics");
        assert!(diagnostics.retained_versions <= (cycle as u64).saturating_add(2));
        assert!(diagnostics.allocated_blocks <= BLOCKS);
        assert!(diagnostics.live_blocks <= diagnostics.allocated_blocks);
        filesystem
            .check_consistency()
            .expect("retention tree stays consistent");

        if cycle % CHECKPOINT_INTERVAL == CHECKPOINT_HOLD {
            let (info, expected) = checkpoint.expect("checkpoint is held");
            filesystem.flush(&mut image).expect("persist checkpoint");
            filesystem = Box::new(
                SynFs::<BLOCKS>::load(&image).expect("restart from checkpoint"),
            );
            assert_eq!(filesystem.checkpoint_info(info.id), Ok(info));

            let snapshot = filesystem
                .checkpoint_snapshot(
                    info.id,
                    ghostos_ghostfs::RmsMapHandle::from_capability((1 << 32) | 1).unwrap(),
                )
                .expect("open persisted checkpoint");
            let mut snapshot_contents = [0; 32];
            let mut copied = 0;
            snapshot
                .visit_file_pages("/data/long-run", |page| {
                    snapshot_contents[copied..copied + page.bytes.len()]
                        .copy_from_slice(page.bytes);
                    copied += page.bytes.len();
                })
                .expect("read persisted checkpoint");
            assert_eq!(snapshot_contents, [expected; 32]);

            filesystem
                .release_checkpoint(info.id)
                .expect("release checkpoint");
            assert_eq!(
                filesystem.checkpoint_info(info.id),
                Err(Error::CheckpointNotFound)
            );
            checkpoint = None;
        }
    }

    let first = filesystem.collect_garbage();
    let diagnostics = filesystem.diagnostics().expect("final diagnostics");
    assert_eq!(diagnostics.allocated_blocks, diagnostics.live_blocks);
    assert!(first.freed_blocks <= BLOCKS);
    let second = filesystem.collect_garbage();
    assert_eq!(second.freed_blocks, 0);
    assert_eq!(filesystem.diagnostics().unwrap().checkpoints, 0);
    filesystem
        .check_consistency()
        .expect("final retained tree is consistent");
}

#[test]
fn exact_and_latest_delete_preserve_snapshots_until_release() {
    let mut filesystem = SynFs::<BLOCKS>::new();
    filesystem
        .create_directory("/data", true)
        .expect("create data directory");
    filesystem.write("/data/item", b"old").expect("write version one");
    let checkpoint = filesystem.create_checkpoint().expect("pin snapshot");
    filesystem.write("/data/item", b"new").expect("write version two");

    let deleted = filesystem
        .delete("/data/item;1")
        .expect("delete exact older version");
    assert_eq!(deleted.version, 1);
    assert_eq!(filesystem.lookup("/data/item").unwrap().version, 2);
    assert_eq!(filesystem.lookup("/data/item;1"), Err(Error::NotFound));

    let snapshot = filesystem
        .checkpoint_snapshot(
            checkpoint.id,
            ghostos_ghostfs::RmsMapHandle::from_capability((1 << 32) | 1).unwrap(),
        )
        .expect("open checkpoint snapshot");
    let mut snapshot_contents = [0; 3];
    let mut copied = 0;
    snapshot
        .visit_file_pages("/data/item", |page| {
            snapshot_contents[copied..copied + page.bytes.len()].copy_from_slice(page.bytes);
            copied += page.bytes.len();
        })
        .expect("snapshot retains exact deleted version");
    assert_eq!(&snapshot_contents, b"old");

    let deleted = filesystem.delete("/data/item").expect("delete latest version");
    assert_eq!(deleted.version, 2);
    assert_eq!(deleted.link_count, 0);
    assert_eq!(filesystem.lookup("/data/item"), Err(Error::NotFound));

    filesystem.release_checkpoint(checkpoint.id).expect("release snapshot");
    assert!(filesystem.collect_garbage().freed_blocks > 0);
    filesystem.check_consistency().expect("consistent deleted tree");
}

#[test]
fn deleting_the_final_hard_link_reclaims_shared_data() {
    let mut filesystem = SynFs::<BLOCKS>::new();
    filesystem
        .create_directory("/data", true)
        .expect("create data directory");
    filesystem.write("/data/source", b"shared").expect("write source");
    filesystem
        .link("/data/source", "/data/alias")
        .expect("create hard link");

    let deleted = filesystem.delete("/data/source").expect("delete first link");
    assert_eq!(deleted.link_count, 1);
    assert_eq!(filesystem.lookup("/data/source"), Err(Error::NotFound));
    assert_eq!(filesystem.lookup("/data/alias").unwrap().link_count, 1);

    let deleted = filesystem.delete("/data/alias").expect("delete final link");
    assert_eq!(deleted.link_count, 0);
    assert_eq!(filesystem.lookup("/data/alias"), Err(Error::NotFound));
    assert!(filesystem.collect_garbage().freed_blocks > 0);
    filesystem.check_consistency().expect("consistent link cleanup");
}

#[test]
fn wildcard_versions_are_sorted_bounded_and_selector_scoped() {
    let mut filesystem = SynFs::<BLOCKS>::new();
    filesystem
        .create_directory("/data", true)
        .expect("create data directory");
    filesystem.write("/data/zeta", b"z1").expect("write zeta v1");
    filesystem.write("/data/alpha", b"a1").expect("write alpha v1");
    filesystem.write("/data/zeta", b"z2").expect("write zeta v2");

    let mut matches = [None; 4];
    assert_eq!(filesystem.expand_paths("/data/*", &mut matches), Ok(2));
    assert_eq!(matches[0].unwrap().as_str(), "/data/alpha");
    assert_eq!(matches[1].unwrap().as_str(), "/data/zeta");

    let mut retained = [None; 4];
    assert_eq!(filesystem.expand_paths("/data/*;1", &mut retained), Ok(2));
    assert_eq!(retained[0].unwrap().as_str(), "/data/alpha");
    assert_eq!(retained[1].unwrap().as_str(), "/data/zeta");

    let mut latest_only = [None; 1];
    assert_eq!(
        filesystem.expand_paths("/data/*", &mut latest_only),
        Err(Error::BufferTooSmall { required: 2 })
    );

    filesystem.delete("/data/zeta;1").expect("delete selected version");
    let mut after_delete = [None; 4];
    assert_eq!(filesystem.expand_paths("/data/*;1", &mut after_delete), Ok(1));
    assert_eq!(after_delete[0].unwrap().as_str(), "/data/alpha");
    assert_eq!(filesystem.lookup("/data/zeta").unwrap().version, 2);
}

#[test]
fn wildcard_pages_resume_from_record_cursors() {
    let mut filesystem = SynFs::<BLOCKS>::new();
    filesystem
        .create_directory("/data", true)
        .expect("create data directory");
    for path in ["/data/alpha", "/data/beta", "/data/gamma"] {
        filesystem.write(path, b"value").expect("create wildcard file");
    }

    let mut first = [None; 1];
    let mut first_cursor = [0; 1];
    let (count, next) = filesystem
        .expand_paths_page_with_cursors("/data/*", 0, &mut first, &mut first_cursor)
        .expect("first wildcard page");
    assert_eq!(count, 1);
    assert_eq!(first[0].unwrap().as_str(), "/data/alpha");
    let next = next.expect("second wildcard page");
    assert_eq!(first_cursor[0], 1);

    let mut second = [None; 1];
    let (count, next) = filesystem
        .expand_paths_page("/data/*", next, &mut second)
        .expect("second wildcard page");
    assert_eq!(count, 1);
    assert_eq!(second[0].unwrap().as_str(), "/data/beta");
    assert!(next.is_some());
}

#[test]
fn property_wildcard_version_selection_never_falls_back_to_latest() {
    use ghostos_test_support::property::{run_assert, Config};

    run_assert("ghostfs.wildcard-version-selection", Config::new(0x59_3, 64), |_, _, entropy| {
        let requested = (entropy.next_u64() % 4 + 1) as u32;
        let mut filesystem = SynFs::<BLOCKS>::new();
        if filesystem.create_directory("/data", true).is_err()
            || filesystem.write("/data/item", b"one").is_err()
            || filesystem.write("/data/item", b"two").is_err()
        {
            return false
        }
        let pattern = format!("/data/*;{requested}");
        let mut matches = [None; 2];
        let count = filesystem.expand_paths(&pattern, &mut matches).ok();
        count == Some(usize::from(requested <= 2))
    })
    .expect("generated version selectors preserve wildcard scope");
}

#[test]
fn path_limits_and_quotas_reject_unsafe_or_excessive_input() {
    // SynFs::new() returns a large struct by value; libtest stacks are too
    // small for debug-build frames at this size.
    let handle = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(path_limits_and_quotas_reject_unsafe_or_excessive_input_body)
        .unwrap();
    handle.join().unwrap();
}

fn path_limits_and_quotas_reject_unsafe_or_excessive_input_body() {
    assert_eq!(VersionedPath::parse("/tmp/file;0").unwrap().version, VersionSelector::Latest);
    assert_eq!(VersionedPath::parse("/tmp/file;7").unwrap().version, VersionSelector::Exact(7));
    assert_eq!(VersionedPath::parse("/tmp/file;wat"), Err(Error::InvalidVersion));
    assert_eq!(VersionedPath::parse("/tmp/file;").unwrap_err(), Error::InvalidVersion);
    assert_eq!(VersionedPath::parse("/tmp/../file").unwrap_err(), Error::InvalidPath);

    let too_long = "/x".repeat(MAX_PATH_BYTES);
    assert_eq!(SynFs::<BLOCKS>::new().lookup(&too_long), Err(Error::InvalidPath));

    let mut filesystem = SynFs::<BLOCKS>::new();
    filesystem
        .set_limits(VolumeLimits {
            max_bytes: 3,
            max_files: 1,
            max_blocks: BLOCKS,
        })
        .expect("set quota");
    filesystem.write("/one", b"123").expect("write within quota");
    assert_eq!(filesystem.write("/two", b"x"), Err(Error::QuotaExceeded));
    assert_eq!(filesystem.write("/one", b"1234"), Err(Error::QuotaExceeded));
}

#[derive(Clone, Debug)]
struct MemoryDevice {
    blocks: Vec<[u8; BLOCK_SIZE]>,
    fail_reads: bool,
    fail_writes: bool,
}

impl MemoryDevice {
    fn new(blocks: usize) -> Self {
        Self {
            blocks: vec![[0; BLOCK_SIZE]; blocks],
            fail_reads: false,
            fail_writes: false,
        }
    }
}

impl BlockDevice for MemoryDevice {
    fn read_block(&mut self, block: u64, output: &mut [u8]) -> Result<(), ()> {
        if self.fail_reads || output.len() != BLOCK_SIZE {
            return Err(())
        }
        output.copy_from_slice(self.blocks.get(block as usize).ok_or(())?);
        Ok(())
    }

    fn write_block(&mut self, block: u64, input: &[u8]) -> Result<(), ()> {
        if self.fail_writes || input.len() != BLOCK_SIZE {
            return Err(())
        }
        self.blocks.get_mut(block as usize).ok_or(())?.copy_from_slice(input);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), ()> {
        Ok(())
    }

    fn discard_block(&mut self, block: u64) -> Result<(), ()> {
        self.blocks.get_mut(block as usize).ok_or(())?.fill(0);
        Ok(())
    }
}

#[test]
fn storage_pool_and_block_queue_cover_failure_and_mirror_rebuild_paths() {
    let first = StorageDeviceId::new(1).unwrap();
    let second = StorageDeviceId::new(2).unwrap();
    let replacement = StorageDeviceId::new(3).unwrap();
    let pool_id = StoragePoolId::new(1).unwrap();
    let mut io = StoragePoolIo::<MemoryDevice, 4, 2>::new();
    io.register_device(first, StorageClass::Nvme, 32, BLOCK_SIZE as u32, 1, MemoryDevice::new(32))
        .expect("register first device");
    io.register_device(second, StorageClass::Nvme, 32, BLOCK_SIZE as u32, 2, MemoryDevice::new(32))
        .expect("register second device");
    io.create_pool(pool_id, "mirror", PoolLayout::Mirror, &[first, second])
        .expect("create mirror");
    let start = io.admin_mut().allocate(pool_id, 2).expect("allocate mirror blocks");
    assert_eq!(start, 0);
    assert_eq!(io.admin().resolve_block(pool_id, 1, 1).unwrap().device, second);

    io.admin_mut().set_device_health(second, DeviceHealth::Failed).unwrap();
    assert_eq!(io.admin().pool_health(pool_id).unwrap(), ghostos_ghostfs::PoolHealth::Degraded);
    assert_eq!(io.admin().resolve_block(pool_id, 0, 1), Err(StoragePoolError::DeviceFailed));
    io.replace_device(
        pool_id,
        second,
        replacement,
        StorageClass::Nvme,
        32,
        BLOCK_SIZE as u32,
        3,
        MemoryDevice::new(32),
    )
    .expect("register replacement");
    let mut scratch = [0; BLOCK_SIZE];
    assert_eq!(io.rebuild_mirror(pool_id, replacement, 0, 2, &mut scratch).unwrap(), 2);

    let mut queue = BlockIoQueue::<1>::new();
    let request = BlockRequest::read(pool_id, 0);
    let token = queue.submit(request).expect("submit block read");
    assert_eq!(queue.submit(request), Err(BlockIoError::QueueFull));
    assert_eq!(queue.cancel(token), Ok(()));
    assert_eq!(queue.cancel(token), Err(BlockIoError::InvalidToken));
    assert_eq!(BlockRequest::write(pool_id, 0, &[0; BLOCK_SIZE + 1]), Err(BlockIoError::InvalidRequest));
}

#[test]
fn storage_queue_reports_device_io_and_completion_results() {
    let device = StorageDeviceId::new(1).unwrap();
    let pool = StoragePoolId::new(1).unwrap();
    let mut io = StoragePoolIo::<MemoryDevice, 2, 1>::new();
    io.register_device(device, StorageClass::Nvme, 4, BLOCK_SIZE as u32, 1, MemoryDevice::new(4))
        .expect("register device");
    io.create_pool(pool, "pool", PoolLayout::Stripe, &[device]).expect("create pool");
    io.admin_mut().allocate(pool, 1).expect("allocate pool block");
    let mut queue = BlockIoQueue::<2>::new();
    queue.submit(BlockRequest::write(pool, 0, &[7; BLOCK_SIZE]).unwrap()).unwrap();
    assert_eq!(queue.dispatch(&mut io), 1);
    let completion = queue.poll().expect("poll write completion");
    assert_eq!(completion.result, Ok(BlockIoResult::Complete { bytes: BLOCK_SIZE as u16 }));

    io.admin_mut().set_device_health(device, DeviceHealth::Failed).unwrap();
    queue.submit(BlockRequest::read(pool, 0)).unwrap();
    queue.dispatch(&mut io);
    assert!(matches!(queue.poll().unwrap().result, Err(BlockIoError::Pool(StoragePoolError::DeviceFailed))));
    assert_eq!(BlockOperation::Read, BlockOperation::Read);
}
