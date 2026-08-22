use super::volume::{self, Superblock};
use super::{BlockStore, Error, SynFs, BLOCK_SIZE};
use ghostos_durability::{CrashBoundary, CrashDomain, InterruptionInjector, NoInterruption};

const MIGRATION_MAGIC: &[u8; 8] = b"SYNFMIGR";
const MIGRATION_RECORD_VERSION: u16 = 1;
const MIGRATION_CHECKSUM_OFFSET: usize = BLOCK_SIZE - 8;
const MIGRATION_TOTAL_OFFSET: usize = 40;
const MIGRATION_SOURCE_BANK_OFFSET: usize = 12;
const MIGRATION_TARGET_BANK_OFFSET: usize = 13;
const MIGRATION_SOURCE_VERSION_OFFSET: usize = 14;
const MIGRATION_TARGET_VERSION_OFFSET: usize = 16;
const MIGRATION_SOURCE_SEQUENCE_OFFSET: usize = 24;
const MIGRATION_NEXT_BLOCK_OFFSET: usize = 32;

pub const MAX_MIGRATION_BLOCKS_PER_STEP: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackgroundIoLimit {
    pub max_blocks_per_step: usize,
}

impl BackgroundIoLimit {
    pub const fn new(max_blocks_per_step: usize) -> Self {
        Self { max_blocks_per_step }
    }

    fn effective(self) -> Result<usize, Error> {
        if self.max_blocks_per_step == 0 {
            return Err(Error::InvalidMigrationLimit)
        }
        Ok(self.max_blocks_per_step.min(MAX_MIGRATION_BLOCKS_PER_STEP))
    }
}

