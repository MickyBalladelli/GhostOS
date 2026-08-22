#![no_main]

use libfuzzer_sys::fuzz_target;
use ghostos_host_filesystems::{scan_partitions, Error, Partition, ReadAt, Volume};

struct Bytes<'a> {
    bytes: &'a [u8],
}

impl ReadAt for Bytes<'_> {
    fn read_at(&mut self, offset: u64, output: &mut [u8]) -> Result<(), Error> {
        let start = usize::try_from(offset).map_err(|_| Error::InvalidOffset)?;
        let end = start.checked_add(output.len()).ok_or(Error::InvalidOffset)?;
        let source = self.bytes.get(start..end).ok_or(Error::Io)?;
        output.copy_from_slice(source);
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    let mut device = Bytes { bytes: data };
    let mut partitions = [Partition::EMPTY; 32];
    let mut scratch = [0; 64 * 1024];
    let logical_block_size = 512 + data.first().copied().unwrap_or(0) as usize;
    let _ = scan_partitions(
        &mut device,
        logical_block_size,
        &mut partitions,
        &mut scratch,
    );

    let partition = Partition {
        start: 0,
        length: data.len() as u64,
        kind: ghostos_host_filesystems::PartitionKind::Other,
    };
    let mut device = Bytes { bytes: data };
    let _ = Volume::mount(&mut device, partition, &mut scratch);
});
