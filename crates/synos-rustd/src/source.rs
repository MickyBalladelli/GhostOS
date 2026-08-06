use synos_synfs::{DirectoryEntry, FileType, SynFs};
use synos_system_model::ContentId;

use crate::{BuildRequest, Error, Text, MAX_PATH_BYTES};

pub const MAX_SOURCE_FILES: usize = 128;
pub const MAX_SOURCE_DIRECTORIES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct SourceCapability(u64);

impl SourceCapability {
    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw >> 32 == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceGrant {
    pub capability: SourceCapability,
    pub source_root: Text<MAX_PATH_BYTES>,
    pub manifest: Text<MAX_PATH_BYTES>,
}

impl SourceGrant {
    pub fn new(
        capability: SourceCapability,
        source_root: &str,
        manifest: &str,
    ) -> Result<Self, Error> {
        if source_root == "/" || !is_member(source_root, manifest) {
            return Err(Error::InvalidRequest);
        }
        Ok(Self {
            capability,
            source_root: Text::new(source_root)?,
            manifest: Text::new(manifest)?,
        })
    }

    pub fn authorize(&self, request: &BuildRequest) -> Result<(), Error> {
        if self.capability.raw() == 0
            || self.source_root != request.source_root
            || self.manifest != request.manifest
        {
            return Err(Error::SourceCapabilityDenied);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceFile {
    pub path: Text<MAX_PATH_BYTES>,
    pub content: ContentId,
    pub size: u64,
    pub version: u32,
    pub checksum: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSnapshot {
    pub grant: SourceGrant,
    pub identity: ContentId,
    pub total_bytes: u64,
    pub files: [Option<SourceFile>; MAX_SOURCE_FILES],
    pub length: u16,
}

impl SourceSnapshot {
    pub const fn file_count(&self) -> usize {
        self.length as usize
    }

    pub fn files(&self) -> impl Iterator<Item = SourceFile> + '_ {
        self.files.iter().flatten().copied()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.files().any(|file| file.path.as_str() == path)
    }

    pub fn read_file<const BLOCKS: usize>(
        &self,
        filesystem: &SynFs<BLOCKS>,
        path: &str,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, Error> {
        if !self.contains(path) {
            return Err(Error::SourceCapabilityDenied);
        }
        Ok(filesystem
            .read_at(path, offset, destination)
            .map_err(Error::SourceFilesystem)?
            .bytes_read)
    }
}

pub fn snapshot<const BLOCKS: usize>(
    filesystem: &SynFs<BLOCKS>,
    grant: SourceGrant,
    request: &BuildRequest,
    max_files: usize,
    max_bytes: u64,
) -> Result<SourceSnapshot, Error> {
    grant.authorize(request)?;
    if max_files == 0 || max_files > MAX_SOURCE_FILES {
        return Err(Error::SourceLimit);
    }
    let root = filesystem
        .lookup(grant.source_root.as_str())
        .map_err(Error::SourceFilesystem)?;
    let manifest = filesystem
        .lookup(grant.manifest.as_str())
        .map_err(Error::SourceFilesystem)?;
    if root.file_type != FileType::Directory
        || manifest.file_type != FileType::Regular
        || !is_member(grant.source_root.as_str(), grant.manifest.as_str())
    {
        return Err(Error::InvalidRequest);
    }

    let mut directories = [None; MAX_SOURCE_DIRECTORIES];
    directories[0] = Some(grant.source_root);
    let mut directory_length = 1;
    let mut files = [None; MAX_SOURCE_FILES];
    let mut file_length = 0;
    let mut total_bytes = 0_u64;
    while directory_length != 0 {
        directory_length -= 1;
        let directory = directories[directory_length].take().ok_or(Error::InvalidRequest)?;
        let mut entries = [DirectoryEntry::EMPTY; 256];
        let count = filesystem
            .list_directory(directory.as_str(), &mut entries)
            .map_err(Error::SourceFilesystem)?;
        for entry in entries.into_iter().take(count) {
            let path = join_path(directory.as_str(), entry.name.as_str())?;
            match entry.file_type {
                FileType::Directory => {
                    if directory_length == MAX_SOURCE_DIRECTORIES {
                        return Err(Error::SourceLimit);
                    }
                    directories[directory_length] = Some(path);
                    directory_length += 1;
                }
                FileType::Regular => {
                    if file_length == max_files || file_length == MAX_SOURCE_FILES {
                        return Err(Error::SourceLimit);
                    }
                    let metadata = filesystem
                        .lookup(path.as_str())
                        .map_err(Error::SourceFilesystem)?;
                    total_bytes = total_bytes
                        .checked_add(metadata.size)
                        .ok_or(Error::SourceLimit)?;
                    if total_bytes > max_bytes {
                        return Err(Error::SourceLimit);
                    }
                    let content = file_identity(
                        &path,
                        metadata.version,
                        metadata.size,
                        metadata.checksum,
                    );
                    files[file_length] = Some(SourceFile {
                        path,
                        content,
                        size: metadata.size,
                        version: metadata.version,
                        checksum: metadata.checksum,
                    });
                    file_length += 1;
                }
                FileType::Symlink => return Err(Error::InvalidRequest),
            }
        }
    }
    let identity = snapshot_identity(&files, file_length, total_bytes, request.target);
    Ok(SourceSnapshot {
        grant,
        identity,
        total_bytes,
        files,
        length: file_length as u16,
    })
}

fn is_member(root: &str, path: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|suffix| suffix.starts_with('/') && suffix.len() > 1)
}

fn join_path(root: &str, name: &str) -> Result<Text<MAX_PATH_BYTES>, Error> {
    let separator = usize::from(!root.ends_with('/'));
    let length = root
        .len()
        .checked_add(separator)
        .and_then(|length| length.checked_add(name.len()))
        .ok_or(Error::InvalidRequest)?;
    if length > MAX_PATH_BYTES {
        return Err(Error::InvalidRequest);
    }
    let mut bytes = [0; MAX_PATH_BYTES];
    bytes[..root.len()].copy_from_slice(root.as_bytes());
    let mut cursor = root.len();
    if separator != 0 {
        bytes[cursor] = b'/';
        cursor += 1;
    }
    bytes[cursor..length].copy_from_slice(name.as_bytes());
    Text::new(core::str::from_utf8(&bytes[..length]).map_err(|_| Error::InvalidRequest)?)
}

fn file_identity(path: &Text<MAX_PATH_BYTES>, version: u32, size: u64, checksum: u64) -> ContentId {
    let mut material = [0; MAX_PATH_BYTES + 20];
    let path_bytes = path.as_str().as_bytes();
    material[..path_bytes.len()].copy_from_slice(path_bytes);
    let mut cursor = path_bytes.len();
    material[cursor..cursor + 4].copy_from_slice(&version.to_be_bytes());
    cursor += 4;
    material[cursor..cursor + 8].copy_from_slice(&size.to_be_bytes());
    cursor += 8;
    material[cursor..cursor + 8].copy_from_slice(&checksum.to_be_bytes());
    ContentId::hash(&material[..cursor + 8])
}

fn snapshot_identity(
    files: &[Option<SourceFile>; MAX_SOURCE_FILES],
    length: usize,
    total_bytes: u64,
    target: crate::Target,
) -> ContentId {
    let mut material = [0; MAX_SOURCE_FILES * (MAX_PATH_BYTES + 32) + 17];
    let mut cursor = 0;
    material[cursor..cursor + 8].copy_from_slice(&total_bytes.to_be_bytes());
    cursor += 8;
    material[cursor] = target as u8;
    cursor += 1;
    for file in files.iter().take(length).flatten() {
        let path = file.path.as_str().as_bytes();
        if cursor + path.len() + 32 > material.len() {
            break;
        }
        material[cursor..cursor + path.len()].copy_from_slice(path);
        cursor += path.len();
        material[cursor..cursor + 32].copy_from_slice(file.content.as_bytes());
        cursor += 32;
    }
    ContentId::hash(&material[..cursor])
}
