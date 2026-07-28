use crate::{
    Error, File, Partition, ReadAt, le_u16, le_u32, path_components,
};

const FAT_ENTRY_END: u32 = 0x0fff_fff8;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_LONG_NAME: u8 = 0x0f;

pub struct Fat32 {
    partition_start: u64,
    partition_end: u64,
    fat_start: u64,
    data_start: u64,
    bytes_per_cluster: u32,
    root_cluster: u32,
}

struct DirectoryEntry {
    first_cluster: u32,
    size: u32,
    attributes: u8,
}

impl Fat32 {
    pub fn mount<D: ReadAt>(
        device: &mut D,
        partition: Partition,
        scratch: &mut [u8],
    ) -> Result<Self, Error> {
        if scratch.len() < 512 {
            return Err(Error::BufferTooSmall)
        }
        let partition_end = partition.start
            .checked_add(partition.length)
            .ok_or(Error::InvalidOffset)?;
        if partition.length < 512 {
            return Err(Error::Corrupt)
        }
        device.read_at(partition.start, &mut scratch[..512])?;
        if scratch[82..90] != *b"FAT32   " || scratch[510..512] != [0x55, 0xaa] {
            return Err(Error::Corrupt)
        }

        let bytes_per_sector = le_u16(&scratch[11..]) as u32;
        let sectors_per_cluster = scratch[13] as u32;
        let reserved_sectors = le_u16(&scratch[14..]) as u32;
        let fat_count = scratch[16] as u32;
        let sectors_per_fat = le_u32(&scratch[36..]);
        let root_cluster = le_u32(&scratch[44..]) & 0x0fff_ffff;
        if !bytes_per_sector.is_power_of_two()
            || !(512..=4096).contains(&bytes_per_sector)
            || !sectors_per_cluster.is_power_of_two()
            || sectors_per_cluster == 0
            || fat_count == 0
            || sectors_per_fat == 0
            || root_cluster < 2
        {
            return Err(Error::Corrupt)
        }

        let fat_start = partition.start
            + reserved_sectors as u64 * bytes_per_sector as u64;
        let data_start = fat_start
            + fat_count as u64 * sectors_per_fat as u64 * bytes_per_sector as u64;
        Ok(Self {
            partition_start: partition.start,
            partition_end,
            fat_start,
            data_start,
            bytes_per_cluster: bytes_per_sector * sectors_per_cluster,
            root_cluster,
        })
    }

    pub fn open<D: ReadAt>(
        &self,
        device: &mut D,
        path: &str,
        _scratch: &mut [u8],
    ) -> Result<File, Error> {
        let mut directory = self.root_cluster;
        let mut components = path_components(path).peekable();
        let Some(first) = components.next() else {
            return Err(Error::NotFound)
        };
        let mut entry = self.find_in_directory(device, directory, first)?;

        for component in components {
            if entry.attributes & ATTR_DIRECTORY == 0 {
                return Err(Error::NotFound)
            }
            directory = entry.first_cluster;
            entry = self.find_in_directory(device, directory, component)?;
        }

        if entry.attributes & ATTR_DIRECTORY != 0 {
            return Err(Error::NotSupported)
        }
        Ok(File {
            id: entry.first_cluster as u64,
            auxiliary: 0,
            size: entry.size as u64,
        })
    }

    pub fn read<D: ReadAt>(
        &self,
        device: &mut D,
        file: File,
        offset: u64,
        output: &mut [u8],
        _scratch: &mut [u8],
    ) -> Result<usize, Error> {
        if offset >= file.size || output.is_empty() {
            return Ok(0)
        }
        let wanted = output.len().min((file.size - offset) as usize);
        let cluster_size = self.bytes_per_cluster as u64;
        let mut cluster = file.id as u32;
        let mut clusters_to_skip = offset / cluster_size;
        while clusters_to_skip != 0 {
            cluster = self.next_cluster(device, cluster)?;
            clusters_to_skip -= 1;
        }

        let mut in_cluster = (offset % cluster_size) as usize;
        let mut written = 0;
        while written < wanted {
            if cluster < 2 || cluster >= FAT_ENTRY_END {
                return Err(Error::Corrupt)
            }
            let available = self.bytes_per_cluster as usize - in_cluster;
            let take = available.min(wanted - written);
            let disk_offset = self.cluster_offset(cluster)?
                .checked_add(in_cluster as u64)
                .ok_or(Error::InvalidOffset)?;
            self.read_at(device, disk_offset, &mut output[written..written + take])?;
            written += take;
            in_cluster = 0;
            if written < wanted {
                cluster = self.next_cluster(device, cluster)?;
            }
        }
        Ok(written)
    }

