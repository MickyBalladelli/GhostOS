use crate::{
    Error, File, Partition, ReadAt, le_u16, le_u32, le_u64, path_components,
};

const FILE_RECORD_MAGIC: &[u8; 4] = b"FILE";
const ATTRIBUTE_END: u32 = 0xffff_ffff;
const ATTRIBUTE_DATA: u32 = 0x80;
const ATTRIBUTE_INDEX_ROOT: u32 = 0x90;
const ATTRIBUTE_INDEX_ALLOCATION: u32 = 0xa0;
const ROOT_FILE_RECORD: u64 = 5;

pub struct Ntfs {
    partition_start: u64,
    partition_end: u64,
    bytes_per_sector: u16,
    bytes_per_cluster: u32,
    file_record_size: u32,
    index_record_size: u32,
    mft_offset: u64,
    mft_runs: [u8; 512],
    mft_runs_length: u16,
}

struct Attribute {
    offset: usize,
    length: usize,
    non_resident: bool,
}

impl Ntfs {
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
        if scratch[3..11] != *b"NTFS    " || scratch[510..512] != [0x55, 0xaa] {
            return Err(Error::Corrupt)
        }

        let bytes_per_sector = le_u16(&scratch[11..]);
        let sectors_per_cluster = scratch[13] as u32;
        let bytes_per_cluster = (bytes_per_sector as u32)
            .checked_mul(sectors_per_cluster)
            .ok_or(Error::Corrupt)?;
        let file_record_size = decode_record_size(scratch[64] as i8, bytes_per_cluster)?;
        let index_record_size = decode_record_size(scratch[68] as i8, bytes_per_cluster)?;
        let mft_lcn = le_u64(&scratch[48..]);
        if !bytes_per_sector.is_power_of_two()
            || !(512..=4096).contains(&bytes_per_sector)
            || sectors_per_cluster == 0
            || !sectors_per_cluster.is_power_of_two()
            || file_record_size < 512
            || file_record_size > 65536
            || index_record_size < 512
            || index_record_size > 65536
        {
            return Err(Error::Corrupt)
        }

        if scratch.len() < file_record_size.max(index_record_size) as usize {
            return Err(Error::BufferTooSmall)
        }

