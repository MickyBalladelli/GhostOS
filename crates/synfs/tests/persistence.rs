use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use synos_synfs::{BlockIoError, BlockStore, Error, StorageDeviceId, SynFs, BLOCK_SIZE};

const MAX_BLOCKS: usize = 64;

struct DiskImage {
    file: File,
    fail_after_writes: Option<usize>,
    writes: usize,
}

impl DiskImage {
    fn create(path: &Path) -> Self {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(path)
            .expect("create disk image");
        file.set_len(SynFs::<MAX_BLOCKS>::volume_bytes() as u64)
            .expect("size disk image");
        Self {
            file,
            fail_after_writes: None,
            writes: 0,
        }
    }

    fn open(path: &Path, fail_after_writes: Option<usize>) -> Self {
        Self {
            file: OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .expect("open disk image"),
            fail_after_writes,
            writes: 0,
        }
    }

    fn seek_block(&mut self, block: u64) {
        self.file
            .seek(SeekFrom::Start(block * BLOCK_SIZE as u64))
            .expect("seek disk image block");
    }

    fn device_error() -> BlockIoError {
        BlockIoError::DeviceIo {
            device: StorageDeviceId::new(1).expect("valid test device"),
        }
    }
}

impl BlockStore for DiskImage {
    fn read_block(&mut self, block: u64, output: &mut [u8]) -> Result<(), BlockIoError> {
        if output.len() != BLOCK_SIZE {
            return Err(BlockIoError::InvalidBlockSize);
        }
        self.seek_block(block);
        self.file.read_exact(output).map_err(|_| Self::device_error())
    }

    fn write_block(&mut self, block: u64, input: &[u8]) -> Result<(), BlockIoError> {
        if input.len() != BLOCK_SIZE {
            return Err(BlockIoError::InvalidBlockSize);
        }
        self.seek_block(block);
        if self
            .fail_after_writes
            .is_some_and(|limit| self.writes >= limit)
        {
            self.file
                .write_all(&input[..BLOCK_SIZE / 2])
                .map_err(|_| Self::device_error())?;
            self.writes += 1;
            return Err(Self::device_error());
        }
        self.file.write_all(input).map_err(|_| Self::device_error())?;
        self.writes += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockIoError> {
        self.file.sync_all().map_err(|_| Self::device_error())
    }

    fn discard_block(&mut self, block: u64) -> Result<(), BlockIoError> {
        self.seek_block(block);
        self.file
            .write_all(&[0; BLOCK_SIZE])
            .map_err(|_| Self::device_error())
    }
}

struct TemporaryImage(PathBuf);

impl TemporaryImage {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!("synos-{label}-{}-{stamp}.img", std::process::id())))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryImage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn write_durable_state(path: &Path) {
    let mut disk = DiskImage::create(path);
    let mut image = vec![0; SynFs::<MAX_BLOCKS>::volume_bytes()];
    SynFs::<MAX_BLOCKS>::format_to_device(&mut image, &mut disk).expect("format disk image");
    let mut filesystem =
        SynFs::<MAX_BLOCKS>::load_from_device(&mut image, &mut disk).expect("load formatted image");
    filesystem
        .create_directory("/data", true)
        .expect("create data directory");
    filesystem
        .write("/data/state", b"durable state")
        .expect("write durable state");
    filesystem
        .flush_to_device(&mut disk)
        .expect("commit durable state");
}

fn read_state(path: &Path) -> SynFs<MAX_BLOCKS> {
    let mut disk = DiskImage::open(path, None);
    let mut image = vec![0; SynFs::<MAX_BLOCKS>::volume_bytes()];
    SynFs::<MAX_BLOCKS>::load_from_device(&mut image, &mut disk).expect("recover disk image")
}

#[test]
fn persists_to_real_disk_image() {
    let image_path = TemporaryImage::new("persistence");
    write_durable_state(image_path.path());

    let filesystem = read_state(image_path.path());
    let mut contents = [0; 13];
    let result = filesystem
        .read("/data/state", &mut contents)
        .expect("read state after reopen");
    assert_eq!(result.bytes_read, 13);
    assert_eq!(&contents, b"durable state");
    filesystem.check_consistency().expect("consistent reopened image");
}

