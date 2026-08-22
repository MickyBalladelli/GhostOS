//! TEMPORARY diagnostic probe: replays the kernel first-admin sequence
//! (fresh volume, no service packages) to find why CONFIRM fails with
//! CORRUPT at the username read-back. Delete this file after diagnosis.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use ghostos_ghostfs::{
    BlockDevice, BlockIoError, BlockStore, MountedSystemVolume, ServiceManifest,
    StorageDeviceId, SynFs, BLOCK_SIZE, SYSTEM_DISK_MANIFEST_BYTES, SYSTEM_VOLUME_BLOCKS,
};

struct RealDisk {
    file: File,
}

impl RealDisk {
    fn open_read_only(path: &Path) -> Self {
        let file = File::open(path).expect("open real system.raw");
        Self { file }
    }
}

impl BlockDevice for RealDisk {
    fn read_block(&mut self, lba: u64, output: &mut [u8]) -> Result<(), ()> {
        self.file
            .seek(SeekFrom::Start(lba * 512))
            .map_err(|_| ())?;
        self.file.read_exact(output).map_err(|_| ())
    }

    fn write_block(&mut self, _lba: u64, _input: &[u8]) -> Result<(), ()> {
        Err(())
    }

    fn flush(&mut self) -> Result<(), ()> {
        Err(())
    }

    fn discard_block(&mut self, _lba: u64) -> Result<(), ()> {
        Err(())
    }
}

#[test]
fn inspect_real_system_disk() {
    // MountedSystemVolume::mount consumes multi-megabyte stack frames in
    // debug builds; libtest threads are too small for it.
    let handle = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(inspect_real_system_disk_body)
        .expect("spawn big-stack probe thread");
    handle.join().expect("real disk probe panicked");
}

fn inspect_real_system_disk_body() {
    let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../virtual_machine/state/system.raw"));
    let mut device = RealDisk::open_read_only(path);
    let mut manifest_scratch = [0u8; SYSTEM_DISK_MANIFEST_BYTES];
    let mut volume_image = vec![0u8; SynFs::<SYSTEM_VOLUME_BLOCKS>::volume_bytes()];
    let mounted =
        MountedSystemVolume::mount(&mut device, &mut manifest_scratch, &mut volume_image);
    let mounted = match mounted {
        Ok(mounted) => mounted,
        Err(error) => {
            println!("MOUNT FAILED: {error:?}");
            return;
        }
    };
    println!(
        "manifest: generation={} disk_size={} volume_offset={} volume_size={}",
        mounted.manifest.generation,
        mounted.manifest.disk_size,
        mounted.manifest.system_volume_offset,
        mounted.manifest.system_volume_size
    );
    report("real disk", &mounted.filesystem);

    let mut filesystem = mounted.filesystem;

    for path in [
        "/system",
        "/system/security",
        "/system/security/first-admin-username",
        "/system/security/first-admin-credential",
        "/system/security/authorization",
    ] {
        match filesystem.lookup(path) {
            Ok(metadata) => {
                println!("[{path}] type={:?} size={}", metadata.file_type, metadata.size);
                if metadata.file_type == ghostos_ghostfs::FileType::Regular && metadata.size > 0 && metadata.size <= 128 {
                    let mut buffer = [0u8; 128];
                    match filesystem.read(path, &mut buffer[..metadata.size as usize]) {
                        Ok(read) => println!("[{path}] bytes={:02x?}", &buffer[..read.bytes_read]),
                        Err(error) => println!("[{path}] READ FAILED: {error:?}"),
                    }
                } else if metadata.file_type == ghostos_ghostfs::FileType::Regular {
                    println!("[{path}] too large to dump");
                }
            }
            Err(error) => println!("[{path}] lookup: {error:?}"),
        }
    }

    run_first_admin_sequence(&mut filesystem, "real-disk");
}

const MAX_BLOCKS: usize = 32;

const DATABASE: &str = "/system/security/authorization";
const USERNAME_PATH: &str = "/system/security/first-admin-username";
const CREDENTIAL_PATH: &str = "/system/security/first-admin-credential";

struct DiskImage {
    file: File,
}

impl DiskImage {
    fn create(path: &Path) -> Self {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(path)
            .expect("create probe disk image");
        file.set_len(SynFs::<MAX_BLOCKS>::volume_bytes() as u64)
            .expect("size probe disk image");
        Self { file }
    }

