use synos_status::{IntoStatus, Status};

pub const CLUSTER_METADATA_STATE_FILE: &str = "/system/cluster/metadata.dat";
pub const CLUSTER_METADATA_MAGIC: &[u8; 8] = b"SYNCLID1";
pub const CLUSTER_METADATA_FORMAT_VERSION: u16 = 1;
pub const MAX_CLUSTERS: usize = 8;
pub const MAX_CLUSTER_NAME_BYTES: usize = 64;
pub const MAX_CLUSTER_DESCRIPTION_BYTES: usize = 128;
pub const MAX_CLUSTER_ALIASES: usize = 4;
pub const MAX_TRUSTED_PEERS: usize = 8;
pub const MAX_INVITATIONS: usize = 8;
pub const MAX_CERTIFICATES: usize = 8;
pub const CLUSTER_ID_BYTES: usize = 16;
pub const NODE_ID_BYTES: usize = 16;
pub const TOKEN_BYTES: usize = 32;
pub const FINGERPRINT_BYTES: usize = 32;

const HEADER_BYTES: usize = 64;
const CLUSTER_RECORD_BYTES: usize = 1_778;
const CHECKSUM_OFFSET: usize = 56;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ClusterId([u8; CLUSTER_ID_BYTES]);

impl ClusterId {
    pub fn new(bytes: [u8; CLUSTER_ID_BYTES]) -> Result<Self, ClusterMetadataError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(ClusterMetadataError::InvalidValue);
        }
        Ok(Self(bytes))
    }

    pub const fn from_raw(bytes: [u8; CLUSTER_ID_BYTES]) -> Option<Self> {
        let mut index = 0;
        while index < CLUSTER_ID_BYTES {
            if bytes[index] != 0 {
                return Some(Self(bytes));
            }
            index += 1;
        }
        None
    }

    pub const fn from_valid_raw(bytes: [u8; CLUSTER_ID_BYTES]) -> Self {
        Self(bytes)
    }

    pub const fn raw(self) -> [u8; CLUSTER_ID_BYTES] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetadataText<const CAPACITY: usize> {
    bytes: [u8; CAPACITY],
    len: u16,
}

