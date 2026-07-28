use crate::{
    Error, File, Partition, ReadAt, le_u16, le_u32, path_components,
};

const EXT4_MAGIC: u16 = 0xef53;
const EXTENT_MAGIC: u16 = 0xf30a;
const EXTENTS_FLAG: u32 = 0x0008_0000;
const ROOT_INODE: u32 = 2;

pub struct Ext4 {
    partition_start: u64,
    partition_end: u64,
    block_size: u32,
    inodes_per_group: u32,
    inode_size: u16,
    descriptor_size: u16,
}

struct Inode {
    size: u64,
    flags: u32,
    extent_root: [u8; 60],
}

impl Ext4 {
    pub fn mount<D: ReadAt>(
        device: &mut D,
        partition: Partition,
        scratch: &mut [u8],
    ) -> Result<Self, Error> {
        if scratch.len() < 1024 {
            return Err(Error::BufferTooSmall)
        }
        let partition_end = partition.start
            .checked_add(partition.length)
            .ok_or(Error::InvalidOffset)?;
        if partition.length < 2048 {
            return Err(Error::Corrupt)
        }
        device.read_at(partition.start + 1024, &mut scratch[..1024])?;
        if le_u16(&scratch[56..]) != EXT4_MAGIC {
            return Err(Error::Corrupt)
        }

        let block_size = 1024u32
            .checked_shl(le_u32(&scratch[24..]))
            .ok_or(Error::Corrupt)?;
        let blocks_per_group = le_u32(&scratch[32..]);
        let inodes_per_group = le_u32(&scratch[40..]);
        let inode_size = le_u16(&scratch[88..]).max(128);
        let descriptor_size = le_u16(&scratch[254..]).max(32);
        if !block_size.is_power_of_two()
            || !(1024..=65536).contains(&block_size)
            || blocks_per_group == 0
            || inodes_per_group == 0
            || !(128..=1024).contains(&inode_size)
            || descriptor_size < 32
        {
            return Err(Error::Corrupt)
        }

        Ok(Self {
            partition_start: partition.start,
            partition_end,
            block_size,
            inodes_per_group,
            inode_size,
            descriptor_size,
        })
    }

    pub fn open<D: ReadAt>(
        &self,
        device: &mut D,
        path: &str,
        scratch: &mut [u8],
    ) -> Result<File, Error> {
        self.require_scratch(scratch)?;
        let mut inode_number = ROOT_INODE;
        let mut components = path_components(path).peekable();
        if components.peek().is_none() {
            return Err(Error::NotFound)
        }

        for component in components {
            let directory = self.load_inode(device, inode_number, scratch)?;
            inode_number = self.find_in_directory(
                device,
                &directory,
                component.as_bytes(),
                scratch,
            )?;
        }

        let inode = self.load_inode(device, inode_number, scratch)?;
        Ok(File {
            id: inode_number as u64,
            auxiliary: 0,
            size: inode.size,
        })
    }

    pub fn read<D: ReadAt>(
        &self,
        device: &mut D,
        file: File,
        offset: u64,
        output: &mut [u8],
        scratch: &mut [u8],
    ) -> Result<usize, Error> {
        self.require_scratch(scratch)?;
        if offset >= file.size || output.is_empty() {
            return Ok(0)
        }
        let inode = self.load_inode(device, file.id as u32, scratch)?;
        let wanted = output.len().min((file.size - offset) as usize);
        let block_size = self.block_size as u64;
        let mut written = 0;
        while written < wanted {
            let position = offset + written as u64;
            let file_block = position / block_size;
            let in_block = (position % block_size) as usize;
            let take = (self.block_size as usize - in_block).min(wanted - written);
            match self.map_block(device, &inode, file_block, scratch)? {
                Some(block) => {
                    let disk_offset = self.block_offset(block)?
                        .checked_add(in_block as u64)
                        .ok_or(Error::InvalidOffset)?;
                    self.read_at(device, disk_offset, &mut output[written..written + take])?
                }
                None => output[written..written + take].fill(0),
            }
            written += take;
        }
        Ok(written)
    }

    fn find_in_directory<D: ReadAt>(
        &self,
        device: &mut D,
        directory: &Inode,
        target: &[u8],
        scratch: &mut [u8],
    ) -> Result<u32, Error> {
        let mut logical_block = 0u64;
        let block_count = directory.size.div_ceil(self.block_size as u64);
        while logical_block < block_count {
            let Some(physical_block) =
                self.map_block(device, directory, logical_block, scratch)?
            else {
                logical_block += 1;
                continue
            };
            let block_start = self.block_offset(physical_block)?;
            let mut position = 0usize;
            while position + 8 <= self.block_size as usize {
                let mut header = [0u8; 8];
                self.read_at(device, block_start + position as u64, &mut header)?;
                let inode = le_u32(&header);
                let record_length = le_u16(&header[4..]) as usize;
                let name_length = header[6] as usize;
                if record_length < 8
                    || record_length & 3 != 0
                    || position + record_length > self.block_size as usize
                    || name_length > record_length - 8
                {
                    return Err(Error::Corrupt)
                }
                if inode != 0 && name_length == target.len() {
                    let mut name = [0u8; 255];
                    if name_length > name.len() {
                        return Err(Error::Corrupt)
                    }
                    self.read_at(
                        device,
                        block_start + position as u64 + 8,
                        &mut name[..name_length],
                    )?;
                    if &name[..name_length] == target {
                        return Ok(inode)
                    }
                }
                position += record_length;
            }
            logical_block += 1;
        }
        Err(Error::NotFound)
    }