impl Default for BackgroundIoLimit {
    fn default() -> Self {
        Self::new(8)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FormatMigrationPhase {
    Copying = 1,
    ShadowValidation = 2,
    ReadyToCommit = 3,
    Committed = 4,
    RolledBack = 5,
}

impl FormatMigrationPhase {
    fn from_raw(raw: u8) -> Result<Self, Error> {
        match raw {
            1 => Ok(Self::Copying),
            2 => Ok(Self::ShadowValidation),
            3 => Ok(Self::ReadyToCommit),
            4 => Ok(Self::Committed),
            5 => Ok(Self::RolledBack),
            _ => Err(Error::Corrupt),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatMigrationProgress {
    pub source_format: u16,
    pub target_format: u16,
    pub blocks_copied: usize,
    pub total_blocks: usize,
    pub io_blocks: usize,
    pub phase: FormatMigrationPhase,
}

#[derive(Clone, Copy)]
struct MigrationRecord {
    phase: FormatMigrationPhase,
    source_bank: usize,
    target_bank: usize,
    source_version: u16,
    target_version: u16,
    source_sequence: u64,
    next_block: usize,
}

pub struct FormatMigration<const MAX_BLOCKS: usize> {
    record: MigrationRecord,
}

impl<const MAX_BLOCKS: usize> SynFs<MAX_BLOCKS> {
    pub fn begin_format_migration(
        image: &mut [u8],
        target_version: u16,
    ) -> Result<FormatMigration<MAX_BLOCKS>, Error> {
        let source = Self::load(image)?;
        validate_target(source.format_version(), target_version)?;
        let target_bank = 1 - source.volume_bank;
        if read_migration_record::<MAX_BLOCKS>(image, target_bank)?.is_some() {
            return Err(Error::MigrationInProgress)
        }
        let migration = FormatMigration {
            record: MigrationRecord {
                phase: FormatMigrationPhase::Copying,
                source_bank: source.volume_bank,
                target_bank,
                source_version: source.format_version(),
                target_version,
                source_sequence: source.volume_sequence,
                next_block: 0,
            },
        };
        write_migration_record::<MAX_BLOCKS>(image, migration.record)?;
        Ok(migration)
    }

    pub fn resume_format_migration(
        image: &mut [u8],
    ) -> Result<Option<FormatMigration<MAX_BLOCKS>>, Error> {
        let source = Self::load(image)?;
        let mut found = None;
        for bank in 0..2 {
            let Some(record) = read_migration_record::<MAX_BLOCKS>(image, bank)? else {
                continue
            };
            if record.source_bank == source.volume_bank
                && record.source_sequence == source.volume_sequence
                && record.target_bank == bank
                && record.source_version == source.format_version()
                && record.target_version == volume::VOLUME_FORMAT_VERSION
            {
                if found.is_some() {
                    return Err(Error::Corrupt)
                }
                found = Some(FormatMigration { record })
            }
        }
        Ok(found)
    }

    pub fn begin_format_migration_to_device<D: BlockStore>(
        image: &mut [u8],
        device: &mut D,
        target_version: u16,
    ) -> Result<FormatMigration<MAX_BLOCKS>, Error> {
        Self::load_from_device(image, device)?;
        let migration = Self::begin_format_migration(image, target_version)?;
        migration.write_record_to_device(image, device)?;
        Ok(migration)
    }
}

impl<const MAX_BLOCKS: usize> FormatMigration<MAX_BLOCKS> {
    pub const fn progress(&self) -> FormatMigrationProgress {
        FormatMigrationProgress {
            source_format: self.record.source_version,
            target_format: self.record.target_version,
            blocks_copied: self.record.next_block,
            total_blocks: MAX_BLOCKS + 1,
            io_blocks: 0,
            phase: self.record.phase,
        }
    }

    pub fn step(
        &mut self,
        image: &mut [u8],
        limit: BackgroundIoLimit,
    ) -> Result<FormatMigrationProgress, Error> {
        let mut no_interruption = NoInterruption;
        self.step_with_interruption(image, limit, &mut no_interruption)
    }

    pub fn step_with_interruption<I: InterruptionInjector>(
        &mut self,
        image: &mut [u8],
        limit: BackgroundIoLimit,
        injector: &mut I,
    ) -> Result<FormatMigrationProgress, Error> {
        require_image_size::<MAX_BLOCKS>(image)?;
        let io_limit = limit.effective()?;
        if self.record.phase != FormatMigrationPhase::RolledBack {
            self.verify_source(image)?;
        }
        match self.record.phase {
            FormatMigrationPhase::Copying => {
                if self.record.next_block < MAX_BLOCKS + 1 {
                    let count = io_limit.min(MAX_BLOCKS + 1 - self.record.next_block);
                    for _ in 0..count {
                        copy_block::<MAX_BLOCKS>(
                            image,
                            self.record.source_bank,
                            self.record.target_bank,
                            self.record.next_block,
                        )?;
                        self.record.next_block += 1;
                    }
                    write_migration_record::<MAX_BLOCKS>(image, self.record)?;
                    if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::JournalRecord) {
                        return Err(Error::Interrupted)
                    }
                    return Ok(self.progress_with_io(count))
                }
                self.record.phase = FormatMigrationPhase::ShadowValidation;
                write_migration_record::<MAX_BLOCKS>(image, self.record)?;
                if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::JournalRecord) {
                    return Err(Error::Interrupted)
                }
            }
            FormatMigrationPhase::ShadowValidation => {
                self.shadow_validate(image)?;
                self.record.phase = FormatMigrationPhase::ReadyToCommit;
                write_migration_record::<MAX_BLOCKS>(image, self.record)?;
                if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::ManifestSlot) {
                    return Err(Error::Interrupted)
                }
            }
            FormatMigrationPhase::ReadyToCommit => {
                self.commit_image(image)?;
                self.record.phase = FormatMigrationPhase::Committed;
                if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::Flush) {
                    return Err(Error::Interrupted)
                }
            }
            FormatMigrationPhase::Committed | FormatMigrationPhase::RolledBack => {
                return Ok(self.progress_with_io(0))
            }
        }
        Ok(self.progress_with_io(0))
    }

    pub fn step_to_device<D: BlockStore>(
        &mut self,
        image: &mut [u8],
        device: &mut D,
        limit: BackgroundIoLimit,
    ) -> Result<FormatMigrationProgress, Error> {
        let mut no_interruption = NoInterruption;
        self.step_to_device_with_interruption(image, device, limit, &mut no_interruption)
    }

    pub fn step_to_device_with_interruption<D: BlockStore, I: InterruptionInjector>(
        &mut self,
        image: &mut [u8],
        device: &mut D,
        limit: BackgroundIoLimit,
        injector: &mut I,
    ) -> Result<FormatMigrationProgress, Error> {
        require_image_size::<MAX_BLOCKS>(image)?;
        let io_limit = limit.effective()?;
        if self.record.phase != FormatMigrationPhase::RolledBack {
            self.verify_source(image)?;
        }
        match self.record.phase {
            FormatMigrationPhase::Copying => {
                if self.record.next_block < MAX_BLOCKS + 1 {
                    let count = io_limit.min(MAX_BLOCKS + 1 - self.record.next_block);
                    for _ in 0..count {
                        let progress = self.record.next_block;
                        copy_block::<MAX_BLOCKS>(
                            image,
                            self.record.source_bank,
                            self.record.target_bank,
                            progress,
                        )?;
                        write_progress_block_to_device::<MAX_BLOCKS, D>(
                            image,
                            device,
                            self.record.target_bank,
                            progress,
                        )?;
                        self.record.next_block += 1;
                    }
                    self.write_record_to_device(image, device)?;
                    if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::JournalRecord) {
                        return Err(Error::Interrupted)
                    }
                    return Ok(self.progress_with_io(count))
                }
                self.record.phase = FormatMigrationPhase::ShadowValidation;
                self.write_record_to_device(image, device)?;
                if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::JournalRecord) {
                    return Err(Error::Interrupted)
                }
            }
            FormatMigrationPhase::ShadowValidation => {
                self.shadow_validate(image)?;
                self.record.phase = FormatMigrationPhase::ReadyToCommit;
                self.write_record_to_device(image, device)?;
                if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::ManifestSlot) {
                    return Err(Error::Interrupted)
                }
            }
            FormatMigrationPhase::ReadyToCommit => {
                self.commit_image(image)?;
                let start = volume::generation_offset::<MAX_BLOCKS>(self.record.target_bank);
                device
                    .write_block(
                        (start / BLOCK_SIZE) as u64,
                        image.get(start..start + BLOCK_SIZE).ok_or(Error::Corrupt)?,
                    )
                    .map_err(|_| Error::Io)?;
                device.flush().map_err(|_| Error::Io)?;
                self.record.phase = FormatMigrationPhase::Committed;
                if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::Flush) {
                    return Err(Error::Interrupted)
                }
            }
            FormatMigrationPhase::Committed | FormatMigrationPhase::RolledBack => {
                return Ok(self.progress_with_io(0))
            }
        }
        Ok(self.progress_with_io(0))
    }

    pub fn rollback(&mut self, image: &mut [u8]) -> Result<(), Error> {
        let mut no_interruption = NoInterruption;
        self.rollback_with_interruption(image, &mut no_interruption)
    }

    pub fn rollback_with_interruption<I: InterruptionInjector>(
        &mut self,
        image: &mut [u8],
        injector: &mut I,
    ) -> Result<(), Error> {
        require_image_size::<MAX_BLOCKS>(image)?;
        match self.record.phase {
            FormatMigrationPhase::Committed => {
                let mut source = read_source_superblock::<MAX_BLOCKS>(image, self.record)?;
                source.sequence = source
                    .sequence
                    .checked_add(2)
                    .ok_or(Error::VersionOverflow)?;
                volume::write_superblock::<MAX_BLOCKS>(image, self.record.source_bank, source)?;
                if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::Flush) {
                    return Err(Error::Interrupted)
                }
            }
            FormatMigrationPhase::Copying
            | FormatMigrationPhase::ShadowValidation
            | FormatMigrationPhase::ReadyToCommit => {
                let start = volume::generation_offset::<MAX_BLOCKS>(self.record.target_bank);
                image
                    .get_mut(start..start + BLOCK_SIZE)
                    .ok_or(Error::Corrupt)?
                    .fill(0);
            }
            FormatMigrationPhase::RolledBack => return Ok(()),
        }
        self.record.phase = FormatMigrationPhase::RolledBack;
        Ok(())
    }

    pub fn rollback_to_device<D: BlockStore>(
        &mut self,
        image: &mut [u8],
        device: &mut D,
    ) -> Result<(), Error> {
        let mut no_interruption = NoInterruption;
        self.rollback_to_device_with_interruption(image, device, &mut no_interruption)
    }

    pub fn rollback_to_device_with_interruption<D: BlockStore, I: InterruptionInjector>(
        &mut self,
        image: &mut [u8],
        device: &mut D,
        injector: &mut I,
    ) -> Result<(), Error> {
        require_image_size::<MAX_BLOCKS>(image)?;
        match self.record.phase {
            FormatMigrationPhase::Committed => {
                let mut source = read_source_superblock::<MAX_BLOCKS>(image, self.record)?;
                source.sequence = source
                    .sequence
                    .checked_add(2)
                    .ok_or(Error::VersionOverflow)?;
                volume::write_superblock::<MAX_BLOCKS>(image, self.record.source_bank, source)?;
                let start = volume::generation_offset::<MAX_BLOCKS>(self.record.source_bank);
                device
                    .write_block(
                        (start / BLOCK_SIZE) as u64,
                        image.get(start..start + BLOCK_SIZE).ok_or(Error::Corrupt)?,
                    )
                    .map_err(|_| Error::Io)?;
                device.flush().map_err(|_| Error::Io)?;
                if injector.checkpoint(CrashDomain::SynFs, CrashBoundary::Flush) {
                    return Err(Error::Interrupted)
                }
            }
            FormatMigrationPhase::Copying
            | FormatMigrationPhase::ShadowValidation
            | FormatMigrationPhase::ReadyToCommit => {
                let start = volume::generation_offset::<MAX_BLOCKS>(self.record.target_bank);
                let block = image
                    .get_mut(start..start + BLOCK_SIZE)
                    .ok_or(Error::Corrupt)?;
                block.fill(0);
                device
                    .write_block((start / BLOCK_SIZE) as u64, block)
                    .map_err(|_| Error::Io)?;
                device.flush().map_err(|_| Error::Io)?;
            }
            FormatMigrationPhase::RolledBack => return Ok(()),
        }
        self.record.phase = FormatMigrationPhase::RolledBack;
        Ok(())
    }

    fn write_record_to_device<D: BlockStore>(
        &self,
        image: &mut [u8],
        device: &mut D,
    ) -> Result<(), Error> {
        write_migration_record::<MAX_BLOCKS>(image, self.record)?;
        let start = volume::generation_offset::<MAX_BLOCKS>(self.record.target_bank);
        device
            .write_block(
                (start / BLOCK_SIZE) as u64,
                image.get(start..start + BLOCK_SIZE).ok_or(Error::Corrupt)?,
            )
            .map_err(|_| Error::Io)?;
        device.flush().map_err(|_| Error::Io)
    }

    fn verify_source(&self, image: &[u8]) -> Result<Superblock, Error> {
        let source = read_source_superblock::<MAX_BLOCKS>(image, self.record)?;
        if source.format_version != self.record.source_version {
            return Err(Error::MigrationConflict)
        }
        Ok(source)
    }

    fn shadow_validate(&self, image: &[u8]) -> Result<(), Error> {
        let mut target = self.verify_source(image)?;
        target.sequence = target
            .sequence
            .checked_add(1)
            .ok_or(Error::VersionOverflow)?;
        target.format_version = self.record.target_version;
        volume::load_bank::<MAX_BLOCKS>(image, self.record.target_bank, target).map(|_| ())
    }

    fn commit_image(&self, image: &mut [u8]) -> Result<(), Error> {
        let mut target = self.verify_source(image)?;
        target.sequence = target
            .sequence
            .checked_add(1)
            .ok_or(Error::VersionOverflow)?;
        target.format_version = self.record.target_version;
        volume::write_superblock::<MAX_BLOCKS>(image, self.record.target_bank, target)
    }

    fn progress_with_io(&self, io_blocks: usize) -> FormatMigrationProgress {
        FormatMigrationProgress {
            io_blocks,
            ..self.progress()
        }
    }
}

