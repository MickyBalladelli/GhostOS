use crate::service::{MountInfo, MountState, StoragePath, MAX_MOUNTS};
use crate::{Endpoint, MountOptions, Protocol};

pub const MOUNTS_STATE_FILE: &str = "SYS$SYSTEM:MOUNTS.DAT;1";
pub const MOUNTS_STATE_MAGIC: &[u8; 8] = b"SYNMNT01";
const HEADER_BYTES: usize = 24;
const RECORD_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountStateError {
    BufferTooSmall,
    Corrupt,
    TooManyMounts,
    InvalidMount,
}

/// Versioned, deterministic mount catalog image. The image is intended to be
/// written as one SynFS CoW transaction to `MOUNTS_STATE_FILE`.
#[derive(Clone, Copy)]
pub struct MountCatalog {
    version: u64,
    mounts: [Option<MountInfo>; MAX_MOUNTS],
}

impl MountCatalog {
    pub(crate) const fn from_mounts(
        mounts: [Option<MountInfo>; MAX_MOUNTS],
        version: u64,
    ) -> Self {
        Self { version, mounts }
    }

    pub const fn version(&self) -> u64 {
        self.version
    }

    pub(crate) const fn mounts(&self) -> [Option<MountInfo>; MAX_MOUNTS] {
        self.mounts
    }

