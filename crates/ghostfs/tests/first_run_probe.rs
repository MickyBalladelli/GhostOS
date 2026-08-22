//! TEMPORARY diagnostic probe: replays the provisioner volume layout plus the
//! kernel first-admin sequence against GhostFS to find why CONFIRM fails.
//! Delete this file after diagnosis.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use ghostos_ghostfs::{
    BlockIoError, BlockStore, ServiceManifest, ServiceManifestEntry, StorageDeviceId, SynFs,
    BLOCK_SIZE,
};

const MAX_BLOCKS: usize = 32;

const SERVICE_BIN: usize = 1058;
const LOGIN_BIN: usize = 8897;
const SHELL_BIN: usize = 66792;

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
    let mut manifest = ServiceManifest::new();
    for role in 1..=14u8 {
        let size = match role {
            9 => SHELL_BIN,
            14 => LOGIN_BIN,
            _ => SERVICE_BIN,
        };
        let image = vec![0xA5u8; size];
        let path = format!("/system/services/{role}.pkg");
        filesystem.write(&path, &image).expect("service package");
        manifest
            .push(ServiceManifestEntry::new(role, &path, &image).expect("manifest entry"))
            .expect("manifest push");
    }
    let mut bytes = [0; 2048];
    let length = manifest.encode(&mut bytes).expect("encode manifest");
    filesystem
        .write(ghostos_ghostfs::SERVICE_MANIFEST_PATH, &bytes[..length])
        .expect("manifest write");
    filesystem.flush(&mut image).expect("flush");
    filesystem.check_consistency().expect("consistent");
    filesystem
}

#[test]
fn probe_first_run_commit() {
    let mut filesystem = build_provisioned_volume();
    report("after provisioning", &filesystem);

    const DATABASE: &str = "/system/security/authorization";
    const USERNAME_PATH: &str = "/system/security/first-admin-username";
    const CREDENTIAL_PATH: &str = "/system/security/first-admin-credential";

    assert!(filesystem.lookup(DATABASE).is_err(), "database exists?!");

    match filesystem.transaction().create_directory("/system/security", true) {
        Ok(_) => {}
        Err(error) => println!("create /system/security failed in standalone txn: {error:?}"),
    }
    {
        let mut transaction = filesystem.transaction();
        transaction.create_directory("/system/security", true).expect("security dir");
        match transaction.write(USERNAME_PATH, b"admin") {
            Ok(_) => println!("username staged ok"),
            Err(error) => println!("USERNAME STAGE FAILED: {error:?}"),
        }
        match transaction.commit() {
            Ok(_) => println!("username committed"),
            Err(error) => println!("USERNAME COMMIT FAILED: {error:?}"),
        }
    }
    report("after username save", &filesystem);

    {
        let mut transaction = filesystem.transaction();
        match transaction.write(CREDENTIAL_PATH, &[1u8, 5, b'h', b'e', b'l', b'l', b'o']) {
            Ok(_) => println!("credential staged ok"),
            Err(error) => println!("CREDENTIAL STAGE FAILED: {error:?}"),
        }
        match transaction.commit() {
            Ok(_) => println!("credential committed"),
            Err(error) => println!("CREDENTIAL COMMIT FAILED: {error:?}"),
        }
    }
    report("after credential save", &filesystem);

    let mut record = [0u8; 2 + 32 + 2 + 96];
    record[0] = 1;
    record[1] = 5;
    record[2..7].copy_from_slice(b"admin");
    record[2 + 32] = 1;
    record[3 + 32] = 5;
    record[4 + 32..4 + 32 + 5].copy_from_slice(b"hello");
    {
        let mut transaction = filesystem.transaction();
        if let Ok(_) = transaction.lookup(DATABASE) {
            panic!("database already exists before commit");
        }
        match transaction.write(DATABASE, &record[..4 + 32 + 5]) {
            Ok(_) => println!("record staged ok"),
            Err(error) => println!("RECORD STAGE FAILED: {error:?}"),
        }
        match transaction.delete(USERNAME_PATH) {
            Ok(_) => println!("username delete staged ok"),
            Err(error) => println!("USERNAME DELETE STAGE FAILED: {error:?}"),
        }
        match transaction.delete(CREDENTIAL_PATH) {
            Ok(_) => println!("credential delete staged ok"),
            Err(error) => println!("CREDENTIAL DELETE STAGE FAILED: {error:?}"),
        }
        match transaction.commit() {
            Ok(commit) => println!("commit ok: {commit:?}"),
            Err(error) => println!("COMMIT FAILED: {error:?}"),
        }
    }
    report("after confirm transaction", &filesystem);

    let mut disk = DiskImage::create(Path::new("/tmp/ghostos-fsprobe-volume.img"));
    match filesystem.fsync(&mut disk) {
        Ok(_) => println!("fsync ok"),
        Err(error) => println!("FSYNC FAILED: {error:?}"),
    }
}