    fn seek_block(&mut self, block: u64) {
        self.file
            .seek(SeekFrom::Start(block * BLOCK_SIZE as u64))
            .expect("seek probe block");
    }
}

impl BlockStore for DiskImage {
    fn read_block(&mut self, block: u64, output: &mut [u8]) -> Result<(), BlockIoError> {
        if output.len() != BLOCK_SIZE {
            return Err(BlockIoError::InvalidBlockSize);
        }
        self.seek_block(block);
        self.file
            .read_exact(output)
            .map_err(|_| BlockIoError::DeviceIo {
                device: StorageDeviceId::new(1).unwrap(),
            })
    }

    fn write_block(&mut self, block: u64, input: &[u8]) -> Result<(), BlockIoError> {
        if input.len() != BLOCK_SIZE {
            return Err(BlockIoError::InvalidBlockSize);
        }
        self.seek_block(block);
        self.file
            .write_all(input)
            .map_err(|_| BlockIoError::DeviceIo {
                device: StorageDeviceId::new(1).unwrap(),
            })
    }

    fn flush(&mut self) -> Result<(), BlockIoError> {
        self.file.sync_all().map_err(|_| BlockIoError::DeviceIo {
            device: StorageDeviceId::new(1).unwrap(),
        })
    }

    fn discard_block(&mut self, _block: u64) -> Result<(), BlockIoError> {
        Ok(())
    }
}

fn report(stage: &str, filesystem: &SynFs<MAX_BLOCKS>) {
    println!(
        "[{stage}] used_blocks={} free_bytes={}",
        filesystem.used_blocks(),
        filesystem.free_bytes()
    );
}

fn read_back(filesystem: &SynFs<MAX_BLOCKS>, path: &str, expected: &[u8]) {
    match filesystem.lookup(path) {
        Ok(metadata) => println!(
            "[{path}] lookup ok: type={:?} size={}",
            metadata.file_type, metadata.size
        ),
        Err(error) => println!("[{path}] LOOKUP FAILED: {error:?}"),
    }
    let mut buffer = vec![0u8; expected.len().max(1)];
    match filesystem.read(path, &mut buffer) {
        Ok(read) => {
            if buffer[..read.bytes_read] == *expected {
                println!("[{path}] read ok: {} bytes match", read.bytes_read);
            } else {
                println!("[{path}] READ MISMATCH: {:?}", &buffer[..read.bytes_read]);
            }
        }
        Err(error) => println!("[{path}] READ FAILED: {error:?}"),
    }
}

#[test]
fn probe_fresh_volume_in_memory() {
    let mut filesystem = build_provisioned_volume();
    report("after provisioning", &filesystem);
    run_first_admin_sequence(&mut filesystem, "in-memory");
}

/// Reproduce the kernel's fallback path: AHCI mount fails -> SynFs::new().
#[test]
fn probe_bootstrap_new_filesystem() {
    let mut filesystem = SynFs::<MAX_BLOCKS>::new();
    println!("bootstrap filesystem constructed (no format, no load)");
    report("bootstrap fresh", &filesystem);
    run_first_admin_sequence(&mut filesystem, "bootstrap");
    match filesystem.lookup(DATABASE) {
        Ok(metadata) => println!(
            "[bootstrap] database survived: type={:?} size={}",
            metadata.file_type, metadata.size
        ),
        Err(error) => println!("[bootstrap] database lookup FAILED after confirm: {error:?}"),
    }
}

#[test]
fn probe_boot_from_disk_then_first_run() {
    let mut disk = DiskImage::create(Path::new("/tmp/ghostos-fsprobe-boot.img"));
    {
        let mut filesystem = build_provisioned_volume();
        filesystem.fsync(&mut disk).expect("initial fsync");
    }

    let mut image = vec![0u8; SynFs::<MAX_BLOCKS>::volume_bytes()];
    disk.seek_block(0);
    disk.file.read_exact(&mut image).expect("read back");
    let mut filesystem = SynFs::<MAX_BLOCKS>::load(&image).expect("boot-time mount");
    println!("mounted from disk");
    report("after boot mount", &filesystem);

    run_first_admin_sequence(&mut filesystem, "after-disk-mount");

    filesystem.fsync(&mut disk).expect("final fsync");
}