    pub fn encoded_len(&self) -> usize {
        HEADER_BYTES + self.mounts.iter().flatten().count() * RECORD_BYTES
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, MountStateError> {
        let required = self.encoded_len();
        if output.len() < required {
            return Err(MountStateError::BufferTooSmall)
        }
        output[..required].fill(0);
        output[..8].copy_from_slice(MOUNTS_STATE_MAGIC);
        output[8..16].copy_from_slice(&self.version.to_le_bytes());
        output[16..20].copy_from_slice(&(self.mounts.iter().flatten().count() as u32).to_le_bytes());
        let mut cursor = HEADER_BYTES;
        for mount in self.mounts.iter().flatten() {
            encode_mount(*mount, &mut output[cursor..cursor + RECORD_BYTES])?;
            cursor += RECORD_BYTES;
        }
        Ok(required)
    }

    pub fn decode(input: &[u8]) -> Result<Self, MountStateError> {
        if input.len() < HEADER_BYTES || &input[..8] != MOUNTS_STATE_MAGIC {
            return Err(MountStateError::Corrupt)
        }
        let mut version_bytes = [0; 8];
        version_bytes.copy_from_slice(&input[8..16]);
        let version = u64::from_le_bytes(version_bytes);
        let mut count_bytes = [0; 4];
        count_bytes.copy_from_slice(&input[16..20]);
        let count = u32::from_le_bytes(count_bytes) as usize;
        if count > MAX_MOUNTS || input.len() < HEADER_BYTES + count * RECORD_BYTES {
            return Err(MountStateError::Corrupt)
        }
        let mut mounts = [None; MAX_MOUNTS];
        let mut cursor = HEADER_BYTES;
        for slot in &mut mounts[..count] {
            *slot = Some(decode_mount(&input[cursor..cursor + RECORD_BYTES])?);
            cursor += RECORD_BYTES;
        }
        Ok(Self { version, mounts })
    }

    pub fn save_to_synfs<const BLOCKS: usize>(
        &self,
        filesystem: &mut synos_synfs::SynFs<BLOCKS>,
        staging: &mut [u8],
    ) -> Result<synos_synfs::TransactionCommit, MountStateError> {
        let length = self.encode(staging)?;
        let mut transaction = filesystem.transaction();
        transaction
            .create_directory("/system", true)
            .map_err(|_| MountStateError::InvalidMount)?;
        transaction
            .write("/system/mounts.dat", &staging[..length])
            .map_err(|_| MountStateError::InvalidMount)?;
        transaction.commit().map_err(|_| MountStateError::InvalidMount)
    }
}

fn encode_mount(mount: MountInfo, output: &mut [u8]) -> Result<(), MountStateError> {
    let logical = mount.logical.as_str().as_bytes();
    let host = mount.endpoint.host().as_bytes();
    let endpoint_path = mount.endpoint.path().as_bytes();
    if logical.len() > 128 || host.len() > 95 || endpoint_path.len() > 191 {
        return Err(MountStateError::InvalidMount)
    }
    output[..4].copy_from_slice(&mount.id.raw().to_le_bytes());
    output[4..8].copy_from_slice(&mount.generation.to_le_bytes());
    output[8..16].copy_from_slice(&mount.catalog_version.to_le_bytes());
    output[16] = protocol_byte(mount.protocol);
    output[17] = u8::from(mount.options.read_only);
    output[18] = cache_byte(mount.options.cache);
    output[19] = mount.options.retry_limit;
    output[20..24].copy_from_slice(&(mount.options.max_io_bytes as u32).to_le_bytes());
    output[24..26].copy_from_slice(&mount.endpoint.port().to_le_bytes());
    output[26] = logical.len() as u8;
    output[27] = host.len() as u8;
    output[28..30].copy_from_slice(&(endpoint_path.len() as u16).to_le_bytes());
    output[32..32 + logical.len()].copy_from_slice(logical);
    output[160..160 + host.len()].copy_from_slice(host);
    output[256..256 + endpoint_path.len()].copy_from_slice(endpoint_path);
    Ok(())
}

fn decode_mount(input: &[u8]) -> Result<MountInfo, MountStateError> {
    let id = u32::from_le_bytes(input[..4].try_into().map_err(|_| MountStateError::Corrupt)?);
    let generation = u32::from_le_bytes(input[4..8].try_into().map_err(|_| MountStateError::Corrupt)?);
    let catalog_version = u64::from_le_bytes(input[8..16].try_into().map_err(|_| MountStateError::Corrupt)?);
    let protocol = decode_protocol(input[16]).ok_or(MountStateError::InvalidMount)?;
    let options = MountOptions {
        read_only: input[17] != 0,
        cache: decode_cache(input[18]).ok_or(MountStateError::InvalidMount)?,
        retry_limit: input[19],
        max_io_bytes: u32::from_le_bytes(input[20..24].try_into().map_err(|_| MountStateError::Corrupt)?) as usize,
    };
    options.validate().map_err(|_| MountStateError::InvalidMount)?;
    let port = u16::from_le_bytes(input[24..26].try_into().map_err(|_| MountStateError::Corrupt)?);
    let logical_len = input[26] as usize;
    let host_len = input[27] as usize;
    let path_len = u16::from_le_bytes(input[28..30].try_into().map_err(|_| MountStateError::Corrupt)?) as usize;
    if logical_len == 0 || logical_len > 128 || host_len == 0 || host_len > 95 || path_len == 0 || path_len > 191 {
        return Err(MountStateError::InvalidMount)
    }
    let logical = StoragePath::new(core::str::from_utf8(&input[32..32 + logical_len]).map_err(|_| MountStateError::Corrupt)?)
        .map_err(|_| MountStateError::InvalidMount)?;
    let host = core::str::from_utf8(&input[160..160 + host_len]).map_err(|_| MountStateError::Corrupt)?;
    let path = core::str::from_utf8(&input[256..256 + path_len]).map_err(|_| MountStateError::Corrupt)?;
    let endpoint = Endpoint::new(host, path, port).map_err(|_| MountStateError::InvalidMount)?;
    Ok(MountInfo {
        id: crate::MountId::from_raw(id),
        logical,
        endpoint,
        protocol,
        features: protocol.features(),
        options,
        generation,
        catalog_version,
        state: MountState::Mounted,
    })
}

fn protocol_byte(protocol: Protocol) -> u8 {
    match protocol {
        Protocol::Pnfs(crate::NfsMinorVersion::V41) => 1,
        Protocol::Pnfs(crate::NfsMinorVersion::V42) => 2,
        Protocol::Smb { multichannel, direct, .. } => 3 | (u8::from(multichannel) << 4) | (u8::from(direct) << 5),
        Protocol::NvmeOf(crate::FabricTransport::Tcp) => 4,
        Protocol::NvmeOf(crate::FabricTransport::RoceV2) => 5,
        Protocol::Iscsi => 6,
        Protocol::S3 => 7,
    }
}

fn decode_protocol(value: u8) -> Option<Protocol> {
    Some(match value & 0x0f {
        1 => Protocol::Pnfs(crate::NfsMinorVersion::V41),
        2 => Protocol::Pnfs(crate::NfsMinorVersion::V42),
        3 => Protocol::Smb {
            dialect: crate::SmbDialect::Smb311,
            multichannel: value & (1 << 4) != 0,
            direct: value & (1 << 5) != 0,
        },
        4 => Protocol::NvmeOf(crate::FabricTransport::Tcp),
        5 => Protocol::NvmeOf(crate::FabricTransport::RoceV2),
        6 => Protocol::Iscsi,
        7 => Protocol::S3,
        _ => return None,
    })
}

fn cache_byte(cache: crate::CacheMode) -> u8 {
    match cache {
        crate::CacheMode::Disabled => 0,
        crate::CacheMode::ReadThrough => 1,
        crate::CacheMode::CopyOnWrite => 2,
    }
}

fn decode_cache(value: u8) -> Option<crate::CacheMode> {
    match value {
        0 => Some(crate::CacheMode::Disabled),
        1 => Some(crate::CacheMode::ReadThrough),
        2 => Some(crate::CacheMode::CopyOnWrite),
        _ => None,
    }
}