    fn load_inode<D: ReadAt>(
        &self,
        device: &mut D,
        inode_number: u32,
        scratch: &mut [u8],
    ) -> Result<Inode, Error> {
        if inode_number == 0 {
            return Err(Error::Corrupt)
        }
        let index = inode_number - 1;
        let group = index / self.inodes_per_group;
        let index_in_group = index % self.inodes_per_group;
        let descriptor_table_block = if self.block_size == 1024 { 2 } else { 1 };
        let descriptor_offset = self
            .block_offset(descriptor_table_block)?
            .checked_add(group as u64 * self.descriptor_size as u64)
            .ok_or(Error::InvalidOffset)?;
        let mut descriptor = [0u8; 64];
        let descriptor_bytes = (self.descriptor_size as usize).min(descriptor.len());
        self.read_at(device, descriptor_offset, &mut descriptor[..descriptor_bytes])?;
        let inode_table_low = le_u32(&descriptor[8..]) as u64;
        let inode_table_high = if descriptor_bytes >= 44 {
            le_u32(&descriptor[40..]) as u64
        } else {
            0
        };
        let inode_table = inode_table_low | inode_table_high << 32;
        if inode_table == 0 {
            return Err(Error::Corrupt)
        }

        let inode_offset = self.block_offset(inode_table)?
            .checked_add(index_in_group as u64 * self.inode_size as u64)
            .ok_or(Error::InvalidOffset)?;
        self.read_at(device, inode_offset, &mut scratch[..self.inode_size as usize])?;
        let size = le_u32(&scratch[4..]) as u64
            | (le_u32(&scratch[108..]) as u64) << 32;
        let flags = le_u32(&scratch[32..]);
        let mut extent_root = [0u8; 60];
        extent_root.copy_from_slice(&scratch[40..100]);
        Ok(Inode {
            size,
            flags,
            extent_root,
        })
    }

    fn map_block<D: ReadAt>(
        &self,
        device: &mut D,
        inode: &Inode,
        file_block: u64,
        scratch: &mut [u8],
    ) -> Result<Option<u64>, Error> {
        if inode.flags & EXTENTS_FLAG == 0 {
            return Err(Error::NotSupported)
        }
        let mut root = inode.extent_root.as_slice();
        let mut depth = le_u16(&root[6..]);

        loop {
            if le_u16(root) != EXTENT_MAGIC {
                return Err(Error::Corrupt)
            }
            let entries = le_u16(&root[2..]) as usize;
            let maximum = le_u16(&root[4..]) as usize;
            if entries > maximum || 12 + entries * 12 > root.len() {
                return Err(Error::Corrupt)
            }

            if depth == 0 {
                for index in 0..entries {
                    let extent = &root[12 + index * 12..];
                    let logical = le_u32(extent) as u64;
                    let length = (le_u16(&extent[4..]) & 0x7fff) as u64;
                    if file_block >= logical && file_block < logical + length {
                        let physical = le_u32(&extent[8..]) as u64
                            | (le_u16(&extent[6..]) as u64) << 32;
                        return Ok(Some(physical + file_block - logical))
                    }
                }
                return Ok(None)
            }

            let mut child = None;
            for index in 0..entries {
                let extent_index = &root[12 + index * 12..];
                if le_u32(extent_index) as u64 > file_block {
                    break
                }
                child = Some(
                    le_u32(&extent_index[4..]) as u64
                        | (le_u16(&extent_index[8..]) as u64) << 32,
                );
            }
            let child = child.ok_or(Error::Corrupt)?;
            self.read_at(
                device,
                self.block_offset(child)?,
                &mut scratch[..self.block_size as usize],
            )?;
            root = &scratch[..self.block_size as usize];
            let child_depth = le_u16(&root[6..]);
            if child_depth + 1 != depth {
                return Err(Error::Corrupt)
            }
            depth = child_depth;
        }
    }

    fn block_offset(&self, block: u64) -> Result<u64, Error> {
        self.partition_start
            .checked_add(block.checked_mul(self.block_size as u64).ok_or(Error::InvalidOffset)?)
            .ok_or(Error::InvalidOffset)
    }

    fn require_scratch(&self, scratch: &[u8]) -> Result<(), Error> {
        if scratch.len() < self.block_size as usize
            || scratch.len() < self.inode_size as usize
        {
            Err(Error::BufferTooSmall)
        } else {
            Ok(())
        }
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