fn build_provisioned_volume() -> SynFs<MAX_BLOCKS> {
    let mut image = vec![0u8; SynFs::<MAX_BLOCKS>::volume_bytes()];
    SynFs::<MAX_BLOCKS>::format(&mut image).expect("format");
    let mut filesystem = SynFs::<MAX_BLOCKS>::load(&image).expect("load");

    for directory in [
        "/etc",
        "/etc/ghostos",
        "/system",
        "/system/services",
        "/var",
        "/var/log",
        "/home",
        "/tmp",
    ] {
        filesystem
            .create_directory(directory, true)
            .expect("create directory");
    }
    filesystem
        .write(
            "/etc/ghostos/settings",
            b"SYNSET\0boot_args=console=serial0\0machine_identity=probe\0network_identity=probe\0capabilities=a,b,c\0",
        )
        .expect("settings");
    filesystem
        .write("/etc/ghostos/machine-id", b"fsprobe-machine-id")
        .expect("machine id");
    filesystem
        .write("/etc/ghostos/network-id", b"fsprobe-network-id")
        .expect("network id");
    filesystem
        .write("/etc/ghostos/capabilities", b"a\nb\nc")
        .expect("capabilities");
    filesystem
        .write("/etc/ghostos/disk-format", b"1")
        .expect("disk format");
    let manifest = ServiceManifest::new();
    let mut bytes = [0; 2048];
    let length = manifest.encode(&mut bytes).expect("encode manifest");
    filesystem
        .write(ghostos_ghostfs::SERVICE_MANIFEST_PATH, &bytes[..length])
        .expect("manifest write");
    report("after provisioning", &filesystem);
    assert!(filesystem.lookup(DATABASE).is_err(), "database exists?!");
    filesystem
}

fn run_first_admin_sequence(filesystem: &mut SynFs<MAX_BLOCKS>, label: &str) {
    {
        let mut transaction = filesystem.transaction();
        transaction
            .create_directory("/system/security", true)
            .expect("security dir");
        transaction.write(USERNAME_PATH, b"admin").expect("username stage");
        transaction.commit().expect("username commit");
    }
    println!("[{label}] username save committed");
    report(&format!("{label} after username save"), filesystem);
    let (words, root, generation) = filesystem.debug_arena_dump();
    println!(
        "[{label}] arena: root={root} gen={generation} words={words:05x?}"
    );
    println!(
        "[{label}] username data probe: {:x?}",
        filesystem.debug_data_probe(USERNAME_PATH)
    );
    read_back(filesystem, USERNAME_PATH, b"admin");

    {
        let mut transaction = filesystem.transaction();
        transaction
            .write(CREDENTIAL_PATH, &[1u8, 5, b'h', b'e', b'l', b'l', b'o'])
            .expect("credential stage");
        transaction.commit().expect("credential commit");
    }
    println!("[{label}] credential save committed");
    report(&format!("{label} after credential save"), filesystem);
    let (words, root, generation) = filesystem.debug_arena_dump();
    println!(
        "[{label}] arena: root={root} gen={generation} words={words:05x?}"
    );
    println!(
        "[{label}] username data probe: {:x?}",
        filesystem.debug_data_probe(USERNAME_PATH)
    );
    println!(
        "[{label}] credential data probe: {:x?}",
        filesystem.debug_data_probe(CREDENTIAL_PATH)
    );
    read_back(filesystem, USERNAME_PATH, b"admin");
    read_back(filesystem, CREDENTIAL_PATH, &[1u8, 5, b'h', b'e', b'l', b'l', b'o']);

    let mut record = [0u8; 2 + 32 + 2 + 96];
    record[0] = 1;
    record[1] = 5;
    record[2..7].copy_from_slice(b"admin");
    record[2 + 32] = 1;
    record[3 + 32] = 5;
    record[4 + 32..4 + 32 + 5].copy_from_slice(b"hello");
    {
        let mut transaction = filesystem.transaction();
        assert!(transaction.lookup(DATABASE).is_err(), "database exists?!");
        transaction
            .write(DATABASE, &record[..4 + 32 + 5])
            .expect("record stage");
        transaction.delete(USERNAME_PATH).expect("username delete");
        transaction.delete(CREDENTIAL_PATH).expect("credential delete");
        transaction.commit().expect("confirm commit");
    }
    println!("[{label}] confirm transaction committed");
    report(&format!("{label} after confirm transaction"), filesystem);

    match filesystem.lookup(USERNAME_PATH) {
        Ok(_) => panic!("[{label}] username still present after confirm"),
        Err(_) => println!("[{label}] username removed after confirm"),
    }
}