fn validate_target(source_version: u16, target_version: u16) -> Result<(), Error> {
    if target_version < source_version {
        return Err(Error::DowngradeRefused)
    }
    if target_version == source_version {
        return Err(Error::MigrationAlreadyCurrent)
    }
    if target_version != volume::VOLUME_FORMAT_VERSION
        || !volume::supported_volume_format(source_version)
    {
        return Err(Error::UnsupportedMigration)
    }
    Ok(())
}

fn require_image_size<const MAX_BLOCKS: usize>(image: &[u8]) -> Result<(), Error> {
    if image.len() < SynFs::<MAX_BLOCKS>::volume_bytes() {
        return Err(Error::BufferTooSmall {
            required: SynFs::<MAX_BLOCKS>::volume_bytes(),
        })
    }
    Ok(())
}

fn read_source_superblock<const MAX_BLOCKS: usize>(
    image: &[u8],
    record: MigrationRecord,
) -> Result<Superblock, Error> {
    volume::read_superblock::<MAX_BLOCKS>(image, record.source_bank)?
        .filter(|superblock| superblock.sequence == record.source_sequence)
        .ok_or(Error::MigrationConflict)
}

fn copy_block<const MAX_BLOCKS: usize>(
    image: &mut [u8],
    source_bank: usize,
    target_bank: usize,
    progress: usize,
) -> Result<(), Error> {
    let source_start = if progress == 0 {
        volume::generation_offset::<MAX_BLOCKS>(source_bank) + BLOCK_SIZE
    } else {
        volume::generation_offset::<MAX_BLOCKS>(source_bank) + (progress + 1) * BLOCK_SIZE
    };
    let target_start = if progress == 0 {
        volume::generation_offset::<MAX_BLOCKS>(target_bank) + BLOCK_SIZE
    } else {
        volume::generation_offset::<MAX_BLOCKS>(target_bank) + (progress + 1) * BLOCK_SIZE
    };
    let mut block = [0; BLOCK_SIZE];
    block.copy_from_slice(image.get(source_start..source_start + BLOCK_SIZE).ok_or(Error::Corrupt)?);
    image
        .get_mut(target_start..target_start + BLOCK_SIZE)
        .ok_or(Error::Corrupt)?
        .copy_from_slice(&block);
    Ok(())
}

