use synos_status::{IntoStatus, Status};
use synos_synfs::{FileName, FileType, ReadOnlySnapshot, SynFs};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileFingerprint {
    pub file_type: FileType,
    pub size: u64,
    pub checksum: u64,
}

impl From<synos_synfs::FileVersion> for FileFingerprint {
    fn from(file: synos_synfs::FileVersion) -> Self {
        Self {
            file_type: file.file_type,
            size: file.size,
            checksum: file.checksum,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CowOperation<const MAX_BYTES: usize> {
    Write {
        path: FileName,
        expected: Option<FileFingerprint>,
        len: u32,
        checksum: u64,
        bytes: [u8; MAX_BYTES],
    },
    Delete {
        path: FileName,
        expected: FileFingerprint,
    },
    CreateDirectory {
        path: FileName,
        expected: Option<FileFingerprint>,
    },
    RemoveDirectory {
        path: FileName,
        expected: FileFingerprint,
    },
}

impl<const MAX_BYTES: usize> CowOperation<MAX_BYTES> {
    fn path(self) -> FileName {
        match self {
            Self::Write { path, .. }
            | Self::Delete { path, .. }
            | Self::CreateDirectory { path, .. }
            | Self::RemoveDirectory { path, .. } => path,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CowDelta<const OPS: usize = 64, const MAX_BYTES: usize = 4096> {
    base_generation: u64,
    target_generation: u64,
    operations: [Option<CowOperation<MAX_BYTES>>; OPS],
    len: usize,
}

impl<const OPS: usize, const MAX_BYTES: usize> CowDelta<OPS, MAX_BYTES> {
    pub const fn new(base_generation: u64, target_generation: u64) -> Self {
        Self {
            base_generation,
            target_generation,
            operations: [None; OPS],
            len: 0,
        }
    }

    pub const fn base_generation(&self) -> u64 {
        self.base_generation
    }

    pub const fn target_generation(&self) -> u64 {
        self.target_generation
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn operations(&self) -> impl Iterator<Item = CowOperation<MAX_BYTES>> + '_ {
        self.operations[..self.len].iter().flatten().copied()
    }

    fn push(&mut self, operation: CowOperation<MAX_BYTES>) -> Result<(), DeltaError> {
        let slot = self
            .operations
            .get_mut(self.len)
            .ok_or(DeltaError::Capacity)?;
        *slot = Some(operation);
        self.len += 1;
        Ok(())
    }

    pub fn apply<const BLOCKS: usize>(
        &self,
        filesystem: &mut SynFs<BLOCKS>,
        mode: DeltaMode,
    ) -> Result<DeltaApplyReceipt, DeltaError> {
        apply_delta(filesystem, self, mode)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeltaMode {
    RejectConflicts,
    PreferRemote,
    PreferLocal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeltaError {
    Capacity,
    Conflict { path: FileName },
    FileTooLarge { required: usize },
    InvalidGeneration,
    Filesystem(synos_synfs::Error),
}

impl From<synos_synfs::Error> for DeltaError {
    fn from(error: synos_synfs::Error) -> Self {
        Self::Filesystem(error)
    }
}

impl IntoStatus for DeltaError {
    fn status(self) -> Status {
        match self {
            Self::Capacity | Self::FileTooLarge { .. } => Status::NO_SPACE,
            Self::Conflict { .. } => Status::CONFLICT,
            Self::InvalidGeneration => Status::INVALID_ARGUMENT,
            Self::Filesystem(error) => error.status(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeltaApplyReceipt {
    pub applied: usize,
    pub skipped: usize,
    pub generation: u64,
}

pub fn build_delta<
    'a,
    'b,
    const BLOCKS: usize,
    const OPS: usize,
    const MAX_BYTES: usize,
>(
    base: &ReadOnlySnapshot<'a, BLOCKS>,
    target: &ReadOnlySnapshot<'b, BLOCKS>,
    delta: &mut CowDelta<OPS, MAX_BYTES>,
) -> Result<(), DeltaError> {
    if target.generation() < base.generation() {
        return Err(DeltaError::InvalidGeneration);
    }
    *delta = CowDelta::new(base.generation(), target.generation());

    let mut index = 0;
    while let Some(file) = target.file_at(index)? {
        let expected = lookup_fingerprint(base, file.file.as_str())?;
        if expected == Some(file.into()) {
            index = index.saturating_add(1);
            continue;
        }
        if let Some(previous) = expected {
            if previous.file_type != file.file_type {
                push_remove_or_delete(delta, file.file, previous)?;
            }
        }
        if file.file_type == FileType::Directory {
            delta.push(CowOperation::CreateDirectory {
                path: file.file,
                expected,
            })?;
        } else {
            let size = usize::try_from(file.size).map_err(|_| DeltaError::FileTooLarge {
                required: usize::MAX,
            })?;
            if size > MAX_BYTES {
                return Err(DeltaError::FileTooLarge { required: size });
            }
            let mut bytes = [0; MAX_BYTES];
            let read = target.read_file_at(index, 0, &mut bytes)?;
            if read != size {
                return Err(DeltaError::Filesystem(synos_synfs::Error::Corrupt));
            }
            delta.push(CowOperation::Write {
                path: file.file,
                expected,
                len: read as u32,
                checksum: file.checksum,
                bytes,
            })?;
        }
        index = index.saturating_add(1);
    }

    let mut index = 0;
    while let Some(file) = base.file_at(index)? {
        if lookup_fingerprint(target, file.file.as_str())?.is_none() {
            if file.file_type == FileType::Directory {
                delta.push(CowOperation::RemoveDirectory {
                    path: file.file,
                    expected: file.into(),
                })?;
            } else {
                delta.push(CowOperation::Delete {
                    path: file.file,
                    expected: file.into(),
                })?;
            }
        }
        index = index.saturating_add(1);
    }
    Ok(())
}

fn lookup_fingerprint<const BLOCKS: usize>(
    snapshot: &ReadOnlySnapshot<'_, BLOCKS>,
    path: &str,
) -> Result<Option<FileFingerprint>, DeltaError> {
    match snapshot.lookup(path) {
        Ok(file) => Ok(Some(file.into())),
        Err(synos_synfs::Error::NotFound) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn push_remove_or_delete<const OPS: usize, const MAX_BYTES: usize>(
    delta: &mut CowDelta<OPS, MAX_BYTES>,
    path: FileName,
    expected: FileFingerprint,
) -> Result<(), DeltaError> {
    if expected.file_type == FileType::Directory {
        delta.push(CowOperation::RemoveDirectory { path, expected })
    } else {
        delta.push(CowOperation::Delete { path, expected })
    }
}

pub fn apply_delta<const BLOCKS: usize, const OPS: usize, const MAX_BYTES: usize>(
    filesystem: &mut SynFs<BLOCKS>,
    delta: &CowDelta<OPS, MAX_BYTES>,
    mode: DeltaMode,
) -> Result<DeltaApplyReceipt, DeltaError> {
    if filesystem.generation() < delta.base_generation {
        return Err(DeltaError::InvalidGeneration);
    }

    let mut apply = [false; OPS];
    let mut skipped = 0;
    for (index, operation) in delta.operations().enumerate() {
        if let CowOperation::Write { len, .. } = operation {
            if len as usize > MAX_BYTES {
                return Err(DeltaError::FileTooLarge {
                    required: len as usize,
                });
            }
        }
        let path = operation.path();
        let current = filesystem.lookup(path.as_str()).ok().map(Into::into);
        let expected = match operation {
            CowOperation::Write { expected, .. }
            | CowOperation::CreateDirectory { expected, .. } => expected,
            CowOperation::Delete { expected, .. }
            | CowOperation::RemoveDirectory { expected, .. } => Some(expected),
        };
        if current != expected {
            match mode {
                DeltaMode::RejectConflicts => return Err(DeltaError::Conflict { path }),
                DeltaMode::PreferLocal => skipped += 1,
                DeltaMode::PreferRemote => apply[index] = true,
            }
        } else {
            apply[index] = true;
        }
    }

    let mut transaction = filesystem.transaction();
    let mut applied = 0;
    for (index, operation) in delta.operations().enumerate() {
        if !apply[index] || !matches!(operation, CowOperation::Delete { .. }) {
            continue;
        }
        let CowOperation::Delete { path, .. } = operation else { unreachable!() };
        transaction.delete(path.as_str())?;
        applied += 1;
    }
    for index in (0..delta.len()).rev() {
        if !apply[index] {
            continue;
        }
        let Some(CowOperation::RemoveDirectory { path, .. }) = delta.operations().nth(index) else {
            continue;
        };
        transaction.remove_directory(path.as_str())?;
        applied += 1;
    }
    for (index, operation) in delta.operations().enumerate() {
        if !apply[index] {
            continue;
        }
        let CowOperation::CreateDirectory { path, .. } = operation else {
            continue;
        };
        transaction.create_directory(path.as_str(), true)?;
        applied += 1;
    }
    for (index, operation) in delta.operations().enumerate() {
        if !apply[index] {
            continue;
        }
        let CowOperation::Write { path, len, bytes, .. } = operation else {
            continue;
        };
        transaction.write(path.as_str(), &bytes[..len as usize])?;
        applied += 1;
    }
    let commit = transaction.commit()?;
    if delta.target_generation > 0 && filesystem.generation() < delta.target_generation {
        filesystem.synchronize_generation(delta.target_generation)?;
    }
    Ok(DeltaApplyReceipt {
        applied,
        skipped,
        generation: commit.generation.max(filesystem.generation()),
    })
}
