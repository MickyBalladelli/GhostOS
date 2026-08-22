use crate::crypto::{
    CryptoError, Digest, EncryptionKey, CHUNK_BYTES, ENCRYPTED_CHUNK_OVERHEAD,
    MAX_ENCRYPTED_CHUNK_BYTES,
};
use ghostos_status::{IntoStatus, Status};
use ghostos_ghostfs::{Error as FileError, FileName, FileType, FileVersion, RmsMapHandle, SynFs};

pub const MAX_RESTORE_FILE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackupId(u64);

impl BackupId {
    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkRef {
    pub object_id: Digest,
    pub plaintext_digest: Digest,
    pub length: u32,
    pub source_file_index: u32,
    pub source_offset: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestFile {
    pub file: FileVersion,
    pub first_chunk: u32,
    pub chunk_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackupManifest<const MAX_FILES: usize, const MAX_CHUNKS: usize> {
    pub id: BackupId,
    pub generation: u64,
    pub base: Option<BackupId>,
    pub file_count: u32,
    pub chunk_count: u32,
    pub complete: bool,
    pub files: [Option<ManifestFile>; MAX_FILES],
    pub chunks: [Option<ChunkRef>; MAX_CHUNKS],
}

impl<const MAX_FILES: usize, const MAX_CHUNKS: usize>
    BackupManifest<MAX_FILES, MAX_CHUNKS>
{
    fn new(id: BackupId, generation: u64, base: Option<BackupId>) -> Self {
        Self {
            id,
            generation,
            base,
            file_count: 0,
            chunk_count: 0,
            complete: false,
            files: [None; MAX_FILES],
            chunks: [None; MAX_CHUNKS],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionPolicy {
    pub keep_last: u16,
    pub max_age_generations: u64,
}

impl RetentionPolicy {
    pub const fn new(keep_last: u16, max_age_generations: u64) -> Self {
        Self {
            keep_last,
            max_age_generations,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ManifestSlot<const MAX_FILES: usize, const MAX_CHUNKS: usize> {
    manifest: BackupManifest<MAX_FILES, MAX_CHUNKS>,
    legal_hold: bool,
}

pub struct BackupCatalog<const MAX_BACKUPS: usize, const MAX_FILES: usize, const MAX_CHUNKS: usize> {
    slots: [Option<ManifestSlot<MAX_FILES, MAX_CHUNKS>>; MAX_BACKUPS],
    policy: RetentionPolicy,
}

impl<const MAX_BACKUPS: usize, const MAX_FILES: usize, const MAX_CHUNKS: usize> Default
    for BackupCatalog<MAX_BACKUPS, MAX_FILES, MAX_CHUNKS>
{
    fn default() -> Self {
        Self::new(RetentionPolicy::new(1, 0))
    }
}

impl<const MAX_BACKUPS: usize, const MAX_FILES: usize, const MAX_CHUNKS: usize>
    BackupCatalog<MAX_BACKUPS, MAX_FILES, MAX_CHUNKS>
{
    pub const fn new(policy: RetentionPolicy) -> Self {
        Self {
            slots: [None; MAX_BACKUPS],
            policy,
        }
    }

    pub const fn policy(&self) -> RetentionPolicy {
        self.policy
    }

    pub fn latest(&self) -> Option<&BackupManifest<MAX_FILES, MAX_CHUNKS>> {
        self.slots
            .iter()
            .flatten()
            .map(|slot| &slot.manifest)
            .filter(|manifest| manifest.complete)
            .max_by_key(|manifest| (manifest.generation, manifest.id.raw()))
    }

    pub fn manifest(&self, id: BackupId) -> Option<&BackupManifest<MAX_FILES, MAX_CHUNKS>> {
        self.slots
            .iter()
            .flatten()
            .find(|slot| slot.manifest.id == id)
            .map(|slot| &slot.manifest)
    }

    pub fn record(
        &mut self,
        manifest: BackupManifest<MAX_FILES, MAX_CHUNKS>,
    ) -> Result<(), RecoveryError> {
        if !manifest.complete {
            return Err(RecoveryError::ManifestIncomplete)
        }
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(RecoveryError::CatalogFull)?;
        *slot = Some(ManifestSlot {
            manifest,
            legal_hold: false,
        });
        Ok(())
    }

    pub fn set_legal_hold(&mut self, id: BackupId, held: bool) -> Result<(), RecoveryError> {
        let slot = self
            .slots
            .iter_mut()
            .flatten()
            .find(|slot| slot.manifest.id == id)
            .ok_or(RecoveryError::ManifestNotFound)?;
        slot.legal_hold = held;
        Ok(())
    }

    pub fn legal_hold(&self, id: BackupId) -> Option<bool> {
        self.slots
            .iter()
            .flatten()
            .find(|slot| slot.manifest.id == id)
            .map(|slot| slot.legal_hold)
    }

    pub fn object_known(&self, object_id: Digest) -> bool {
        self.slots.iter().flatten().any(|slot| {
            slot.manifest.chunks[..slot.manifest.chunk_count as usize]
                .iter()
                .flatten()
                .any(|chunk| chunk.object_id == object_id)
        })
    }

    pub fn prune(&mut self, current_generation: u64) -> usize {
        let mut removed = 0;
        for index in 0..MAX_BACKUPS {
            let Some(slot) = self.slots[index] else { continue };
            if slot.legal_hold || !slot.manifest.complete {
                continue
            }
            let newer = self
                .slots
                .iter()
                .flatten()
                .filter(|other| {
                    other.manifest.complete
                        && (other.manifest.generation, other.manifest.id.raw())
                            > (slot.manifest.generation, slot.manifest.id.raw())
                })
                .count();
            let outside_age = self.policy.max_age_generations != 0
                && current_generation.saturating_sub(slot.manifest.generation)
                    > self.policy.max_age_generations;
            if newer >= usize::from(self.policy.keep_last) || outside_age {
                self.slots[index] = None;
                removed += 1;
            }
        }
        removed
    }
}

pub trait ResumableObjectUploader {
    fn begin_object(&mut self, object_id: Digest, total_bytes: u64) -> Result<u64, Status>;

    /// Return the next durable offset. A retry must use the returned offset.
    fn append_object(
        &mut self,
        object_id: Digest,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, Status>;

    fn finish_object(&mut self, object_id: Digest, total_bytes: u64) -> Result<(), Status>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackupProgress {
    pub state: BackupUploadState,
    pub chunks_scanned: u32,
    pub objects_uploaded: u32,
    pub bytes_transferred: u64,
    pub bytes_resumed: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupUploadState {
    Uploading,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryBackupReport<const MAX_FILES: usize, const MAX_CHUNKS: usize> {
    pub manifest: BackupManifest<MAX_FILES, MAX_CHUNKS>,
    pub objects_uploaded: u32,
    pub bytes_transferred: u64,
    pub bytes_resumed: u64,
    pub rpo_generations: u64,
}

pub struct EncryptedBackupJob<const MAX_BLOCKS: usize, const MAX_FILES: usize, const MAX_CHUNKS: usize> {
    checkpoint: ghostos_ghostfs::CheckpointInfo,
    capability: RmsMapHandle,
    key: EncryptionKey,
    manifest: BackupManifest<MAX_FILES, MAX_CHUNKS>,
    upload: [bool; MAX_CHUNKS],
    chunk_index: u32,
    wire: [u8; MAX_ENCRYPTED_CHUNK_BYTES],
    wire_length: usize,
    wire_offset: u64,
    wire_started: bool,
    objects_uploaded: u32,
    bytes_transferred: u64,
    bytes_resumed: u64,
    released: bool,
}

pub trait BackupCatalogView<const MAX_FILES: usize, const MAX_CHUNKS: usize> {
    fn latest_manifest(&self) -> Option<&BackupManifest<MAX_FILES, MAX_CHUNKS>>;
    fn object_known(&self, object_id: Digest) -> bool;
}

impl<const MAX_BACKUPS: usize, const MAX_FILES: usize, const MAX_CHUNKS: usize>
    BackupCatalogView<MAX_FILES, MAX_CHUNKS>
    for BackupCatalog<MAX_BACKUPS, MAX_FILES, MAX_CHUNKS>
{
    fn latest_manifest(&self) -> Option<&BackupManifest<MAX_FILES, MAX_CHUNKS>> {
        self.latest()
    }

    fn object_known(&self, object_id: Digest) -> bool {
        self.object_known(object_id)
    }
}

impl<const MAX_BLOCKS: usize, const MAX_FILES: usize, const MAX_CHUNKS: usize>
    EncryptedBackupJob<MAX_BLOCKS, MAX_FILES, MAX_CHUNKS>
{
    pub fn begin<C: BackupCatalogView<MAX_FILES, MAX_CHUNKS>>(
        filesystem: &mut SynFs<MAX_BLOCKS>,
        capability: RmsMapHandle,
        key: EncryptionKey,
        catalog: &C,
    ) -> Result<Self, RecoveryError> {
        let checkpoint = filesystem.create_checkpoint()?;
        let result = Self::build(filesystem, checkpoint, capability, key, catalog);
        if result.is_err() {
            let _ = filesystem.release_checkpoint(checkpoint.id);
        }
        result
    }

    pub fn start<C: BackupCatalogView<MAX_FILES, MAX_CHUNKS>>(
        filesystem: &mut SynFs<MAX_BLOCKS>,
        capability: RmsMapHandle,
        key: EncryptionKey,
        catalog: &C,
    ) -> Result<Self, RecoveryError> {
        Self::begin(filesystem, capability, key, catalog)
    }

    fn build<C: BackupCatalogView<MAX_FILES, MAX_CHUNKS>>(
        filesystem: &SynFs<MAX_BLOCKS>,
        checkpoint: ghostos_ghostfs::CheckpointInfo,
        capability: RmsMapHandle,
        key: EncryptionKey,
        catalog: &C,
    ) -> Result<Self, RecoveryError> {
        let snapshot = filesystem.checkpoint_snapshot(checkpoint.id, capability)?;
        let id = BackupId::from_raw(checkpoint.id.raw()).ok_or(RecoveryError::InvalidState)?;
        let mut manifest = BackupManifest::new(
            id,
            checkpoint.generation,
            catalog.latest_manifest().map(|manifest| manifest.id),
        );
        let mut upload = [false; MAX_CHUNKS];
        let mut file_index = 0u32;
        while let Some(file) = snapshot.file_at(file_index)? {
            let file_slot = manifest.file_count as usize;
            if file_slot == MAX_FILES {
                return Err(RecoveryError::Capacity)
            }
            let first_chunk = manifest.chunk_count;
            if file.file_type == FileType::Regular {
                let mut source_offset = 0u64;
                while source_offset < file.size {
                    let amount = usize::try_from(
                        file.size.saturating_sub(source_offset).min(CHUNK_BYTES as u64),
                    )
                    .map_err(|_| RecoveryError::ObjectTooLarge)?;
                    let mut bytes = [0; CHUNK_BYTES];
                    let copied = snapshot.read_file_at(
                        file_index,
                        source_offset,
                        &mut bytes[..amount],
                    )?;
                    if copied != amount {
                        return Err(RecoveryError::File(FileError::Corrupt))
                    }
                    let plaintext_digest = Digest::hash(&bytes[..amount]);
                    let chunk = ChunkRef {
                        object_id: key.object_id(plaintext_digest),
                        plaintext_digest,
                        length: amount as u32,
                        source_file_index: file_index,
                        source_offset,
                    };
                    let chunk_slot = manifest.chunk_count as usize;
                    if chunk_slot == MAX_CHUNKS {
                        return Err(RecoveryError::Capacity)
                    }
                    let already_in_plan = manifest.chunks[..chunk_slot]
                        .iter()
                        .flatten()
                        .any(|existing| existing.object_id == chunk.object_id);
                    upload[chunk_slot] =
                        !catalog.object_known(chunk.object_id) && !already_in_plan;
                    manifest.chunks[chunk_slot] = Some(chunk);
                    manifest.chunk_count += 1;
                    source_offset = source_offset.saturating_add(amount as u64);
                }
            }
            manifest.files[file_slot] = Some(ManifestFile {
                file,
                first_chunk,
                chunk_count: manifest.chunk_count.saturating_sub(first_chunk),
            });
            manifest.file_count += 1;
            file_index = file_index.saturating_add(1);
        }
        Ok(Self {
            checkpoint,
            capability,
            key,
            manifest,
            upload,
            chunk_index: 0,
            wire: [0; MAX_ENCRYPTED_CHUNK_BYTES],
            wire_length: 0,
            wire_offset: 0,
            wire_started: false,
            objects_uploaded: 0,
            bytes_transferred: 0,
            bytes_resumed: 0,
            released: false,
        })
    }

    pub const fn manifest(&self) -> &BackupManifest<MAX_FILES, MAX_CHUNKS> {
        &self.manifest
    }

    pub fn poll(
        &mut self,
        filesystem: &SynFs<MAX_BLOCKS>,
        uploader: &mut impl ResumableObjectUploader,
        byte_budget: usize,
    ) -> Result<BackupProgress, RecoveryError> {
        if byte_budget == 0 {
            return Err(RecoveryError::InvalidBudget)
        }
        let snapshot = filesystem.checkpoint_snapshot(self.checkpoint.id, self.capability)?;
        let mut remaining = byte_budget;
        while remaining != 0 && self.chunk_index < self.manifest.chunk_count {
            let chunk = self.manifest.chunks[self.chunk_index as usize]
                .ok_or(RecoveryError::ManifestIncomplete)?;
            if !self.upload[self.chunk_index as usize] {
                self.chunk_index += 1;
                continue
            }
            if !self.wire_started {
                let mut plaintext = [0; CHUNK_BYTES];
                let length = usize::try_from(chunk.length)
                    .map_err(|_| RecoveryError::ObjectTooLarge)?;
                let copied = snapshot.read_file_at(
                    chunk.source_file_index,
                    chunk.source_offset,
                    &mut plaintext[..length],
                )?;
                if copied != length || Digest::hash(&plaintext[..length]) != chunk.plaintext_digest {
                    return Err(RecoveryError::File(FileError::Corrupt))
                }
                self.wire_length = self.key.encrypt(
                    chunk.plaintext_digest,
                    &plaintext[..length],
                    &mut self.wire,
                )?;
                self.wire_offset = uploader
                    .begin_object(chunk.object_id, self.wire_length as u64)
                    .map_err(RecoveryError::Upload)?;
                if self.wire_offset > self.wire_length as u64 {
                    return Err(RecoveryError::InvalidResumeOffset)
                }
                self.bytes_resumed = self.bytes_resumed.saturating_add(self.wire_offset);
                self.wire_started = true;
            }
            if self.wire_offset == self.wire_length as u64 {
                uploader
                    .finish_object(chunk.object_id, self.wire_length as u64)
                    .map_err(RecoveryError::Upload)?;
                self.objects_uploaded = self.objects_uploaded.saturating_add(1);
                self.chunk_index += 1;
                self.wire_started = false;
                continue
            }
            let amount = remaining.min(self.wire_length - self.wire_offset as usize);
            let next_offset = uploader
                .append_object(
                    chunk.object_id,
                    self.wire_offset,
                    &self.wire[self.wire_offset as usize..self.wire_offset as usize + amount],
                )
                .map_err(RecoveryError::Upload)?;
            if next_offset <= self.wire_offset
                || next_offset > self.wire_length as u64
                || next_offset > self.wire_offset.saturating_add(amount as u64)
            {
                return Err(RecoveryError::InvalidResumeOffset)
            }
            let accepted = next_offset - self.wire_offset;
            self.wire_offset = next_offset;
            self.bytes_transferred = self.bytes_transferred.saturating_add(accepted);
            remaining = remaining.saturating_sub(accepted as usize);
        }
        if self.chunk_index == self.manifest.chunk_count {
            self.manifest.complete = true;
        }
        Ok(self.progress())
    }

    pub const fn progress(&self) -> BackupProgress {
        BackupProgress {
            state: if self.chunk_index == self.manifest.chunk_count {
                BackupUploadState::Complete
            } else {
                BackupUploadState::Uploading
            },
            chunks_scanned: self.chunk_index,
            objects_uploaded: self.objects_uploaded,
            bytes_transferred: self.bytes_transferred,
            bytes_resumed: self.bytes_resumed,
        }
    }

    pub fn finish<const MAX_BACKUPS: usize>(
        &mut self,
        filesystem: &mut SynFs<MAX_BLOCKS>,
        catalog: &mut BackupCatalog<MAX_BACKUPS, MAX_FILES, MAX_CHUNKS>,
    ) -> Result<RecoveryBackupReport<MAX_FILES, MAX_CHUNKS>, RecoveryError> {
        if self.chunk_index != self.manifest.chunk_count {
            return Err(RecoveryError::NotComplete)
        }
        self.manifest.complete = true;
        if !self.released {
            filesystem.release_checkpoint(self.checkpoint.id)?;
            self.released = true;
        }
        catalog.record(self.manifest)?;
        Ok(RecoveryBackupReport {
            manifest: self.manifest,
            objects_uploaded: self.objects_uploaded,
            bytes_transferred: self.bytes_transferred,
            bytes_resumed: self.bytes_resumed,
            rpo_generations: 0,
        })
    }

    pub fn cancel(&mut self, filesystem: &mut SynFs<MAX_BLOCKS>) -> Result<(), RecoveryError> {
        if !self.released {
            filesystem.release_checkpoint(self.checkpoint.id)?;
            self.released = true;
        }
        Ok(())
    }
}

pub trait ObjectReader {
    fn object_size(&mut self, object_id: Digest) -> Result<u64, Status>;

    fn read_object(
        &mut self,
        object_id: Digest,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, Status>;
}

pub trait BootVerifier<const MAX_BLOCKS: usize> {
    fn verify(&self, filesystem: &SynFs<MAX_BLOCKS>) -> Result<(), SkipReason>;
}

pub struct RequiredBootPaths<const MAX_PATHS: usize> {
    paths: [Option<FileName>; MAX_PATHS],
}

impl<const MAX_PATHS: usize> RequiredBootPaths<MAX_PATHS> {
    pub const fn new(paths: [Option<FileName>; MAX_PATHS]) -> Self {
        Self { paths }
    }

    pub fn contains(&self, path: &str) -> bool {
        self.paths
            .iter()
            .flatten()
            .any(|required| required.as_str() == path)
    }
}

impl<const MAX_BLOCKS: usize, const MAX_PATHS: usize> BootVerifier<MAX_BLOCKS>
    for RequiredBootPaths<MAX_PATHS>
{
    fn verify(&self, filesystem: &SynFs<MAX_BLOCKS>) -> Result<(), SkipReason> {
        for path in self.paths.iter().flatten() {
            let file = filesystem
                .lookup(path.as_str())
                .map_err(|_| SkipReason::RequiredBootObjectMissing)?;
            if file.file_type != FileType::Regular {
                return Err(SkipReason::RequiredBootObjectInvalid)
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkipReason {
    UnsupportedFileType,
    ObjectTooLarge,
    RequiredBootObjectMissing,
    RequiredBootObjectInvalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkippedObject {
    pub path: FileName,
    pub reason: SkipReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoreReport<const MAX_SKIPPED: usize> {
    pub source_generation: u64,
    pub rpo_generations: u64,
    pub rto_ticks: u64,
    pub bytes_transferred: u64,
    pub files_restored: u32,
    pub skipped_count: u32,
    pub skipped: [Option<SkippedObject>; MAX_SKIPPED],
    pub verified: bool,
    pub bootable: bool,
}

impl<const MAX_FILES: usize, const MAX_CHUNKS: usize>
    BackupManifest<MAX_FILES, MAX_CHUNKS>
{
    /// Restore a volume only when the final boot verifier passes.
    pub fn restore_bootable<
        const MAX_BLOCKS: usize,
        const MAX_FILE_BYTES: usize,
        const MAX_SKIPPED: usize,
    >(
        &self,
        target: &mut SynFs<MAX_BLOCKS>,
        reader: &mut impl ObjectReader,
        key: EncryptionKey,
        observed_generation: u64,
        started_tick: u64,
        completed_tick: u64,
        verifier: &impl BootVerifier<MAX_BLOCKS>,
    ) -> Result<RestoreReport<MAX_SKIPPED>, RecoveryError> {
        let checkpoint = target.create_checkpoint()?;
        let report = self.restore::<MAX_BLOCKS, MAX_FILE_BYTES, MAX_SKIPPED>(
            target,
            reader,
            key,
            observed_generation,
            started_tick,
            completed_tick,
            verifier,
        );
        let report = match report {
            Ok(report) => report,
            Err(error) => {
                let _ = target.release_checkpoint(checkpoint.id);
                return Err(error)
            }
        };
        if !report.verified || !report.bootable {
            target.rollback_to_checkpoint(checkpoint.id)?;
            target.release_checkpoint(checkpoint.id)?;
            return Err(RecoveryError::NotBootable)
        }
        target.release_checkpoint(checkpoint.id)?;
        Ok(report)
    }

    pub fn restore<const MAX_BLOCKS: usize, const MAX_FILE_BYTES: usize, const MAX_SKIPPED: usize>(
        &self,
        target: &mut SynFs<MAX_BLOCKS>,
        reader: &mut impl ObjectReader,
        key: EncryptionKey,
        observed_generation: u64,
        started_tick: u64,
        completed_tick: u64,
        verifier: &impl BootVerifier<MAX_BLOCKS>,
    ) -> Result<RestoreReport<MAX_SKIPPED>, RecoveryError> {
        if !self.complete {
            return Err(RecoveryError::ManifestIncomplete)
        }
        let mut skipped = [None; MAX_SKIPPED];
        let mut skipped_count = 0u32;
        let mut files_restored = 0u32;
        let mut bytes_transferred = 0u64;
        let mut transaction = target.transaction();

        for manifest_file in self.files[..self.file_count as usize].iter().flatten() {
            let file = manifest_file.file;
            if file.file_type == FileType::Directory {
                match transaction.create_directory(file.file.as_str(), true) {
                    Ok(_) | Err(FileError::AlreadyExists) => {}
                    Err(error) => return Err(error.into()),
                }
                continue
            }
            if file.file_type != FileType::Regular {
                record_skipped(
                    &mut skipped,
                    &mut skipped_count,
                    file.file,
                    SkipReason::UnsupportedFileType,
                )?;
                continue
            }
            let Ok(file_size) = usize::try_from(file.size) else {
                record_skipped(
                    &mut skipped,
                    &mut skipped_count,
                    file.file,
                    SkipReason::ObjectTooLarge,
                )?;
                continue
            };
            if file_size > MAX_FILE_BYTES {
                record_skipped(
                    &mut skipped,
                    &mut skipped_count,
                    file.file,
                    SkipReason::ObjectTooLarge,
                )?;
                continue
            }
            let mut contents = [0; MAX_FILE_BYTES];
            let chunk_start = manifest_file.first_chunk as usize;
            let chunk_end = chunk_start.saturating_add(manifest_file.chunk_count as usize);
            let mut destination_offset = 0usize;
            for chunk in self.chunks[chunk_start..chunk_end].iter().flatten() {
                let expected_length = usize::try_from(chunk.length)
                    .map_err(|_| RecoveryError::ObjectTooLarge)?;
                let expected_wire = expected_length.saturating_add(ENCRYPTED_CHUNK_OVERHEAD);
                if expected_wire > MAX_ENCRYPTED_CHUNK_BYTES {
                    return Err(RecoveryError::ObjectTooLarge)
                }
                let stored_size = reader
                    .object_size(chunk.object_id)
                    .map_err(RecoveryError::Upload)?;
                if stored_size != expected_wire as u64 {
                    return Err(RecoveryError::ObjectCorrupt)
                }
                let mut wire = [0; MAX_ENCRYPTED_CHUNK_BYTES];
                let mut offset = 0u64;
                while offset < stored_size {
                    let left = usize::try_from(stored_size - offset)
                        .unwrap_or(MAX_ENCRYPTED_CHUNK_BYTES)
                        .min(MAX_ENCRYPTED_CHUNK_BYTES - offset as usize);
                    let copied = reader
                        .read_object(chunk.object_id, offset, &mut wire[offset as usize..offset as usize + left])
                        .map_err(RecoveryError::Upload)?;
                    if copied == 0 || copied > left {
                        return Err(RecoveryError::ObjectCorrupt)
                    }
                    offset = offset.saturating_add(copied as u64);
                    bytes_transferred = bytes_transferred.saturating_add(copied as u64);
                }
                let mut plaintext = [0; CHUNK_BYTES];
                let copied = key.decrypt(
                    chunk.object_id,
                    &wire[..expected_wire],
                    chunk.plaintext_digest,
                    &mut plaintext,
                )?;
                if copied != expected_length
                    || destination_offset.saturating_add(copied) > file_size
                {
                    return Err(RecoveryError::ObjectCorrupt)
                }
                contents[destination_offset..destination_offset + copied]
                    .copy_from_slice(&plaintext[..copied]);
                destination_offset += copied;
            }
            if destination_offset != file_size {
                return Err(RecoveryError::ObjectCorrupt)
            }
            let restored = transaction.write(file.file.as_str(), &contents[..file_size])?;
            if restored.size != file.size || restored.checksum != file.checksum {
                return Err(RecoveryError::VerificationFailed)
            }
            files_restored = files_restored.saturating_add(1);
        }
        transaction.commit()?;
        let bootable = verifier.verify(target).is_ok();
        Ok(RestoreReport {
            source_generation: self.generation,
            rpo_generations: observed_generation.saturating_sub(self.generation),
            rto_ticks: completed_tick.saturating_sub(started_tick),
            bytes_transferred,
            files_restored,
            skipped_count,
            skipped,
            verified: skipped_count == 0 && bootable,
            bootable,
        })
    }
}

fn record_skipped<const MAX_SKIPPED: usize>(
    skipped: &mut [Option<SkippedObject>; MAX_SKIPPED],
    skipped_count: &mut u32,
    path: FileName,
    reason: SkipReason,
) -> Result<(), RecoveryError> {
    let slot = skipped
        .iter_mut()
        .find(|slot| slot.is_none())
        .ok_or(RecoveryError::SkippedCapacity)?;
    *slot = Some(SkippedObject { path, reason });
    *skipped_count = skipped_count.saturating_add(1);
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryError {
    File(FileError),
    Crypto(CryptoError),
    Upload(Status),
    InvalidBudget,
    InvalidResumeOffset,
    InvalidState,
    NotComplete,
    Capacity,
    CatalogFull,
    ManifestNotFound,
    ManifestIncomplete,
    ObjectCorrupt,
    ObjectTooLarge,
    SkippedCapacity,
    VerificationFailed,
    NotBootable,
    UnsupportedConfiguration,
}

impl From<FileError> for RecoveryError {
    fn from(error: FileError) -> Self {
        Self::File(error)
    }
}

impl From<CryptoError> for RecoveryError {
    fn from(error: CryptoError) -> Self {
        Self::Crypto(error)
    }
}

impl IntoStatus for RecoveryError {
    fn status(self) -> Status {
        match self {
            Self::File(error) => error.status(),
            Self::Upload(status) => status,
            Self::Crypto(_)
            | Self::ObjectCorrupt
            | Self::VerificationFailed
            | Self::NotBootable
            | Self::ManifestIncomplete => Status::CORRUPT,
            Self::InvalidBudget
            | Self::InvalidResumeOffset
            | Self::InvalidState
            | Self::UnsupportedConfiguration => Status::INVALID_ARGUMENT,
            Self::NotComplete => Status::BUSY,
            Self::Capacity | Self::CatalogFull | Self::SkippedCapacity => Status::NO_SPACE,
            Self::ManifestNotFound => Status::NOT_FOUND,
            Self::ObjectTooLarge => Status::new(
                ghostos_status::Severity::Error,
                ghostos_status::facility::FILESYSTEM,
                5,
                0,
            )
            .unwrap_or(Status::INVALID_ARGUMENT),
        }
    }
}

impl From<BackupId> for ghostos_ghostfs::CheckpointId {
    fn from(id: BackupId) -> Self {
        ghostos_ghostfs::CheckpointId::from_valid_raw(id.raw())
    }
}