    fn find_in_directory<D: ReadAt>(
        &self,
        device: &mut D,
        mut cluster: u32,
        target: &str,
    ) -> Result<DirectoryEntry, Error> {
        let mut long_name = [0u16; 260];
        let mut long_name_valid = false;
        let entries_per_cluster = self.bytes_per_cluster as usize / 32;

        loop {
            for index in 0..entries_per_cluster {
                let mut raw = [0u8; 32];
                let offset = self.cluster_offset(cluster)?
                    .checked_add((index * 32) as u64)
                    .ok_or(Error::InvalidOffset)?;
                self.read_at(device, offset, &mut raw)?;

                if raw[0] == 0 {
                    return Err(Error::NotFound)
                }
                if raw[0] == 0xe5 {
                    long_name_valid = false;
                    continue
                }
                if raw[11] == ATTR_LONG_NAME {
                    let sequence = raw[0] & 0x1f;
                    if sequence == 0 || sequence > 20 {
                        long_name_valid = false;
                        continue
                    }
                    if raw[0] & 0x40 != 0 {
                        long_name.fill(0);
                        long_name_valid = true;
                    }
                    if long_name_valid {
                        copy_lfn_fragment(&raw, sequence, &mut long_name);
                    }
                    continue
                }

                let matches = if long_name_valid {
                    utf16_name_matches(&long_name, target)
                } else {
                    short_name_matches(&raw[..11], target)
                };
                long_name_valid = false;
                if matches {
                    let first_cluster =
                        ((le_u16(&raw[20..]) as u32) << 16) | le_u16(&raw[26..]) as u32;
                    return Ok(DirectoryEntry {
                        first_cluster,
                        size: le_u32(&raw[28..]),
                        attributes: raw[11],
                    })
                }
            }

            cluster = self.next_cluster(device, cluster)?;
            if cluster >= FAT_ENTRY_END {
                return Err(Error::NotFound)
            }
        }
    }

    fn next_cluster<D: ReadAt>(
        &self,
        device: &mut D,
        cluster: u32,
    ) -> Result<u32, Error> {
        let mut bytes = [0u8; 4];
        let offset = self.fat_start
            .checked_add(cluster as u64 * 4)
            .ok_or(Error::InvalidOffset)?;
        self.read_at(device, offset, &mut bytes)?;
        Ok(le_u32(&bytes) & 0x0fff_ffff)
    }

    fn cluster_offset(&self, cluster: u32) -> Result<u64, Error> {
        if cluster < 2 {
            return Err(Error::Corrupt)
        }
        self.data_start
            .checked_add((cluster - 2) as u64 * self.bytes_per_cluster as u64)
            .ok_or(Error::InvalidOffset)
    }

    fn read_at<D: ReadAt>(
        &self,
        device: &mut D,
        offset: u64,
        bytes: &mut [u8],
    ) -> Result<(), Error> {
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or(Error::InvalidOffset)?;
        if offset < self.partition_start || end > self.partition_end {
            return Err(Error::InvalidOffset)
        }
        device.read_at(offset, bytes)
    }
}

fn copy_lfn_fragment(entry: &[u8; 32], sequence: u8, name: &mut [u16; 260]) {
    let start = (sequence as usize - 1) * 13;
    let positions = [
        1usize, 3, 5, 7, 9,
        14, 16, 18, 20, 22, 24,
        28, 30,
    ];
    for (index, position) in positions.into_iter().enumerate() {
        name[start + index] = le_u16(&entry[position..]);
    }
}

fn utf16_name_matches(name: &[u16], target: &str) -> bool {
    let mut target_chars = target.chars();
    for code_unit in name {
        if *code_unit == 0 || *code_unit == 0xffff {
            return target_chars.next().is_none()
        }
        let Some(character) = char::from_u32(*code_unit as u32) else {
            return false
        };
        let Some(expected) = target_chars.next() else {
            return false
        };
        if !character.eq_ignore_ascii_case(&expected) {
            return false
        }
    }
    target_chars.next().is_none()
}

fn short_name_matches(name: &[u8], target: &str) -> bool {
    let mut rendered = [0u8; 12];
    let mut length = 0;
    for byte in &name[..8] {
        if *byte != b' ' {
            rendered[length] = *byte;
            length += 1;
        }
    }
    if name[8..11].iter().any(|byte| *byte != b' ') {
        rendered[length] = b'.';
        length += 1;
        for byte in &name[8..11] {
            if *byte != b' ' {
                rendered[length] = *byte;
                length += 1;
            }
        }
    }
    rendered[..length].eq_ignore_ascii_case(target.as_bytes())
}