fn write_progress_block_to_device<const MAX_BLOCKS: usize, D: BlockStore>(
    image: &[u8],
    device: &mut D,
    target_bank: usize,
    progress: usize,
) -> Result<(), Error> {
    let start = if progress == 0 {
        volume::generation_offset::<MAX_BLOCKS>(target_bank) + BLOCK_SIZE
    } else {
        volume::generation_offset::<MAX_BLOCKS>(target_bank) + (progress + 1) * BLOCK_SIZE
    };
    device
        .write_block(
            (start / BLOCK_SIZE) as u64,
            image.get(start..start + BLOCK_SIZE).ok_or(Error::Corrupt)?,
        )
        .map_err(|_| Error::Io)
}

fn write_migration_record<const MAX_BLOCKS: usize>(
    image: &mut [u8],
    record: MigrationRecord,
) -> Result<(), Error> {
    let start = volume::generation_offset::<MAX_BLOCKS>(record.target_bank);
    let block = image
        .get_mut(start..start + BLOCK_SIZE)
        .ok_or(Error::Corrupt)?;
    block.fill(0);
    block[..8].copy_from_slice(MIGRATION_MAGIC);
    put_u16(block, 8, MIGRATION_RECORD_VERSION);
    block[10] = record.phase as u8;
    block[MIGRATION_SOURCE_BANK_OFFSET] = record.source_bank as u8;
    block[MIGRATION_TARGET_BANK_OFFSET] = record.target_bank as u8;
    put_u16(block, MIGRATION_SOURCE_VERSION_OFFSET, record.source_version);
    put_u16(block, MIGRATION_TARGET_VERSION_OFFSET, record.target_version);
    put_u64(block, MIGRATION_SOURCE_SEQUENCE_OFFSET, record.source_sequence);
    put_u64(block, MIGRATION_NEXT_BLOCK_OFFSET, record.next_block as u64);
    put_u64(block, MIGRATION_TOTAL_OFFSET, (MAX_BLOCKS + 1) as u64);
    put_u64(block, MIGRATION_CHECKSUM_OFFSET, checksum(&block[..MIGRATION_CHECKSUM_OFFSET]));
    Ok(())
}