#[test]
fn persists_filesystem_shell_workflow_objects_and_relative_target() {
    let image_path = TemporaryImage::new("shell-workflow");
    let mut disk = DiskImage::create(image_path.path());
    let mut image = vec![0; SynFs::<MAX_BLOCKS>::volume_bytes()];
    SynFs::<MAX_BLOCKS>::format_to_device(&mut image, &mut disk).expect("format shell image");
    let mut filesystem =
        SynFs::<MAX_BLOCKS>::load_from_device(&mut image, &mut disk).expect("load shell image");

    filesystem
        .create_directory("/data/work", true)
        .expect("create shell default directory");
    filesystem
        .write("/data/work/note", b"shell contents")
        .expect("create shell workflow file");
    filesystem
        .flush_to_device(&mut disk)
        .expect("persist shell workflow");
    drop(disk);

    let filesystem = read_state(image_path.path());
    assert_eq!(
        filesystem.lookup("/data/work").expect("recovered default directory").file_type,
        synos_synfs::FileType::Directory
    );
    let mut contents = [0; 14];
    let read = filesystem
        .read("/data/work/note", &mut contents)
        .expect("read recovered relative target");
    assert_eq!(read.bytes_read, contents.len());
    assert_eq!(&contents, b"shell contents");
    filesystem.check_consistency().expect("consistent shell workflow image");
}

#[test]
fn persists_link_lifecycle_and_shared_data() {
    let image_path = TemporaryImage::new("link-lifecycle");
    let mut disk = DiskImage::create(image_path.path());
    let mut image = vec![0; SynFs::<MAX_BLOCKS>::volume_bytes()];
    SynFs::<MAX_BLOCKS>::format_to_device(&mut image, &mut disk).expect("format disk image");
    let mut filesystem =
        SynFs::<MAX_BLOCKS>::load_from_device(&mut image, &mut disk).expect("load image");
    filesystem
        .create_directory("/data", true)
        .expect("create data directory");
    filesystem
        .write("/data/source", b"old contents")
        .expect("write source");
    assert_eq!(
        filesystem.link("/data/source", "/data/alias").unwrap().link_count,
        2
    );

    filesystem
        .write("/data/source", b"new contents")
        .expect("create source version");
    let mut alias_contents = [0; 12];
    filesystem
        .read("/data/alias", &mut alias_contents)
        .expect("read alias after source version");
    assert_eq!(&alias_contents, b"old contents");
    filesystem.delete("/data/source").expect("delete source name");
    filesystem
        .rename("/data/alias", "/data/renamed")
        .expect("rename remaining link");
    filesystem
        .flush_to_device(&mut disk)
        .expect("persist link lifecycle");
    drop(disk);

    let filesystem = read_state(image_path.path());
    assert_eq!(filesystem.lookup("/data/source").unwrap().version, 1);
    let mut contents = [0; 12];
    filesystem
        .read("/data/renamed", &mut contents)
        .expect("read recovered link");
    assert_eq!(&contents, b"old contents");
    let mut links = [synos_synfs::LinkEntry::EMPTY; 2];
    assert_eq!(filesystem.list_links("/data/renamed", &mut links), Ok(2));
    assert_eq!(links[0].path.as_str(), "/data/renamed");
    assert_eq!(links[1].path.as_str(), "/data/source");
    assert_eq!(filesystem.lookup("/data/renamed").unwrap().link_count, 2);
    filesystem.check_consistency().expect("consistent linked image");
}

#[test]
fn recovers_previous_generation_after_torn_commit() {
    let baseline_path = TemporaryImage::new("baseline");
    let trial_path = TemporaryImage::new("power-loss");
    write_durable_state(baseline_path.path());

    for fail_after in 0..=MAX_BLOCKS + 1 {
        std::fs::copy(baseline_path.path(), trial_path.path()).expect("copy baseline image");
        let mut disk = DiskImage::open(trial_path.path(), Some(fail_after));
        let mut image = vec![0; SynFs::<MAX_BLOCKS>::volume_bytes()];
        let mut filesystem =
            SynFs::<MAX_BLOCKS>::load_from_device(&mut image, &mut disk).expect("load baseline");
        filesystem
            .write("/data/state", b"newer state")
            .expect("stage newer state");
        assert!(filesystem.flush_to_device(&mut disk).is_err());
        drop(disk);

        let mut disk = DiskImage::open(trial_path.path(), None);
        let mut image = vec![0; SynFs::<MAX_BLOCKS>::volume_bytes()];
        let recovered = SynFs::<MAX_BLOCKS>::load_from_device(&mut image, &mut disk)
            .or_else(|_| SynFs::<MAX_BLOCKS>::recover(&image))
            .expect("recover prior generation");
        let mut contents = [0; 13];
        recovered
            .read("/data/state", &mut contents)
            .expect("read recovered state");
        assert_eq!(&contents, b"durable state");
        recovered.check_consistency().expect("consistent recovery");
    }
}

#[test]
fn rejects_short_disk_images() {
    let mut image = vec![0; SynFs::<MAX_BLOCKS>::volume_bytes() - 1];
    assert!(matches!(
        SynFs::<MAX_BLOCKS>::format(&mut image),
        Err(Error::BufferTooSmall { .. })
    ));
}