impl<const CAPACITY: usize> MetadataText<CAPACITY> {
    pub fn new(value: &str) -> Result<Self, ClusterMetadataError> {
        if value.len() > CAPACITY
            || value.len() > u16::MAX as usize
            || value.as_bytes().contains(&0)
        {
            return Err(ClusterMetadataError::InvalidValue);
        }
        let mut bytes = [0; CAPACITY];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            len: value.len() as u16,
        })
    }

    pub const fn empty() -> Self {
        Self {
            bytes: [0; CAPACITY],
            len: 0,
        }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ClusterLifecycle {
    Creating = 1,
    PendingAdmission = 2,
    Active = 3,
    Degraded = 4,
    Partitioned = 5,
    Draining = 6,
    Leaving = 7,
    Retired = 8,
    Deleted = 9,
}

impl ClusterLifecycle {
    const fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::Creating,
            2 => Self::PendingAdmission,
            3 => Self::Active,
            4 => Self::Degraded,
            5 => Self::Partitioned,
            6 => Self::Draining,
            7 => Self::Leaving,
            8 => Self::Retired,
            9 => Self::Deleted,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MembershipIntent {
    None = 0,
    Create = 1,
    Join = 2,
    Leave = 3,
    Rejoin = 4,
}

impl MembershipIntent {
    const fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            0 => Self::None,
            1 => Self::Create,
            2 => Self::Join,
            3 => Self::Leave,
            4 => Self::Rejoin,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrustedPeer {
    pub node_id: [u8; NODE_ID_BYTES],
    pub certificate_id: [u8; CLUSTER_ID_BYTES],
    pub role: u8,
    pub last_seen_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Invitation {
    pub cluster_id: ClusterId,
    pub token: [u8; TOKEN_BYTES],
    pub expires_at: u64,
    pub scope: u32,
    pub used: bool,
    pub revoked: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Certificate {
    pub fingerprint: [u8; FINGERPRINT_BYTES],
    pub issued_at: u64,
    pub expires_at: u64,
    pub revoked: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterMetadata {
    pub id: ClusterId,
    pub name: MetadataText<MAX_CLUSTER_NAME_BYTES>,
    pub description: MetadataText<MAX_CLUSTER_DESCRIPTION_BYTES>,
    pub aliases: [Option<MetadataText<MAX_CLUSTER_NAME_BYTES>>; MAX_CLUSTER_ALIASES],
    pub owner: [u8; CLUSTER_ID_BYTES],
    pub generation: u64,
    pub created_at: u64,
    pub lifecycle: ClusterLifecycle,
    pub local_membership: MembershipIntent,
    pub trusted_peers: [Option<TrustedPeer>; MAX_TRUSTED_PEERS],
    pub invitations: [Option<Invitation>; MAX_INVITATIONS],
    pub certificates: [Option<Certificate>; MAX_CERTIFICATES],
    pub authority_epoch: u64,
    pub authority: [u8; NODE_ID_BYTES],
}

impl ClusterMetadata {
    pub fn new(
        id: ClusterId,
        name: MetadataText<MAX_CLUSTER_NAME_BYTES>,
        description: MetadataText<MAX_CLUSTER_DESCRIPTION_BYTES>,
        owner: [u8; CLUSTER_ID_BYTES],
        created_at: u64,
    ) -> Self {
        Self {
            id,
            name,
            description,
            aliases: [None; MAX_CLUSTER_ALIASES],
            owner,
            generation: 1,
            created_at,
            lifecycle: ClusterLifecycle::Creating,
            local_membership: MembershipIntent::Create,
            trusted_peers: [None; MAX_TRUSTED_PEERS],
            invitations: [None; MAX_INVITATIONS],
            certificates: [None; MAX_CERTIFICATES],
            authority_epoch: 0,
            authority: [0; NODE_ID_BYTES],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterMetadataSnapshot {
    pub catalog_generation: u64,
    pub active_cluster: Option<ClusterId>,
    pub clusters: [Option<ClusterMetadata>; MAX_CLUSTERS],
    pub writer_epoch: u64,
    pub writer: [u8; NODE_ID_BYTES],
    pub checkpoint: synos_synfs::CheckpointId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterPatch {
    pub description: Option<MetadataText<MAX_CLUSTER_DESCRIPTION_BYTES>>,
    pub owner: Option<[u8; CLUSTER_ID_BYTES]>,
    pub lifecycle: Option<ClusterLifecycle>,
    pub local_membership: Option<MembershipIntent>,
}

impl ClusterPatch {
    pub const EMPTY: Self = Self {
        description: None,
        owner: None,
        lifecycle: None,
        local_membership: None,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterMetadataError {
    AlreadyExists,
    BufferTooSmall { required: usize },
    Capacity,
    Conflict,
    Corrupt,
    InvalidValue,
    NotFound,
    NotRestorable,
    Persistence,
    SplitBrain,
    StaleGeneration,
}

impl IntoStatus for ClusterMetadataError {
    fn status(self) -> Status {
        match self {
            Self::AlreadyExists => Status::ALREADY_EXISTS,
            Self::BufferTooSmall { .. } | Self::Capacity => Status::NO_SPACE,
            Self::Conflict | Self::SplitBrain | Self::StaleGeneration => Status::CONFLICT,
            Self::Corrupt => Status::CORRUPT,
            Self::InvalidValue => Status::INVALID_ARGUMENT,
            Self::NotFound | Self::NotRestorable => Status::NOT_FOUND,
            Self::Persistence => Status::BUSY,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ClusterMetadataCatalog {
    catalog_generation: u64,
    active_cluster: Option<ClusterId>,
    clusters: [Option<ClusterMetadata>; MAX_CLUSTERS],
    writer_epoch: u64,
    writer: [u8; NODE_ID_BYTES],
}

impl ClusterMetadataCatalog {
    pub const fn new() -> Self {
        Self {
            catalog_generation: 0,
            active_cluster: None,
            clusters: [None; MAX_CLUSTERS],
            writer_epoch: 0,
            writer: [0; NODE_ID_BYTES],
        }
    }

    pub const fn catalog_generation(&self) -> u64 {
        self.catalog_generation
    }

    pub const fn active_cluster(&self) -> Option<ClusterId> {
        self.active_cluster
    }

    pub const fn writer_epoch(&self) -> u64 {
        self.writer_epoch
    }

    pub fn cluster(&self, id: ClusterId) -> Option<ClusterMetadata> {
        self.clusters
            .iter()
            .flatten()
            .find(|cluster| cluster.id == id)
            .copied()
    }

    pub fn cluster_at(&self, index: usize) -> Option<ClusterMetadata> {
        self.clusters.get(index).copied().flatten()
    }

    pub fn cluster_by_name(&self, name: &str) -> Option<ClusterMetadata> {
        self.clusters
            .iter()
            .flatten()
            .find(|cluster| {
                cluster.name.as_str().eq_ignore_ascii_case(name)
                    || cluster
                        .aliases
                        .iter()
                        .flatten()
                        .any(|alias| alias.as_str().eq_ignore_ascii_case(name))
            })
            .copied()
    }

    pub fn create_cluster(
        &mut self,
        metadata: ClusterMetadata,
        expected_catalog_generation: u64,
    ) -> Result<(), ClusterMetadataError> {
        self.require_catalog_generation(expected_catalog_generation)?;
        self.validate_cluster(&metadata)?;
        if self
            .clusters
            .iter()
            .flatten()
            .any(|cluster| cluster.id == metadata.id)
        {
            return Err(ClusterMetadataError::AlreadyExists);
        }
        if self.cluster_by_name(metadata.name.as_str()).is_some() {
            return Err(ClusterMetadataError::Conflict);
        }
        let slot = self
            .clusters
            .iter_mut()
            .find(|cluster| cluster.is_none())
            .ok_or(ClusterMetadataError::Capacity)?;
        *slot = Some(metadata);
        self.bump_catalog_generation()
    }

    pub fn rename_cluster(
        &mut self,
        id: ClusterId,
        new_name: MetadataText<MAX_CLUSTER_NAME_BYTES>,
        expected_cluster_generation: u64,
    ) -> Result<(), ClusterMetadataError> {
        if new_name.is_empty() {
            return Err(ClusterMetadataError::InvalidValue);
        }
        if self
            .cluster_by_name(new_name.as_str())
            .is_some_and(|cluster| cluster.id != id)
        {
            return Err(ClusterMetadataError::Conflict);
        }
        let cluster = self.cluster_mut(id)?;
        if cluster.generation != expected_cluster_generation {
            return Err(ClusterMetadataError::StaleGeneration);
        }
        cluster.name = new_name;
        cluster.generation = cluster
            .generation
            .checked_add(1)
            .ok_or(ClusterMetadataError::Conflict)?;
        self.bump_catalog_generation()
    }

    pub fn apply_patch(
        &mut self,
        id: ClusterId,
        patch: ClusterPatch,
        expected_cluster_generation: u64,
    ) -> Result<(), ClusterMetadataError> {
        let cluster = self.cluster_mut(id)?;
        if cluster.generation != expected_cluster_generation {
            return Err(ClusterMetadataError::StaleGeneration);
        }
        if let Some(description) = patch.description {
            cluster.description = description;
        }
        if let Some(owner) = patch.owner {
            cluster.owner = owner;
        }
        if let Some(lifecycle) = patch.lifecycle {
            cluster.lifecycle = lifecycle;
        }
        if let Some(intent) = patch.local_membership {
            cluster.local_membership = intent;
        }
        cluster.generation = cluster
            .generation
            .checked_add(1)
            .ok_or(ClusterMetadataError::Conflict)?;
        self.bump_catalog_generation()
    }

    pub fn set_active_cluster(
        &mut self,
        id: ClusterId,
        expected_catalog_generation: u64,
    ) -> Result<(), ClusterMetadataError> {
        self.require_catalog_generation(expected_catalog_generation)?;
        let cluster = self.cluster(id).ok_or(ClusterMetadataError::NotFound)?;
        if cluster.lifecycle == ClusterLifecycle::Deleted {
            return Err(ClusterMetadataError::Conflict);
        }
        self.active_cluster = Some(id);
        self.bump_catalog_generation()
    }

    pub fn set_writer(
        &mut self,
        epoch: u64,
        writer: [u8; NODE_ID_BYTES],
    ) -> Result<(), ClusterMetadataError> {
        if writer.iter().all(|byte| *byte == 0) || epoch == 0 {
            return Err(ClusterMetadataError::InvalidValue);
        }
        if epoch < self.writer_epoch {
            return Err(ClusterMetadataError::StaleGeneration);
        }
        if epoch == self.writer_epoch && self.writer != [0; NODE_ID_BYTES] && self.writer != writer
        {
            return Err(ClusterMetadataError::SplitBrain);
        }
        self.writer_epoch = epoch;
        self.writer = writer;
        self.bump_catalog_generation()
    }

    pub fn add_alias(
        &mut self,
        id: ClusterId,
        alias: MetadataText<MAX_CLUSTER_NAME_BYTES>,
        expected_cluster_generation: u64,
    ) -> Result<(), ClusterMetadataError> {
        if alias.is_empty() {
            return Err(ClusterMetadataError::InvalidValue);
        }
        if self.cluster_by_name(alias.as_str()).is_some() {
            return Err(ClusterMetadataError::Conflict);
        }
        let cluster = self.cluster_mut(id)?;
        if cluster.generation != expected_cluster_generation {
            return Err(ClusterMetadataError::StaleGeneration);
        }
        let slot = cluster
            .aliases
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ClusterMetadataError::Capacity)?;
        *slot = Some(alias);
        cluster.generation = cluster
            .generation
            .checked_add(1)
            .ok_or(ClusterMetadataError::Conflict)?;
        self.bump_catalog_generation()
    }

    pub fn add_trusted_peer(
        &mut self,
        id: ClusterId,
        peer: TrustedPeer,
        expected_cluster_generation: u64,
    ) -> Result<(), ClusterMetadataError> {
        let cluster = self.cluster_mut(id)?;
        if cluster.generation != expected_cluster_generation {
            return Err(ClusterMetadataError::StaleGeneration);
        }
        if cluster
            .trusted_peers
            .iter()
            .flatten()
            .any(|entry| entry.node_id == peer.node_id)
        {
            return Err(ClusterMetadataError::AlreadyExists);
        }
        let slot = cluster
            .trusted_peers
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ClusterMetadataError::Capacity)?;
        *slot = Some(peer);
        cluster.generation = cluster
            .generation
            .checked_add(1)
            .ok_or(ClusterMetadataError::Conflict)?;
        self.bump_catalog_generation()
    }

    pub fn add_invitation(
        &mut self,
        id: ClusterId,
        invitation: Invitation,
        expected_cluster_generation: u64,
    ) -> Result<(), ClusterMetadataError> {
        if invitation.cluster_id != id || invitation.token.iter().all(|byte| *byte == 0) {
            return Err(ClusterMetadataError::InvalidValue);
        }
        let cluster = self.cluster_mut(id)?;
        if cluster.generation != expected_cluster_generation {
            return Err(ClusterMetadataError::StaleGeneration);
        }
        if cluster
            .invitations
            .iter()
            .flatten()
            .any(|entry| entry.token == invitation.token)
        {
            return Err(ClusterMetadataError::AlreadyExists);
        }
        let slot = cluster
            .invitations
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ClusterMetadataError::Capacity)?;
        *slot = Some(invitation);
        cluster.generation = cluster
            .generation
            .checked_add(1)
            .ok_or(ClusterMetadataError::Conflict)?;
        self.bump_catalog_generation()
    }

    pub fn add_certificate(
        &mut self,
        id: ClusterId,
        certificate: Certificate,
        expected_cluster_generation: u64,
    ) -> Result<(), ClusterMetadataError> {
        if certificate.fingerprint.iter().all(|byte| *byte == 0) {
            return Err(ClusterMetadataError::InvalidValue);
        }
        let cluster = self.cluster_mut(id)?;
        if cluster.generation != expected_cluster_generation {
            return Err(ClusterMetadataError::StaleGeneration);
        }
        if cluster
            .certificates
            .iter()
            .flatten()
            .any(|entry| entry.fingerprint == certificate.fingerprint)
        {
            return Err(ClusterMetadataError::AlreadyExists);
        }
        let slot = cluster
            .certificates
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ClusterMetadataError::Capacity)?;
        *slot = Some(certificate);
        cluster.generation = cluster
            .generation
            .checked_add(1)
            .ok_or(ClusterMetadataError::Conflict)?;
        self.bump_catalog_generation()
    }

    pub fn snapshot<const BLOCKS: usize>(
        &mut self,
        filesystem: &mut synos_synfs::SynFs<BLOCKS>,
    ) -> Result<ClusterMetadataSnapshot, ClusterMetadataError> {
        let checkpoint = filesystem
            .create_checkpoint()
            .map_err(|_| ClusterMetadataError::Persistence)?;
        Ok(ClusterMetadataSnapshot {
            catalog_generation: self.catalog_generation,
            active_cluster: self.active_cluster,
            clusters: self.clusters,
            writer_epoch: self.writer_epoch,
            writer: self.writer,
            checkpoint: checkpoint.id,
        })
    }

    pub fn restore_snapshot<const BLOCKS: usize>(
        &mut self,
        filesystem: &mut synos_synfs::SynFs<BLOCKS>,
        snapshot: ClusterMetadataSnapshot,
        expected_catalog_generation: u64,
    ) -> Result<(), ClusterMetadataError> {
        self.require_catalog_generation(expected_catalog_generation)?;
        self.validate_snapshot(&snapshot)?;
        filesystem
            .restore_checkpoint(snapshot.checkpoint)
            .map_err(|_| ClusterMetadataError::NotRestorable)?;
        self.active_cluster = snapshot.active_cluster;
        self.clusters = snapshot.clusters;
        self.writer_epoch = snapshot.writer_epoch;
        self.writer = snapshot.writer;
        self.bump_catalog_generation()
    }

    pub fn release_snapshot<const BLOCKS: usize>(
        &mut self,
        filesystem: &mut synos_synfs::SynFs<BLOCKS>,
        snapshot: ClusterMetadataSnapshot,
    ) -> Result<(), ClusterMetadataError> {
        filesystem
            .release_checkpoint(snapshot.checkpoint)
            .map_err(|_| ClusterMetadataError::NotRestorable)
    }

    pub const fn encoded_len() -> usize {
        HEADER_BYTES + MAX_CLUSTERS * (CLUSTER_RECORD_BYTES + 1)
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, ClusterMetadataError> {
        let required = Self::encoded_len();
        if output.len() < required {
            return Err(ClusterMetadataError::BufferTooSmall { required });
        }
        output[..required].fill(0);
        output[..8].copy_from_slice(CLUSTER_METADATA_MAGIC);
        output[8..10].copy_from_slice(&CLUSTER_METADATA_FORMAT_VERSION.to_le_bytes());
        output[12..20].copy_from_slice(&self.catalog_generation.to_le_bytes());
        if let Some(id) = self.active_cluster {
            output[20] = 1;
            output[21..37].copy_from_slice(&id.raw());
        }
        output[40..48].copy_from_slice(&self.writer_epoch.to_le_bytes());
        output[48..64].copy_from_slice(&self.writer);
        let mut writer = Writer::new(&mut output[HEADER_BYTES..]);
        for cluster in self.clusters {
            match cluster {
                Some(cluster) => {
                    writer.put_u8(1)?;
                    encode_cluster(&mut writer, cluster)?;
                }
                None => {
                    writer.put_u8(0)?;
                    writer.put_zeros(CLUSTER_RECORD_BYTES);
                }
            }
        }
        let checksum = checksum_without_field(&output[..required]);
        output[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 8].copy_from_slice(&checksum.to_le_bytes());
        Ok(required)
    }

    pub fn decode(input: &[u8]) -> Result<Self, ClusterMetadataError> {
        let required = Self::encoded_len();
        if input.len() < required
            || &input[..8] != CLUSTER_METADATA_MAGIC
            || u16::from_le_bytes([input[8], input[9]]) != CLUSTER_METADATA_FORMAT_VERSION
        {
            return Err(ClusterMetadataError::Corrupt);
        }
        let stored_checksum = u64::from_le_bytes(
            input[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 8]
                .try_into()
                .map_err(|_| ClusterMetadataError::Corrupt)?,
        );
        if checksum_without_field(&input[..required]) != stored_checksum {
            return Err(ClusterMetadataError::Corrupt);
        }
        let catalog_generation = u64::from_le_bytes(
            input[12..20]
                .try_into()
                .map_err(|_| ClusterMetadataError::Corrupt)?,
        );
        if input[20] > 1 {
            return Err(ClusterMetadataError::Corrupt);
        }
        let active_cluster = match input[20] {
            0 => None,
            1 => Some(
                ClusterId::from_raw(
                    input[21..37]
                        .try_into()
                        .map_err(|_| ClusterMetadataError::Corrupt)?,
                )
                .ok_or(ClusterMetadataError::Corrupt)?,
            ),
            _ => return Err(ClusterMetadataError::Corrupt),
        };
        let writer_epoch = u64::from_le_bytes(
            input[40..48]
                .try_into()
                .map_err(|_| ClusterMetadataError::Corrupt)?,
        );
        let writer = input[48..64]
            .try_into()
            .map_err(|_| ClusterMetadataError::Corrupt)?;
        let mut reader = Reader::new(&input[HEADER_BYTES..]);
        let mut clusters = [None; MAX_CLUSTERS];
        for slot in &mut clusters {
            match reader.get_u8()? {
                0 => reader.skip(CLUSTER_RECORD_BYTES)?,
                1 => *slot = Some(decode_cluster(&mut reader)?),
                _ => return Err(ClusterMetadataError::Corrupt),
            }
        }
        let catalog = Self {
            catalog_generation,
            active_cluster,
            clusters,
            writer_epoch,
            writer,
        };
        catalog
            .validate()
            .map_err(|_| ClusterMetadataError::Corrupt)?;
        if catalog
            .active_cluster
            .is_some_and(|id| catalog.cluster(id).is_none())
        {
            return Err(ClusterMetadataError::Corrupt);
        }
        Ok(catalog)
    }

    pub fn export(&self, output: &mut [u8]) -> Result<usize, ClusterMetadataError> {
        self.encode(output)
    }

    pub fn import(input: &[u8]) -> Result<Self, ClusterMetadataError> {
        Self::decode(input)
    }

    pub fn save_to_synfs<const BLOCKS: usize>(
        &self,
        filesystem: &mut synos_synfs::SynFs<BLOCKS>,
        staging: &mut [u8],
    ) -> Result<synos_synfs::TransactionCommit, ClusterMetadataError> {
        let length = self.encode(staging)?;
        let mut transaction = filesystem.transaction();
        for directory in ["/system", "/system/cluster"] {
            match transaction.create_directory(directory, true) {
                Ok(_) | Err(synos_synfs::Error::AlreadyExists) => {}
                Err(_) => return Err(ClusterMetadataError::Persistence),
            }
        }
        transaction
            .write(CLUSTER_METADATA_STATE_FILE, &staging[..length])
            .map_err(|_| ClusterMetadataError::Persistence)?;
        transaction
            .commit()
            .map_err(|_| ClusterMetadataError::Persistence)
    }

    pub fn load_from_synfs<const BLOCKS: usize>(
        filesystem: &synos_synfs::SynFs<BLOCKS>,
        staging: &mut [u8],
    ) -> Result<Self, ClusterMetadataError> {
        if staging.len() < Self::encoded_len() {
            return Err(ClusterMetadataError::BufferTooSmall {
                required: Self::encoded_len(),
            });
        }
        let read = filesystem
            .read(CLUSTER_METADATA_STATE_FILE, staging)
            .map_err(map_synfs_error)?;
        Self::decode(&staging[..read.bytes_read])
    }

    fn require_catalog_generation(&self, expected: u64) -> Result<(), ClusterMetadataError> {
        if self.catalog_generation != expected {
            return Err(ClusterMetadataError::StaleGeneration);
        }
        Ok(())
    }

    fn bump_catalog_generation(&mut self) -> Result<(), ClusterMetadataError> {
        self.catalog_generation = self
            .catalog_generation
            .checked_add(1)
            .ok_or(ClusterMetadataError::Conflict)?;
        Ok(())
    }

    fn cluster_mut(&mut self, id: ClusterId) -> Result<&mut ClusterMetadata, ClusterMetadataError> {
        self.clusters
            .iter_mut()
            .flatten()
            .find(|cluster| cluster.id == id)
            .ok_or(ClusterMetadataError::NotFound)
    }

    fn validate_cluster(&self, cluster: &ClusterMetadata) -> Result<(), ClusterMetadataError> {
        if cluster.name.is_empty()
            || cluster.generation == 0
            || cluster.authority_epoch > 0 && cluster.authority == [0; NODE_ID_BYTES]
        {
            return Err(ClusterMetadataError::InvalidValue);
        }
        if cluster.aliases.iter().flatten().any(|alias| {
            alias.is_empty() || alias.as_str().eq_ignore_ascii_case(cluster.name.as_str())
        }) {
            return Err(ClusterMetadataError::Conflict);
        }
        for index in 0..MAX_CLUSTER_ALIASES {
            let Some(alias) = cluster.aliases[index] else {
                continue;
            };
            if cluster.aliases[index + 1..]
                .iter()
                .flatten()
                .any(|other| alias.as_str().eq_ignore_ascii_case(other.as_str()))
            {
                return Err(ClusterMetadataError::Conflict);
            }
        }
        for index in 0..MAX_TRUSTED_PEERS {
            let Some(peer) = cluster.trusted_peers[index] else {
                continue;
            };
            if peer.node_id.iter().all(|byte| *byte == 0)
                || cluster.trusted_peers[index + 1..]
                    .iter()
                    .flatten()
                    .any(|other| other.node_id == peer.node_id)
            {
                return Err(ClusterMetadataError::Conflict);
            }
        }
        for index in 0..MAX_INVITATIONS {
            let Some(invitation) = cluster.invitations[index] else {
                continue;
            };
            if invitation.cluster_id != cluster.id
                || invitation.token.iter().all(|byte| *byte == 0)
                || cluster.invitations[index + 1..]
                    .iter()
                    .flatten()
                    .any(|other| other.token == invitation.token)
            {
                return Err(ClusterMetadataError::Conflict);
            }
        }
        for index in 0..MAX_CERTIFICATES {
            let Some(certificate) = cluster.certificates[index] else {
                continue;
            };
            if certificate.fingerprint.iter().all(|byte| *byte == 0)
                || cluster.certificates[index + 1..]
                    .iter()
                    .flatten()
                    .any(|other| other.fingerprint == certificate.fingerprint)
            {
                return Err(ClusterMetadataError::Conflict);
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), ClusterMetadataError> {
        if self.writer_epoch > 0 && self.writer == [0; NODE_ID_BYTES] {
            return Err(ClusterMetadataError::Conflict);
        }
        for (index, left) in self.clusters.iter().enumerate() {
            let Some(left) = left else { continue };
            self.validate_cluster(left)?;
            for right in self.clusters[index + 1..].iter().flatten() {
                if left.id == right.id
                    || left.name.as_str().eq_ignore_ascii_case(right.name.as_str())
                {
                    return Err(ClusterMetadataError::Conflict);
                }
                if left.aliases.iter().flatten().any(|alias| {
                    alias.as_str().eq_ignore_ascii_case(right.name.as_str())
                        || right
                            .aliases
                            .iter()
                            .flatten()
                            .any(|other| alias.as_str().eq_ignore_ascii_case(other.as_str()))
                }) {
                    return Err(ClusterMetadataError::Conflict);
                }
                if right
                    .aliases
                    .iter()
                    .flatten()
                    .any(|alias| alias.as_str().eq_ignore_ascii_case(left.name.as_str()))
                {
                    return Err(ClusterMetadataError::Conflict);
                }
            }
        }
        if self
            .active_cluster
            .is_some_and(|id| self.cluster(id).is_none())
        {
            return Err(ClusterMetadataError::NotFound);
        }
        Ok(())
    }

    fn validate_snapshot(
        &self,
        snapshot: &ClusterMetadataSnapshot,
    ) -> Result<(), ClusterMetadataError> {
        let catalog = Self {
            catalog_generation: snapshot.catalog_generation,
            active_cluster: snapshot.active_cluster,
            clusters: snapshot.clusters,
            writer_epoch: snapshot.writer_epoch,
            writer: snapshot.writer,
        };
        catalog.validate()
    }
}

fn encode_cluster(
    writer: &mut Writer<'_>,
    cluster: ClusterMetadata,
) -> Result<(), ClusterMetadataError> {
    writer.put_bytes(&cluster.id.raw())?;
    writer.put_text(&cluster.name)?;
    writer.put_text(&cluster.description)?;
    writer.put_bytes(&cluster.owner)?;
    writer.put_u64(cluster.generation)?;
    writer.put_u64(cluster.created_at)?;
    writer.put_u8(cluster.lifecycle as u8)?;
    writer.put_u8(cluster.local_membership as u8)?;
    for alias in cluster.aliases {
        writer.put_option_text(alias)?;
    }
    for peer in cluster.trusted_peers {
        writer.put_u8(u8::from(peer.is_some()))?;
        if let Some(peer) = peer {
            writer.put_bytes(&peer.node_id)?;
            writer.put_bytes(&peer.certificate_id)?;
            writer.put_u8(peer.role)?;
            writer.put_u64(peer.last_seen_generation)?;
        } else {
            writer.put_zeros(41);
        }
    }
    for invitation in cluster.invitations {
        writer.put_u8(u8::from(invitation.is_some()))?;
        if let Some(invitation) = invitation {
            writer.put_bytes(&invitation.cluster_id.raw())?;
            writer.put_bytes(&invitation.token)?;
            writer.put_u64(invitation.expires_at)?;
            writer.put_u32(invitation.scope)?;
            writer.put_u8(u8::from(invitation.used))?;
            writer.put_u8(u8::from(invitation.revoked))?;
        } else {
            writer.put_zeros(62);
        }
    }
    for certificate in cluster.certificates {
        writer.put_u8(u8::from(certificate.is_some()))?;
        if let Some(certificate) = certificate {
            writer.put_bytes(&certificate.fingerprint)?;
            writer.put_u64(certificate.issued_at)?;
            writer.put_u64(certificate.expires_at)?;
            writer.put_u8(u8::from(certificate.revoked))?;
        } else {
            writer.put_zeros(49);
        }
    }
    writer.put_u64(cluster.authority_epoch)?;
    writer.put_bytes(&cluster.authority)?;
    Ok(())
}

fn decode_cluster(reader: &mut Reader<'_>) -> Result<ClusterMetadata, ClusterMetadataError> {
    let id = ClusterId::new(reader.get_array()?)?;
    let name = reader.get_text::<MAX_CLUSTER_NAME_BYTES>()?;
    let description = reader.get_text::<MAX_CLUSTER_DESCRIPTION_BYTES>()?;
    let owner = reader.get_array::<CLUSTER_ID_BYTES>()?;
    let generation = reader.get_u64()?;
    let created_at = reader.get_u64()?;
    let lifecycle =
        ClusterLifecycle::from_raw(reader.get_u8()?).ok_or(ClusterMetadataError::Corrupt)?;
    let local_membership =
        MembershipIntent::from_raw(reader.get_u8()?).ok_or(ClusterMetadataError::Corrupt)?;
    let mut aliases = [None; MAX_CLUSTER_ALIASES];
    for alias in &mut aliases {
        *alias = reader.get_option_text::<MAX_CLUSTER_NAME_BYTES>()?;
    }
    let mut trusted_peers = [None; MAX_TRUSTED_PEERS];
    for peer in &mut trusted_peers {
        match reader.get_u8()? {
            0 => reader.skip(41)?,
            1 => {
                *peer = Some(TrustedPeer {
                    node_id: reader.get_array()?,
                    certificate_id: reader.get_array()?,
                    role: reader.get_u8()?,
                    last_seen_generation: reader.get_u64()?,
                });
            }
            _ => return Err(ClusterMetadataError::Corrupt),
        }
    }
    let mut invitations = [None; MAX_INVITATIONS];
    for invitation in &mut invitations {
        match reader.get_u8()? {
            0 => reader.skip(62)?,
            1 => {
                *invitation = Some(Invitation {
                    cluster_id: ClusterId::new(reader.get_array()?)?,
                    token: reader.get_array()?,
                    expires_at: reader.get_u64()?,
                    scope: reader.get_u32()?,
                    used: reader.get_bool()?,
                    revoked: reader.get_bool()?,
                });
            }
            _ => return Err(ClusterMetadataError::Corrupt),
        }
    }
    let mut certificates = [None; MAX_CERTIFICATES];
    for certificate in &mut certificates {
        match reader.get_u8()? {
            0 => reader.skip(49)?,
            1 => {
                *certificate = Some(Certificate {
                    fingerprint: reader.get_array()?,
                    issued_at: reader.get_u64()?,
                    expires_at: reader.get_u64()?,
                    revoked: reader.get_bool()?,
                });
            }
            _ => return Err(ClusterMetadataError::Corrupt),
        }
    }
    Ok(ClusterMetadata {
        id,
        name,
        description,
        aliases,
        owner,
        generation,
        created_at,
        lifecycle,
        local_membership,
        trusted_peers,
        invitations,
        certificates,
        authority_epoch: reader.get_u64()?,
        authority: reader.get_array()?,
    })
}

struct Writer<'a> {
    bytes: &'a mut [u8],
    position: usize,
}

impl Writer<'_> {
    fn new(bytes: &mut [u8]) -> Writer<'_> {
        Writer { bytes, position: 0 }
    }

    fn put_u8(&mut self, value: u8) -> Result<(), ClusterMetadataError> {
        self.put_bytes(&[value])
    }

    fn put_u32(&mut self, value: u32) -> Result<(), ClusterMetadataError> {
        self.put_bytes(&value.to_le_bytes())
    }

    fn put_u64(&mut self, value: u64) -> Result<(), ClusterMetadataError> {
        self.put_bytes(&value.to_le_bytes())
    }

    fn put_text<const CAPACITY: usize>(
        &mut self,
        value: &MetadataText<CAPACITY>,
    ) -> Result<(), ClusterMetadataError> {
        self.put_bytes(&value.len.to_le_bytes())?;
        self.put_bytes(&value.bytes)
    }

    fn put_option_text<const CAPACITY: usize>(
        &mut self,
        value: Option<MetadataText<CAPACITY>>,
    ) -> Result<(), ClusterMetadataError> {
        self.put_u8(u8::from(value.is_some()))?;
        match value {
            Some(value) => self.put_text(&value),
            None => {
                self.put_zeros(2 + CAPACITY);
                Ok(())
            }
        }
    }

    fn put_zeros(&mut self, length: usize) {
        let end = self.position + length;
        self.bytes[self.position..end].fill(0);
        self.position = end;
    }

    fn put_bytes(&mut self, value: &[u8]) -> Result<(), ClusterMetadataError> {
        let end = self
            .position
            .checked_add(value.len())
            .ok_or(ClusterMetadataError::Corrupt)?;
        let destination = self
            .bytes
            .get_mut(self.position..end)
            .ok_or(ClusterMetadataError::Corrupt)?;
        destination.copy_from_slice(value);
        self.position = end;
        Ok(())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Reader<'_> {
    fn new(bytes: &[u8]) -> Reader<'_> {
        Reader { bytes, position: 0 }
    }

    fn get_u8(&mut self) -> Result<u8, ClusterMetadataError> {
        let value = *self
            .bytes
            .get(self.position)
            .ok_or(ClusterMetadataError::Corrupt)?;
        self.position += 1;
        Ok(value)
    }

    fn get_u32(&mut self) -> Result<u32, ClusterMetadataError> {
        Ok(u32::from_le_bytes(self.get_array()?))
    }

    fn get_bool(&mut self) -> Result<bool, ClusterMetadataError> {
        match self.get_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ClusterMetadataError::Corrupt),
        }
    }

    fn get_u64(&mut self) -> Result<u64, ClusterMetadataError> {
        Ok(u64::from_le_bytes(self.get_array()?))
    }

    fn get_array<const LENGTH: usize>(&mut self) -> Result<[u8; LENGTH], ClusterMetadataError> {
        let end = self
            .position
            .checked_add(LENGTH)
            .ok_or(ClusterMetadataError::Corrupt)?;
        let source = self
            .bytes
            .get(self.position..end)
            .ok_or(ClusterMetadataError::Corrupt)?;
        let mut value = [0; LENGTH];
        value.copy_from_slice(source);
        self.position = end;
        Ok(value)
    }

    fn get_text<const CAPACITY: usize>(
        &mut self,
    ) -> Result<MetadataText<CAPACITY>, ClusterMetadataError> {
        let length = u16::from_le_bytes(self.get_array()?);
        if length as usize > CAPACITY {
            return Err(ClusterMetadataError::Corrupt);
        }
        let bytes = self.get_array::<CAPACITY>()?;
        let value = core::str::from_utf8(&bytes[..length as usize])
            .map_err(|_| ClusterMetadataError::Corrupt)?;
        MetadataText::new(value)
    }

    fn get_option_text<const CAPACITY: usize>(
        &mut self,
    ) -> Result<Option<MetadataText<CAPACITY>>, ClusterMetadataError> {
        let present = self.get_u8()?;
        if present > 1 {
            return Err(ClusterMetadataError::Corrupt);
        }
        if present == 0 {
            self.skip(2 + CAPACITY)?;
            Ok(None)
        } else {
            Ok(Some(self.get_text()?))
        }
    }

    fn skip(&mut self, length: usize) -> Result<(), ClusterMetadataError> {
        self.position = self
            .position
            .checked_add(length)
            .filter(|position| *position <= self.bytes.len())
            .ok_or(ClusterMetadataError::Corrupt)?;
        Ok(())
    }
}

fn checksum_without_field(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for (index, byte) in bytes.iter().enumerate() {
        if (CHECKSUM_OFFSET..CHECKSUM_OFFSET + 8).contains(&index) {
            continue;
        }
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn map_synfs_error(error: synos_synfs::Error) -> ClusterMetadataError {
    match error {
        synos_synfs::Error::NotFound => ClusterMetadataError::NotFound,
        synos_synfs::Error::Corrupt => ClusterMetadataError::Corrupt,
        synos_synfs::Error::BufferTooSmall { required } => {
            ClusterMetadataError::BufferTooSmall { required }
        }
        _ => ClusterMetadataError::Persistence,
    }
}
