//! Validated service packages stored inside the system GhostFS volume.

use crate::SynFs;

pub const SERVICE_MANIFEST_PATH: &str = "/system/services/manifest";
pub const SERVICE_PACKAGE_CAPACITY: usize = 16;
pub const SERVICE_PACKAGE_PATH_BYTES: usize = 96;
const MAGIC: &[u8; 8] = b"SYNSVC01";
const VERSION: u16 = 1;
const HEADER_BYTES: usize = 12;
const ENTRY_BYTES: usize = 112;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceManifestError {
    BufferTooSmall,
    Corrupt,
    Filesystem,
    InvalidPath,
    TooManyPackages,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceManifestEntry {
    pub role: u8,
    path_length: u8,
    pub image_bytes: u32,
    pub checksum: u32,
    path: [u8; SERVICE_PACKAGE_PATH_BYTES],
}

impl ServiceManifestEntry {
    pub fn new(role: u8, path: &str, image: &[u8]) -> Result<Self, ServiceManifestError> {
        if role == 0
            || path.is_empty()
            || path.len() > SERVICE_PACKAGE_PATH_BYTES
            || !path.starts_with('/')
            || image.is_empty()
        {
            return Err(ServiceManifestError::InvalidPath)
        }
        let mut stored_path = [0; SERVICE_PACKAGE_PATH_BYTES];
        stored_path[..path.len()].copy_from_slice(path.as_bytes());
        Ok(Self {
            role,
            path_length: path.len() as u8,
            image_bytes: u32::try_from(image.len()).map_err(|_| ServiceManifestError::BufferTooSmall)?,
            checksum: checksum(image),
            path: stored_path,
        })
    }

    pub fn path(&self) -> &str {
        core::str::from_utf8(&self.path[..self.path_length as usize]).unwrap_or("")
    }
}

#[derive(Clone, Copy)]
pub struct ServiceManifest {
    entries: [Option<ServiceManifestEntry>; SERVICE_PACKAGE_CAPACITY],
    count: usize,
}

impl ServiceManifest {
    pub const fn new() -> Self {
        Self {
            entries: [None; SERVICE_PACKAGE_CAPACITY],
            count: 0,
        }
    }

    pub fn push(&mut self, entry: ServiceManifestEntry) -> Result<(), ServiceManifestError> {
        if self.count == self.entries.len() {
            return Err(ServiceManifestError::TooManyPackages)
        }
        if self.entries[..self.count]
            .iter()
            .flatten()
            .any(|existing| existing.role == entry.role || existing.path() == entry.path())
        {
            return Err(ServiceManifestError::Corrupt)
        }
        self.entries[self.count] = Some(entry);
        self.count += 1;
        Ok(())
    }

    pub fn entries(&self) -> impl Iterator<Item = ServiceManifestEntry> + '_ {
        self.entries[..self.count].iter().flatten().copied()
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, ServiceManifestError> {
        let required = HEADER_BYTES + self.count * ENTRY_BYTES + 4;
        let output = output.get_mut(..required).ok_or(ServiceManifestError::BufferTooSmall)?;
        output.fill(0);
        output[..8].copy_from_slice(MAGIC);
        output[8..10].copy_from_slice(&VERSION.to_le_bytes());
        output[10..12].copy_from_slice(&(self.count as u16).to_le_bytes());
        for (index, entry) in self.entries().enumerate() {
            let start = HEADER_BYTES + index * ENTRY_BYTES;
            output[start] = entry.role;
            output[start + 1] = entry.path_length;
            output[start + 4..start + 8].copy_from_slice(&entry.image_bytes.to_le_bytes());
            output[start + 8..start + 12].copy_from_slice(&entry.checksum.to_le_bytes());
            output[start + 16..start + 16 + SERVICE_PACKAGE_PATH_BYTES]
                .copy_from_slice(&entry.path);
        }
        let digest = checksum(&output[..required - 4]);
        output[required - 4..].copy_from_slice(&digest.to_le_bytes());
        Ok(required)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ServiceManifestError> {
        if bytes.len() < HEADER_BYTES + 4 || bytes.get(..8) != Some(MAGIC) {
            return Err(ServiceManifestError::Corrupt)
        }
        if u16::from_le_bytes(bytes[8..10].try_into().map_err(|_| ServiceManifestError::Corrupt)?) != VERSION {
            return Err(ServiceManifestError::Corrupt)
        }
        let count = u16::from_le_bytes(bytes[10..12].try_into().map_err(|_| ServiceManifestError::Corrupt)?) as usize;
        if count > SERVICE_PACKAGE_CAPACITY {
            return Err(ServiceManifestError::TooManyPackages)
        }
        let required = HEADER_BYTES + count * ENTRY_BYTES + 4;
        let bytes = bytes.get(..required).ok_or(ServiceManifestError::Corrupt)?;
        let stored = u32::from_le_bytes(bytes[required - 4..].try_into().map_err(|_| ServiceManifestError::Corrupt)?);
        if checksum(&bytes[..required - 4]) != stored {
            return Err(ServiceManifestError::Corrupt)
        }
        let mut manifest = Self::new();
        for index in 0..count {
            let start = HEADER_BYTES + index * ENTRY_BYTES;
            let path_length = bytes[start + 1] as usize;
            if path_length == 0 || path_length > SERVICE_PACKAGE_PATH_BYTES {
                return Err(ServiceManifestError::InvalidPath)
            }
            let path_bytes = &bytes[start + 16..start + 16 + path_length];
            let path = core::str::from_utf8(path_bytes).map_err(|_| ServiceManifestError::InvalidPath)?;
            let image_bytes = u32::from_le_bytes(bytes[start + 4..start + 8].try_into().map_err(|_| ServiceManifestError::Corrupt)?);
            let image_checksum = u32::from_le_bytes(bytes[start + 8..start + 12].try_into().map_err(|_| ServiceManifestError::Corrupt)?);
            if bytes[start] == 0 || image_bytes == 0 || !path.starts_with('/') {
                return Err(ServiceManifestError::Corrupt)
            }
            let mut stored_path = [0; SERVICE_PACKAGE_PATH_BYTES];
            stored_path[..path_length].copy_from_slice(path_bytes);
            let entry = ServiceManifestEntry {
                role: bytes[start],
                path_length: path_length as u8,
                image_bytes,
                checksum: image_checksum,
                path: stored_path,
            };
            manifest.push(entry)?
        }
        Ok(manifest)
    }

    pub fn load<const BLOCKS: usize>(
        filesystem: &SynFs<BLOCKS>,
        scratch: &mut [u8],
    ) -> Result<Self, ServiceManifestError> {
        let info = filesystem.lookup(SERVICE_MANIFEST_PATH).map_err(|_| ServiceManifestError::Filesystem)?;
        let length = usize::try_from(info.size).map_err(|_| ServiceManifestError::BufferTooSmall)?;
        let output = scratch.get_mut(..length).ok_or(ServiceManifestError::BufferTooSmall)?;
        filesystem.read(SERVICE_MANIFEST_PATH, output).map_err(|_| ServiceManifestError::Filesystem)?;
        Self::decode(output)
    }

    pub fn load_package<const BLOCKS: usize>(
        &self,
        filesystem: &SynFs<BLOCKS>,
        role: u8,
        output: &mut [u8],
    ) -> Result<usize, ServiceManifestError> {
        let entry = self.entries().find(|entry| entry.role == role).ok_or(ServiceManifestError::Corrupt)?;
        let length = entry.image_bytes as usize;
        let output = output.get_mut(..length).ok_or(ServiceManifestError::BufferTooSmall)?;
        filesystem.read(entry.path(), output).map_err(|_| ServiceManifestError::Filesystem)?;
        if checksum(output) != entry.checksum {
            return Err(ServiceManifestError::Corrupt)
        }
        Ok(length)
    }
}

impl Default for ServiceManifest {
    fn default() -> Self {
        Self::new()
    }
}

fn checksum(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 }
        }
    }
    !crc
}