fn read_migration_record<const MAX_BLOCKS: usize>(
    image: &[u8],
    bank: usize,
) -> Result<Option<MigrationRecord>, Error> {
    let start = volume::generation_offset::<MAX_BLOCKS>(bank);
    let block = image.get(start..start + BLOCK_SIZE).ok_or(Error::Corrupt)?;
    if &block[..8] != MIGRATION_MAGIC {
        return Ok(None)
    }
    if u16_at(block, 8) != MIGRATION_RECORD_VERSION
        || u64_at(block, MIGRATION_CHECKSUM_OFFSET)
            != checksum(&block[..MIGRATION_CHECKSUM_OFFSET])
        || block[MIGRATION_SOURCE_BANK_OFFSET] > 1
        || block[MIGRATION_TARGET_BANK_OFFSET] > 1
        || block[MIGRATION_SOURCE_BANK_OFFSET] == block[MIGRATION_TARGET_BANK_OFFSET]
        || u64_at(block, MIGRATION_TOTAL_OFFSET) != (MAX_BLOCKS + 1) as u64
        || u64_at(block, MIGRATION_NEXT_BLOCK_OFFSET) > u64_at(block, MIGRATION_TOTAL_OFFSET)
    {
        return Ok(None)
    }
    Ok(Some(MigrationRecord {
        phase: FormatMigrationPhase::from_raw(block[10])?,
        source_bank: block[MIGRATION_SOURCE_BANK_OFFSET] as usize,
        target_bank: block[MIGRATION_TARGET_BANK_OFFSET] as usize,
        source_version: u16_at(block, MIGRATION_SOURCE_VERSION_OFFSET),
        target_version: u16_at(block, MIGRATION_TARGET_VERSION_OFFSET),
        source_sequence: u64_at(block, MIGRATION_SOURCE_SEQUENCE_OFFSET),
        next_block: u64_at(block, MIGRATION_NEXT_BLOCK_OFFSET) as usize,
    }))
}

fn put_u16(block: &mut [u8], offset: usize, value: u16) {
    block[offset..offset + 2].copy_from_slice(&value.to_le_bytes())
}

fn put_u64(block: &mut [u8], offset: usize, value: u64) {
    block[offset..offset + 8].copy_from_slice(&value.to_le_bytes())
}

fn u16_at(block: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([block[offset], block[offset + 1]])
}

fn u64_at(block: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        block[offset],
        block[offset + 1],
        block[offset + 2],
        block[offset + 3],
        block[offset + 4],
        block[offset + 5],
        block[offset + 6],
        block[offset + 7],
    ])
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3)
    }
    hash
}
