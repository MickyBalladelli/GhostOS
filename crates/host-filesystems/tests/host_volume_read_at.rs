// Inventory: coverage_59_5.rs (legacy roadmap section 59).
use ghostos_host_filesystems::{
    detect, scan_partitions, Error, FileSystemKind, Partition, PartitionKind, ReadAt,
};

struct Image {
    bytes: Vec<u8>,
    fail: bool,
}

impl Image {
    fn new(length: usize) -> Self {
        Self {
            bytes: vec![0; length],
            fail: false,
        }
    }
}

impl ReadAt for Image {
    fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> Result<(), Error> {
        if self.fail {
            return Err(Error::Io)
        }
        let start = usize::try_from(offset).map_err(|_| Error::InvalidOffset)?;
        let end = start.checked_add(bytes.len()).ok_or(Error::InvalidOffset)?;
        let source = self.bytes.get(start..end).ok_or(Error::InvalidOffset)?;
        bytes.copy_from_slice(source);
        Ok(())
    }
}

fn partition() -> Partition {
    Partition {
        start: 0,
        length: 4096,
        kind: PartitionKind::Other,
    }
}

#[test]
fn detects_valid_read_only_filesystem_signatures() {
    let mut ext4 = Image::new(4096);
    ext4.bytes[1080..1082].copy_from_slice(&0xef53u16.to_le_bytes());
    assert_eq!(detect(&mut ext4, partition(), &mut [0; 2048]), Ok(FileSystemKind::Ext4));

    let mut fat = Image::new(4096);
    fat.bytes[82..90].copy_from_slice(b"FAT32   ");
    assert_eq!(detect(&mut fat, partition(), &mut [0; 2048]), Ok(FileSystemKind::Fat32));

    let mut ntfs = Image::new(4096);
    ntfs.bytes[3..11].copy_from_slice(b"NTFS    ");
    assert_eq!(detect(&mut ntfs, partition(), &mut [0; 2048]), Ok(FileSystemKind::Ntfs));
}

#[test]
fn rejects_truncated_corrupt_unsupported_and_failing_devices() {
    let mut image = Image::new(2048);
    assert_eq!(detect(&mut image, partition(), &mut [0; 2047]), Err(Error::BufferTooSmall));
    assert_eq!(detect(&mut image, partition(), &mut [0; 2048]), Err(Error::NotSupported));

    image.bytes[3..11].copy_from_slice(b"NTFS    ");
    image.fail = true;
    assert_eq!(detect(&mut image, partition(), &mut [0; 2048]), Err(Error::Io));

    let mut short = Image::new(1024);
    assert_eq!(detect(&mut short, partition(), &mut [0; 2048]), Err(Error::InvalidOffset));
    assert_eq!(detect(&mut short, Partition { length: 1024, ..partition() }, &mut [0; 2048]), Err(Error::Corrupt));
}

#[test]
fn scans_mbr_partition_bounds_and_signature() {
    let mut image = Image::new(4096);
    image.bytes[510..512].copy_from_slice(&[0x55, 0xaa]);
    image.bytes[446 + 4] = 0x83;
    image.bytes[446 + 8..446 + 12].copy_from_slice(&1u32.to_le_bytes());
    image.bytes[446 + 12..446 + 16].copy_from_slice(&4u32.to_le_bytes());
    let mut partitions = [Partition::EMPTY; 2];
    assert_eq!(
        scan_partitions(&mut image, 512, &mut partitions, &mut [0; 512]),
        Ok(1)
    );
    assert_eq!(partitions[0].start, 512);
    assert_eq!(partitions[0].length, 2048);
    assert_eq!(partitions[0].kind, PartitionKind::LinuxFilesystem);

    image.bytes[510] = 0;
    assert_eq!(scan_partitions(&mut image, 512, &mut partitions, &mut [0; 512]), Err(Error::Corrupt));
}