        let mut volume = Self {
            partition_start: partition.start,
            partition_end,
            bytes_per_sector,
            bytes_per_cluster,
            file_record_size,
            index_record_size,
            mft_offset: partition.start
                .checked_add(
                    mft_lcn
                        .checked_mul(bytes_per_cluster as u64)
                        .ok_or(Error::InvalidOffset)?,
                )
                .ok_or(Error::InvalidOffset)?,
            mft_runs: [0; 512],
            mft_runs_length: 0,
        };
        volume.read_at(
            device,
            volume.mft_offset,
            &mut scratch[..file_record_size as usize],
        )?;
        apply_fixups(
            &mut scratch[..file_record_size as usize],
            bytes_per_sector as usize,
        )?;
        let mft_data = find_unnamed_attribute(
            &scratch[..file_record_size as usize],
            ATTRIBUTE_DATA,
        )?
        .ok_or(Error::Corrupt)?;
        if !mft_data.non_resident {
            return Err(Error::Corrupt)
        }
        let runlist_offset = le_u16(&scratch[mft_data.offset + 32..]) as usize;
        let runlist_start = mft_data.offset
            .checked_add(runlist_offset)
            .ok_or(Error::Corrupt)?;
        let runlist_end = mft_data.offset + mft_data.length;
        let runlist_length = runlist_end
            .checked_sub(runlist_start)
            .ok_or(Error::Corrupt)?;
        if runlist_length > volume.mft_runs.len() {
            return Err(Error::NotSupported)
        }
        volume.mft_runs[..runlist_length]
            .copy_from_slice(&scratch[runlist_start..runlist_end]);
        volume.mft_runs_length = runlist_length as u16;
        Ok(volume)
    }

    pub fn open<D: ReadAt>(
        &self,
        device: &mut D,
        path: &str,
        scratch: &mut [u8],
    ) -> Result<File, Error> {
        self.require_scratch(scratch)?;
        let mut record_number = ROOT_FILE_RECORD;
        let mut components = path_components(path).peekable();
        if components.peek().is_none() {
            return Err(Error::NotFound)
        }

        for component in components {
            self.load_record(device, record_number, scratch)?;
            record_number = self.find_index_entry(device, scratch, component)?;
        }

        self.load_record(device, record_number, scratch)?;
        let data = find_unnamed_attribute(scratch, ATTRIBUTE_DATA)?
            .ok_or(Error::NotFound)?;
        let size = if data.non_resident {
            le_u64(&scratch[data.offset + 48..])
        } else {
            le_u32(&scratch[data.offset + 16..]) as u64
        };
        Ok(File {
            id: record_number,
            auxiliary: 0,
            size,
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
        self.load_record(device, file.id, scratch)?;
        let attribute = find_unnamed_attribute(scratch, ATTRIBUTE_DATA)?
            .ok_or(Error::NotFound)?;
        let wanted = output.len().min((file.size - offset) as usize);

        if !attribute.non_resident {
            let value_length = le_u32(&scratch[attribute.offset + 16..]) as usize;
            let value_offset = le_u16(&scratch[attribute.offset + 20..]) as usize;
            let value_start = attribute.offset
                .checked_add(value_offset)
                .ok_or(Error::Corrupt)?;
            if value_start + value_length > attribute.offset + attribute.length
                || offset as usize + wanted > value_length
            {
                return Err(Error::Corrupt)
            }
            output[..wanted].copy_from_slice(
                &scratch[value_start + offset as usize..value_start + offset as usize + wanted],
            );
            return Ok(wanted)
        }

        let runlist_offset = le_u16(&scratch[attribute.offset + 32..]) as usize;
        let runlist_start = attribute.offset
            .checked_add(runlist_offset)
            .ok_or(Error::Corrupt)?;
        if runlist_start >= attribute.offset + attribute.length {
            return Err(Error::Corrupt)
        }
        self.read_runs(
            device,
            &scratch[runlist_start..attribute.offset + attribute.length],
            offset,
            &mut output[..wanted],
        )?;
        Ok(wanted)
    }

    fn load_record<D: ReadAt>(
        &self,
        device: &mut D,
        record_number: u64,
        scratch: &mut [u8],
    ) -> Result<(), Error> {
        let offset = record_number
            .checked_mul(self.file_record_size as u64)
            .ok_or(Error::InvalidOffset)?;
        let record = &mut scratch[..self.file_record_size as usize];
        if self.mft_runs_length == 0 {
            self.read_at(
                device,
                self.mft_offset
                    .checked_add(offset)
                    .ok_or(Error::InvalidOffset)?,
                record,
            )?
        } else {
            self.read_runs(
                device,
                &self.mft_runs[..self.mft_runs_length as usize],
                offset,
                record,
            )?
        }
        apply_fixups(record, self.bytes_per_sector as usize)?;
        if &record[..4] != FILE_RECORD_MAGIC {
            return Err(Error::Corrupt)
        }
        Ok(())
    }

    fn find_index_entry<D: ReadAt>(
        &self,
        device: &mut D,
        record: &mut [u8],
        target: &str,
    ) -> Result<u64, Error> {
        let attribute = find_unnamed_attribute(record, ATTRIBUTE_INDEX_ROOT)?
            .ok_or(Error::NotFound)?;
        if attribute.non_resident {
            return Err(Error::Corrupt)
        }
        let value_length = le_u32(&record[attribute.offset + 16..]) as usize;
        let value_offset = le_u16(&record[attribute.offset + 20..]) as usize;
        let value_start = attribute.offset
            .checked_add(value_offset)
            .ok_or(Error::Corrupt)?;
        let value_end = value_start
            .checked_add(value_length)
            .ok_or(Error::Corrupt)?;
        if value_end > attribute.offset + attribute.length || value_length < 32 {
            return Err(Error::Corrupt)
        }

        let index_header = value_start + 16;
        let entries_offset = le_u32(&record[index_header..]) as usize;
        let total_size = le_u32(&record[index_header + 4..]) as usize;
        let position = index_header
            .checked_add(entries_offset)
            .ok_or(Error::Corrupt)?;
        let entries_end = index_header
            .checked_add(total_size)
            .ok_or(Error::Corrupt)?
            .min(value_end);
        if let Some(reference) =
            search_index_entries(record, position, entries_end, target)?
        {
            return Ok(reference)
        }

        let allocation = find_unnamed_attribute(record, ATTRIBUTE_INDEX_ALLOCATION)?
            .ok_or(Error::NotFound)?;
        if !allocation.non_resident {
            return Err(Error::Corrupt)
        }
        let runlist_offset = le_u16(&record[allocation.offset + 32..]) as usize;
        let runlist_start = allocation.offset
            .checked_add(runlist_offset)
            .ok_or(Error::Corrupt)?;
        let runlist_end = allocation.offset + allocation.length;
        if runlist_start >= runlist_end {
            return Err(Error::Corrupt)
        }
        let runlist_length = runlist_end - runlist_start;
        let mut runlist = [0u8; 512];
        if runlist_length > runlist.len() {
            return Err(Error::NotSupported)
        }
        runlist[..runlist_length].copy_from_slice(&record[runlist_start..runlist_end]);
        let allocation_size = le_u64(&record[allocation.offset + 48..]);

        let record_size = self.index_record_size as usize;
        let mut logical_offset = 0u64;
        while logical_offset < allocation_size {
            self.read_runs(
                device,
                &runlist[..runlist_length],
                logical_offset,
                &mut record[..record_size],
            )?;
            if &record[..4] == b"INDX" {
                apply_fixups(&mut record[..record_size], self.bytes_per_sector as usize)?;
                let index_header = 24usize;
                let entries_offset = le_u32(&record[index_header..]) as usize;
                let total_size = le_u32(&record[index_header + 4..]) as usize;
                let entries_start = index_header
                    .checked_add(entries_offset)
                    .ok_or(Error::Corrupt)?;
                let entries_end = index_header
                    .checked_add(total_size)
                    .ok_or(Error::Corrupt)?
                    .min(record_size);
                if let Some(reference) =
                    search_index_entries(record, entries_start, entries_end, target)?
                {
                    return Ok(reference)
                }
            }
            logical_offset = logical_offset
                .checked_add(self.index_record_size as u64)
                .ok_or(Error::InvalidOffset)?;
        }
        Err(Error::NotFound)
    }

    fn read_runs<D: ReadAt>(
        &self,
        device: &mut D,
        runlist: &[u8],
        offset: u64,
        output: &mut [u8],
    ) -> Result<(), Error> {
        let mut run_position = 0usize;
        let mut logical_start = 0u64;
        let mut previous_lcn = 0i64;
        let request_end = offset
            .checked_add(output.len() as u64)
            .ok_or(Error::InvalidOffset)?;

        while run_position < runlist.len() {
            let header = runlist[run_position];
            run_position += 1;
            if header == 0 {
                break
            }
            let length_bytes = (header & 0x0f) as usize;
            let offset_bytes = (header >> 4) as usize;
            if length_bytes == 0
                || length_bytes > 8
                || offset_bytes > 8
                || run_position + length_bytes + offset_bytes > runlist.len()
            {
                return Err(Error::Corrupt)
            }

            let cluster_count = unsigned_integer(
                &runlist[run_position..run_position + length_bytes],
            );
            run_position += length_bytes;
            let lcn_delta = if offset_bytes == 0 {
                None
            } else {
                let delta = signed_integer(
                    &runlist[run_position..run_position + offset_bytes],
                );
                run_position += offset_bytes;
                previous_lcn = previous_lcn.checked_add(delta).ok_or(Error::Corrupt)?;
                if previous_lcn < 0 {
                    return Err(Error::Corrupt)
                }
                Some(previous_lcn as u64)
            };

            let run_length = cluster_count
                .checked_mul(self.bytes_per_cluster as u64)
                .ok_or(Error::InvalidOffset)?;
            let logical_end = logical_start
                .checked_add(run_length)
                .ok_or(Error::InvalidOffset)?;
            let overlap_start = logical_start.max(offset);
            let overlap_end = logical_end.min(request_end);
            if overlap_start < overlap_end {
                let output_start = (overlap_start - offset) as usize;
                let take = (overlap_end - overlap_start) as usize;
                if let Some(lcn) = lcn_delta {
                    let disk_offset = self.partition_start
                        .checked_add(
                            lcn.checked_mul(self.bytes_per_cluster as u64)
                                .ok_or(Error::InvalidOffset)?,
                        )
                        .and_then(|base| base.checked_add(overlap_start - logical_start))
                        .ok_or(Error::InvalidOffset)?;
                    self.read_at(
                        device,
                        disk_offset,
                        &mut output[output_start..output_start + take],
                    )?
                } else {
                    output[output_start..output_start + take].fill(0)
                }
            }
            logical_start = logical_end;
            if logical_start >= request_end {
                return Ok(())
            }
        }

        if logical_start < request_end {
            Err(Error::Corrupt)
        } else {
            Ok(())
        }
    }

    fn require_scratch(&self, scratch: &[u8]) -> Result<(), Error> {
        if scratch.len() < self.file_record_size.max(self.index_record_size) as usize {
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

fn search_index_entries(
    record: &[u8],
    mut position: usize,
    entries_end: usize,
    target: &str,
) -> Result<Option<u64>, Error> {
    while position + 16 <= entries_end {
        let file_reference = le_u64(&record[position..]) & 0x0000_ffff_ffff_ffff;
        let entry_length = le_u16(&record[position + 8..]) as usize;
        let key_length = le_u16(&record[position + 10..]) as usize;
        let flags = le_u16(&record[position + 12..]);
        if entry_length < 16 || position + entry_length > entries_end {
            return Err(Error::Corrupt)
        }
        if flags & 2 != 0 {
            break
        }
        if key_length >= 66 && 16 + key_length <= entry_length {
            let key = position + 16;
            let name_length = record[key + 64] as usize;
            let name_start = key + 66;
            let name_end = name_start
                .checked_add(name_length * 2)
                .ok_or(Error::Corrupt)?;
            if name_end <= position + entry_length
                && utf16_bytes_match(&record[name_start..name_end], target)
            {
                return Ok(Some(file_reference))
            }
        }
        position += entry_length;
    }
    Ok(None)
}

fn find_unnamed_attribute(
    record: &[u8],
    wanted_type: u32,
) -> Result<Option<Attribute>, Error> {
    if record.len() < 24 {
        return Err(Error::Corrupt)
    }
    let bytes_in_use = le_u32(&record[24..]) as usize;
    let mut offset = le_u16(&record[20..]) as usize;
    if bytes_in_use > record.len() || offset >= bytes_in_use {
        return Err(Error::Corrupt)
    }

    while offset + 16 <= bytes_in_use {
        let attribute_type = le_u32(&record[offset..]);
        if attribute_type == ATTRIBUTE_END {
            return Ok(None)
        }
        let length = le_u32(&record[offset + 4..]) as usize;
        if length < 24 || offset + length > bytes_in_use {
            return Err(Error::Corrupt)
        }
        let name_length = record[offset + 9];
        if attribute_type == wanted_type && name_length == 0 {
            return Ok(Some(Attribute {
                offset,
                length,
                non_resident: record[offset + 8] != 0,
            }))
        }
        offset += length;
    }
    Err(Error::Corrupt)
}

fn apply_fixups(record: &mut [u8], bytes_per_sector: usize) -> Result<(), Error> {
    if bytes_per_sector < 2 || record.len() < 8 {
        return Err(Error::Corrupt)
    }
    let array_offset = le_u16(&record[4..]) as usize;
    let array_count = le_u16(&record[6..]) as usize;
    if array_count == 0
        || array_offset + array_count * 2 > record.len()
        || (array_count - 1) * bytes_per_sector > record.len()
    {
        return Err(Error::Corrupt)
    }
    let sequence = [record[array_offset], record[array_offset + 1]];
    for index in 1..array_count {
        let sector_end = index * bytes_per_sector;
        if record[sector_end - 2..sector_end] != sequence {
            return Err(Error::Corrupt)
        }
        let replacement = array_offset + index * 2;
        record[sector_end - 2] = record[replacement];
        record[sector_end - 1] = record[replacement + 1];
    }
    Ok(())
}

fn decode_record_size(encoded: i8, bytes_per_cluster: u32) -> Result<u32, Error> {
    if encoded > 0 {
        bytes_per_cluster
            .checked_mul(encoded as u32)
            .ok_or(Error::Corrupt)
    } else if encoded < 0 {
        1u32.checked_shl((-encoded) as u32).ok_or(Error::Corrupt)
    } else {
        Err(Error::Corrupt)
    }
}

fn utf16_bytes_match(name: &[u8], target: &str) -> bool {
    let mut target_chars = target.chars();
    for bytes in name.chunks_exact(2) {
        let Some(character) =
            char::from_u32(u16::from_le_bytes([bytes[0], bytes[1]]) as u32)
        else {
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

fn unsigned_integer(bytes: &[u8]) -> u64 {
    let mut value = 0u64;
    for (index, byte) in bytes.iter().enumerate() {
        value |= (*byte as u64) << (index * 8);
    }
    value
}

fn signed_integer(bytes: &[u8]) -> i64 {
    let mut value = unsigned_integer(bytes);
    if bytes.last().is_some_and(|byte| byte & 0x80 != 0) && bytes.len() < 8 {
        value |= u64::MAX << (bytes.len() * 8);
    }
    value as i64
}
