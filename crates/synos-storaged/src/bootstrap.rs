use synos_status::{IntoStatus, Status};
use synos_system_model::ContentId;

use crate::cluster::{
    CLUSTER_ID_BYTES, ClusterId, ClusterLifecycle, ClusterMetadata, ClusterMetadataCatalog,
    ClusterMetadataError, MAX_CLUSTER_DESCRIPTION_BYTES, MAX_CLUSTER_NAME_BYTES, MembershipIntent,
    MetadataText, NODE_ID_BYTES, TOKEN_BYTES,
};

pub const CLUSTER_BOOTSTRAP_STATE_FILE: &str = "/system/cluster/bootstrap.dat";
pub const CLUSTER_BOOTSTRAP_MAGIC: &[u8; 8] = b"SYNBOOT1";
pub const CLUSTER_BOOTSTRAP_FORMAT_VERSION: u16 = 1;
pub const MAX_BOOTSTRAP_ENDPOINT_BYTES: usize = 96;
pub const BOOTSTRAP_SIGNATURE_BYTES: usize = 32;
pub const BOOTSTRAP_ENCODED_BYTES: usize = 576;
pub const MAX_CLOCK_OFFSET_US: i64 = 500_000;
pub const MAX_CLOCK_UNCERTAINTY_US: u64 = 1_000_000;

const CHECKSUM_OFFSET: usize = BOOTSTRAP_ENCODED_BYTES - 8;
const REQUIRED_CAPABILITIES: u32 = NodeCapabilities::CONTROL_PLANE.bits()
    | NodeCapabilities::PERSISTENT_METADATA.bits()
    | NodeCapabilities::NETWORK.bits()
    | NodeCapabilities::CLOCK.bits();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct NodeCapabilities(u32);

impl NodeCapabilities {
    pub const CONTROL_PLANE: Self = Self(1 << 0);
    pub const PERSISTENT_METADATA: Self = Self(1 << 1);
    pub const NETWORK: Self = Self(1 << 2);
    pub const CLOCK: Self = Self(1 << 3);
    pub const STORAGE: Self = Self(1 << 4);
    pub const MEMORY: Self = Self(1 << 5);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn with(self, capability: Self) -> Self {
        Self(self.0 | capability.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct TransportSet(u8);

impl TransportSet {
    pub const LOOPBACK: Self = Self(1 << 0);
    pub const ETHERNET: Self = Self(1 << 1);
    pub const CXL: Self = Self(1 << 2);
    pub const WIRELESS: Self = Self(1 << 3);
    pub const TUNNEL: Self = Self(1 << 4);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn with(self, transport: Self) -> Self {
        Self(self.0 | transport.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AdmissionPolicy {
    Open = 1,
    Invitation = 2,
    Attested = 3,
}

impl AdmissionPolicy {
    fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::Open,
            2 => Self::Invitation,
            3 => Self::Attested,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuorumPolicy {
    pub voting_members: u16,
    pub required_votes: u16,
}

impl QuorumPolicy {
    pub const SINGLE_NODE: Self = Self {
        voting_members: 1,
        required_votes: 1,
    };

    fn validate(self) -> Result<(), ClusterBootstrapError> {
        if self.voting_members == 0
            || self.required_votes == 0
            || self.required_votes > self.voting_members
        {
            return Err(ClusterBootstrapError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockHealth {
    pub synchronized: bool,
    pub offset_us: i64,
    pub uncertainty_us: u64,
}

impl ClockHealth {
    pub const HEALTHY: Self = Self {
        synchronized: true,
        offset_us: 0,
        uncertainty_us: 0,
    };

    pub const fn is_healthy(self) -> bool {
        self.synchronized
            && self.offset_us.unsigned_abs() <= MAX_CLOCK_OFFSET_US as u64
            && self.uncertainty_us <= MAX_CLOCK_UNCERTAINTY_US
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttestationEvidence {
    pub measurement: [u8; BOOTSTRAP_SIGNATURE_BYTES],
    pub verified: bool,
}

impl AttestationEvidence {
    pub const NONE: Self = Self {
        measurement: [0; BOOTSTRAP_SIGNATURE_BYTES],
        verified: false,
    };

    fn is_valid(self) -> bool {
        self.verified && !is_zero(&self.measurement)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolCompatibility {
    pub control_version: u16,
    pub data_version: u16,
    pub minimum_version: u16,
}

impl ProtocolCompatibility {
    fn validate(self) -> Result<(), ClusterBootstrapError> {
        if self.control_version == 0
            || self.data_version == 0
            || self.minimum_version == 0
            || self.minimum_version > self.control_version
            || self.minimum_version > self.data_version
        {
            return Err(ClusterBootstrapError::ProtocolMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterCreateRequest {
    pub requested_cluster_id: Option<ClusterId>,
    pub name: MetadataText<MAX_CLUSTER_NAME_BYTES>,
    pub description: MetadataText<MAX_CLUSTER_DESCRIPTION_BYTES>,
    pub node_id: [u8; NODE_ID_BYTES],
    pub now_us: u64,
    pub endpoint: MetadataText<MAX_BOOTSTRAP_ENDPOINT_BYTES>,
    pub capabilities: NodeCapabilities,
    pub transports: TransportSet,
    pub protocols: ProtocolCompatibility,
    pub admission_policy: AdmissionPolicy,
    pub quorum: QuorumPolicy,
    pub attestation: AttestationEvidence,
    pub clock: ClockHealth,
}

impl ClusterCreateRequest {
    pub fn new(
        name: MetadataText<MAX_CLUSTER_NAME_BYTES>,
        node_id: [u8; NODE_ID_BYTES],
        now_us: u64,
    ) -> Self {
        Self {
            requested_cluster_id: None,
            name,
            description: MetadataText::empty(),
            node_id,
            now_us,
            endpoint: MetadataText::empty(),
            capabilities: NodeCapabilities::CONTROL_PLANE
                .with(NodeCapabilities::PERSISTENT_METADATA)
                .with(NodeCapabilities::NETWORK)
                .with(NodeCapabilities::CLOCK),
            transports: TransportSet::LOOPBACK,
            protocols: ProtocolCompatibility {
                control_version: 1,
                data_version: 1,
                minimum_version: 1,
            },
            admission_policy: AdmissionPolicy::Invitation,
            quorum: QuorumPolicy::SINGLE_NODE,
            attestation: AttestationEvidence::NONE,
            clock: ClockHealth::HEALTHY,
        }
    }

    fn validate(&self) -> Result<(), ClusterBootstrapError> {
        if self.name.is_empty() || is_zero(&self.node_id) || self.transports.bits() == 0 {
            return Err(ClusterBootstrapError::InvalidConfiguration);
        }
        if !self.transports.contains(TransportSet::LOOPBACK) && self.endpoint.is_empty() {
            return Err(ClusterBootstrapError::TransportUnavailable);
        }
        self.protocols.validate()?;
        self.quorum.validate()?;
        Ok(())
    }

    fn fingerprint(self) -> [u8; BOOTSTRAP_SIGNATURE_BYTES] {
        let mut material = [0; 512];
        let mut writer = Writer::new(&mut material);
        writer.put_optional_id(self.requested_cluster_id);
        writer.put_text(&self.name);
        writer.put_text(&self.description);
        writer.put_bytes(&self.node_id);
        writer.put_text(&self.endpoint);
        writer.put_u32(self.capabilities.bits());
        writer.put_u8(self.transports.bits());
        writer.put_u16(self.protocols.control_version);
        writer.put_u16(self.protocols.data_version);
        writer.put_u16(self.protocols.minimum_version);
        writer.put_u8(self.admission_policy as u8);
        writer.put_u16(self.quorum.voting_members);
        writer.put_u16(self.quorum.required_votes);
        writer.put_u8(u8::from(self.attestation.verified));
        writer.put_bytes(&self.attestation.measurement);
        writer.put_u8(u8::from(self.clock.synchronized));
        writer.put_i64(self.clock.offset_us);
        writer.put_u64(self.clock.uncertainty_us);
        *ContentId::hash(writer.bytes()).as_bytes()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootstrapChecks {
    pub node_id: [u8; NODE_ID_BYTES],
    pub capabilities: NodeCapabilities,
    pub transports: TransportSet,
    pub protocols: ProtocolCompatibility,
    pub clock: ClockHealth,
    pub attestation: AttestationEvidence,
    pub quorum_available: u16,
}

impl BootstrapChecks {
    pub fn from_request(request: &ClusterCreateRequest) -> Self {
        Self {
            node_id: request.node_id,
            capabilities: request.capabilities,
            transports: request.transports,
            protocols: request.protocols,
            clock: request.clock,
            attestation: request.attestation,
            quorum_available: request.quorum.voting_members.min(1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BootstrapLifecycle {
    Creating = 1,
    Active = 2,
    Degraded = 3,
    Retired = 4,
}

impl BootstrapLifecycle {
    fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::Creating,
            2 => Self::Active,
            3 => Self::Degraded,
            4 => Self::Retired,
            _ => return None,
        })
    }

    const fn metadata(self) -> ClusterLifecycle {
        match self {
            Self::Creating => ClusterLifecycle::Creating,
            Self::Active => ClusterLifecycle::Active,
            Self::Degraded => ClusterLifecycle::Degraded,
            Self::Retired => ClusterLifecycle::Retired,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootstrapToken {
    pub token: [u8; TOKEN_BYTES],
    pub expires_at_us: u64,
    pub scope: u32,
    pub used: bool,
    pub revoked: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootstrapNodeRecord {
    pub node_id: [u8; NODE_ID_BYTES],
    pub capabilities: NodeCapabilities,
    pub transports: TransportSet,
    pub protocols: ProtocolCompatibility,
    pub attestation: AttestationEvidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterRootIdentity {
    cluster_id: ClusterId,
    key_id: [u8; CLUSTER_ID_BYTES],
    key: [u8; BOOTSTRAP_SIGNATURE_BYTES],
    epoch: u64,
}

impl ClusterRootIdentity {
    pub const fn cluster_id(self) -> ClusterId {
        self.cluster_id
    }

    pub const fn key_id(self) -> [u8; CLUSTER_ID_BYTES] {
        self.key_id
    }

    pub const fn epoch(self) -> u64 {
        self.epoch
    }

    fn sign(self, material: &[u8]) -> [u8; BOOTSTRAP_SIGNATURE_BYTES] {
        hmac_sha256(&self.key, material)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootstrapAdvertisement {
    pub cluster_id: ClusterId,
    pub node_id: [u8; NODE_ID_BYTES],
    pub root_key_id: [u8; CLUSTER_ID_BYTES],
    pub sequence: u64,
    pub expires_at_us: u64,
    pub endpoint: MetadataText<MAX_BOOTSTRAP_ENDPOINT_BYTES>,
    pub transports: TransportSet,
    pub protocols: ProtocolCompatibility,
    pub signature: [u8; BOOTSTRAP_SIGNATURE_BYTES],
}

impl BootstrapAdvertisement {
    pub fn verify(
        &self,
        root: ClusterRootIdentity,
        now_us: u64,
    ) -> Result<(), ClusterBootstrapError> {
        if self.cluster_id != root.cluster_id
            || self.root_key_id != root.key_id
            || self.expires_at_us <= now_us
        {
            return Err(ClusterBootstrapError::AdvertisementExpired);
        }
        let expected = root.sign(&self.material());
        if constant_time_equal(&expected, &self.signature) {
            Ok(())
        } else {
            Err(ClusterBootstrapError::SignatureMismatch)
        }
    }

    fn material(&self) -> [u8; 176] {
        let mut material = [0; 176];
        let mut writer = Writer::new(&mut material);
        writer.put_bytes(&self.cluster_id.raw());
        writer.put_bytes(&self.node_id);
        writer.put_bytes(&self.root_key_id);
        writer.put_u64(self.sequence);
        writer.put_u64(self.expires_at_us);
        writer.put_text(&self.endpoint);
        writer.put_u8(self.transports.bits());
        writer.put_u16(self.protocols.control_version);
        writer.put_u16(self.protocols.data_version);
        writer.put_u16(self.protocols.minimum_version);
        material
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterBootstrapError {
    AdvertisementExpired,
    AlreadyExists,
    BufferTooSmall { required: usize },
    CapabilityUnavailable,
    ClockUnhealthy,
    Conflict,
    Corrupt,
    InvalidConfiguration,
    InvalidToken,
    NotFound,
    Persistence,
    ProtocolMismatch,
    QuorumUnavailable,
    Revoked,
    SignatureMismatch,
    TransportUnavailable,
    AttestationRequired,
}

impl IntoStatus for ClusterBootstrapError {
    fn status(self) -> Status {
        match self {
            Self::BufferTooSmall { .. } => Status::NO_SPACE,
            Self::AlreadyExists => Status::ALREADY_EXISTS,
            Self::CapabilityUnavailable
            | Self::ClockUnhealthy
            | Self::ProtocolMismatch
            | Self::QuorumUnavailable
            | Self::TransportUnavailable
            | Self::AttestationRequired
            | Self::Revoked
            | Self::InvalidToken => Status::ACCESS_DENIED,
            Self::Conflict => Status::CONFLICT,
            Self::Corrupt => Status::CORRUPT,
            Self::NotFound => Status::NOT_FOUND,
            Self::Persistence => Status::BUSY,
            Self::SignatureMismatch => Status::ACCESS_DENIED,
            Self::InvalidConfiguration | Self::AdvertisementExpired => Status::INVALID_ARGUMENT,
        }
    }
}

pub trait EntropySource {
    fn fill_bytes(&mut self, output: &mut [u8]) -> Result<(), ClusterBootstrapError>;
}

/// Deterministic source for boot tests. Production passes a hardware-backed source.
pub struct SeedEntropy(u64);

impl SeedEntropy {
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }
}

impl EntropySource for SeedEntropy {
    fn fill_bytes(&mut self, output: &mut [u8]) -> Result<(), ClusterBootstrapError> {
        if self.0 == 0 {
            return Err(ClusterBootstrapError::InvalidConfiguration);
        }
        for byte in output {
            self.0 ^= self.0 << 7;
            self.0 ^= self.0 >> 9;
            self.0 ^= self.0 << 8;
            *byte = self.0 as u8;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct ClusterBootstrapState {
    pub cluster_id: ClusterId,
    pub name: MetadataText<MAX_CLUSTER_NAME_BYTES>,
    pub description: MetadataText<MAX_CLUSTER_DESCRIPTION_BYTES>,
    pub node: BootstrapNodeRecord,
    pub root: ClusterRootIdentity,
    pub admission_policy: AdmissionPolicy,
    pub attestation_required: bool,
    pub quorum: QuorumPolicy,
    pub bootstrap_token: BootstrapToken,
    pub endpoint: MetadataText<MAX_BOOTSTRAP_ENDPOINT_BYTES>,
    pub lifecycle: BootstrapLifecycle,
    pub generation: u64,
    pub created_at_us: u64,
    pub advertisement_sequence: u64,
    pub credential_epoch: u64,
    request_fingerprint: [u8; BOOTSTRAP_SIGNATURE_BYTES],
}

impl core::fmt::Debug for ClusterBootstrapState {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ClusterBootstrapState")
            .field("cluster_id", &self.cluster_id)
            .field("name", &self.name.as_str())
            .field("lifecycle", &self.lifecycle)
            .field("generation", &self.generation)
            .field("credential_epoch", &self.credential_epoch)
            .finish()
    }
}

impl PartialEq for ClusterBootstrapState {
    fn eq(&self, other: &Self) -> bool {
        self.cluster_id == other.cluster_id
            && self.name == other.name
            && self.description == other.description
            && self.node == other.node
            && self.root == other.root
            && self.admission_policy == other.admission_policy
            && self.attestation_required == other.attestation_required
            && self.quorum == other.quorum
            && self.bootstrap_token == other.bootstrap_token
            && self.endpoint == other.endpoint
            && self.lifecycle == other.lifecycle
            && self.generation == other.generation
            && self.created_at_us == other.created_at_us
            && self.advertisement_sequence == other.advertisement_sequence
            && self.credential_epoch == other.credential_epoch
            && self.request_fingerprint == other.request_fingerprint
    }
}

impl Eq for ClusterBootstrapState {}

impl ClusterBootstrapState {
    pub fn create(
        request: ClusterCreateRequest,
        entropy: &mut impl EntropySource,
    ) -> Result<Self, ClusterBootstrapError> {
        request.validate()?;
        let mut random = [0; CLUSTER_ID_BYTES + BOOTSTRAP_SIGNATURE_BYTES + TOKEN_BYTES];
        entropy.fill_bytes(&mut random)?;
        let mut id_bytes = [0; CLUSTER_ID_BYTES];
        id_bytes.copy_from_slice(&random[..CLUSTER_ID_BYTES]);
        let cluster_id = request
            .requested_cluster_id
            .or_else(|| ClusterId::from_raw(id_bytes))
            .ok_or(ClusterBootstrapError::InvalidConfiguration)?;
        let mut key = [0; BOOTSTRAP_SIGNATURE_BYTES];
        key.copy_from_slice(
            &random[CLUSTER_ID_BYTES..CLUSTER_ID_BYTES + BOOTSTRAP_SIGNATURE_BYTES],
        );
        let mut token = [0; TOKEN_BYTES];
        token.copy_from_slice(&random[CLUSTER_ID_BYTES + BOOTSTRAP_SIGNATURE_BYTES..]);
        if is_zero(&key) || is_zero(&token) {
            return Err(ClusterBootstrapError::InvalidConfiguration);
        }
        let key_hash = ContentId::hash(&key);
        let mut key_id = [0; CLUSTER_ID_BYTES];
        key_id.copy_from_slice(&key_hash.as_bytes()[..CLUSTER_ID_BYTES]);
        let token_expiration = request
            .now_us
            .checked_add(86_400_000_000)
            .ok_or(ClusterBootstrapError::InvalidConfiguration)?;
        Ok(Self {
            cluster_id,
            name: request.name,
            description: request.description,
            node: BootstrapNodeRecord {
                node_id: request.node_id,
                capabilities: request.capabilities,
                transports: request.transports,
                protocols: request.protocols,
                attestation: request.attestation,
            },
            root: ClusterRootIdentity {
                cluster_id,
                key_id,
                key,
                epoch: 1,
            },
            admission_policy: request.admission_policy,
            attestation_required: request.admission_policy == AdmissionPolicy::Attested,
            quorum: request.quorum,
            bootstrap_token: BootstrapToken {
                token,
                expires_at_us: token_expiration,
                scope: u32::MAX,
                used: false,
                revoked: false,
            },
            endpoint: request.endpoint,
            lifecycle: BootstrapLifecycle::Creating,
            generation: 1,
            created_at_us: request.now_us,
            advertisement_sequence: 0,
            credential_epoch: 1,
            request_fingerprint: request.fingerprint(),
        })
    }

    pub fn create_or_resume(
        existing: Option<&Self>,
        request: ClusterCreateRequest,
        entropy: &mut impl EntropySource,
    ) -> Result<Self, ClusterBootstrapError> {
        request.validate()?;
        if let Some(existing) = existing {
            if existing.request_fingerprint == request.fingerprint() {
                return Ok(*existing);
            }
            return Err(ClusterBootstrapError::Conflict);
        }
        Self::create(request, entropy)
    }

    pub fn validate_readiness(&self, checks: BootstrapChecks) -> Result<(), ClusterBootstrapError> {
        if checks.node_id != self.node.node_id {
            return Err(ClusterBootstrapError::Conflict);
        }
        if !checks
            .capabilities
            .contains(NodeCapabilities::from_bits(REQUIRED_CAPABILITIES))
        {
            return Err(ClusterBootstrapError::CapabilityUnavailable);
        }
        if !checks.transports.intersects(self.node.transports) {
            return Err(ClusterBootstrapError::TransportUnavailable);
        }
        checks.protocols.validate()?;
        if checks.protocols.control_version < self.node.protocols.minimum_version
            || checks.protocols.data_version < self.node.protocols.minimum_version
            || checks.protocols.minimum_version > self.node.protocols.control_version
            || checks.protocols.minimum_version > self.node.protocols.data_version
        {
            return Err(ClusterBootstrapError::ProtocolMismatch);
        }
        if !checks.clock.is_healthy() {
            return Err(ClusterBootstrapError::ClockUnhealthy);
        }
        if self.attestation_required && !checks.attestation.is_valid() {
            return Err(ClusterBootstrapError::AttestationRequired);
        }
        if checks.quorum_available < self.quorum.required_votes {
            return Err(ClusterBootstrapError::QuorumUnavailable);
        }
        Ok(())
    }

    pub fn activate(&mut self, checks: BootstrapChecks) -> Result<(), ClusterBootstrapError> {
        if self.lifecycle == BootstrapLifecycle::Retired {
            return Err(ClusterBootstrapError::Conflict);
        }
        self.validate_readiness(checks)?;
        if self.lifecycle != BootstrapLifecycle::Active {
            self.lifecycle = BootstrapLifecycle::Active;
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or(ClusterBootstrapError::Conflict)?;
        }
        Ok(())
    }

    pub fn metadata(&self) -> ClusterMetadata {
        ClusterMetadata {
            id: self.cluster_id,
            name: self.name,
            description: self.description,
            aliases: [None; crate::cluster::MAX_CLUSTER_ALIASES],
            owner: self.root.key_id,
            generation: self.generation,
            created_at: self.created_at_us,
            lifecycle: self.lifecycle.metadata(),
            local_membership: MembershipIntent::Create,
            trusted_peers: [None; crate::cluster::MAX_TRUSTED_PEERS],
            invitations: [None; crate::cluster::MAX_INVITATIONS],
            certificates: [None; crate::cluster::MAX_CERTIFICATES],
            authority_epoch: self.root.epoch,
            authority: self.node.node_id,
        }
    }

    pub fn install_in_catalog(
        &self,
        catalog: &mut ClusterMetadataCatalog,
        expected_catalog_generation: u64,
    ) -> Result<(), ClusterBootstrapError> {
        if let Some(existing) = catalog.cluster(self.cluster_id) {
            if existing.name == self.name && existing.owner == self.root.key_id {
                if catalog.active_cluster() == Some(self.cluster_id) {
                    return Ok(());
                }
                catalog
                    .set_active_cluster(self.cluster_id, expected_catalog_generation)
                    .map_err(map_metadata_error)?;
                return Ok(());
            }
            return Err(ClusterBootstrapError::Conflict);
        }
        catalog
            .create_cluster(self.metadata(), expected_catalog_generation)
            .map_err(map_metadata_error)?;
        catalog
            .set_active_cluster(self.cluster_id, catalog.catalog_generation())
            .map_err(map_metadata_error)
    }

    pub fn consume_bootstrap_token(
        &mut self,
        token: [u8; TOKEN_BYTES],
        now_us: u64,
        required_scope: u32,
    ) -> Result<(), ClusterBootstrapError> {
        if self.bootstrap_token.token != token || self.bootstrap_token.used {
            return Err(ClusterBootstrapError::InvalidToken);
        }
        if self.bootstrap_token.revoked {
            return Err(ClusterBootstrapError::Revoked);
        }
        if now_us >= self.bootstrap_token.expires_at_us
            || self.bootstrap_token.scope & required_scope != required_scope
        {
            return Err(ClusterBootstrapError::InvalidToken);
        }
        self.bootstrap_token.used = true;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(ClusterBootstrapError::Conflict)?;
        Ok(())
    }

    pub fn rotate_bootstrap_credentials(
        &mut self,
        entropy: &mut impl EntropySource,
        expires_at_us: u64,
    ) -> Result<(), ClusterBootstrapError> {
        if expires_at_us <= self.created_at_us {
            return Err(ClusterBootstrapError::InvalidConfiguration);
        }
        let mut token = [0; TOKEN_BYTES];
        entropy.fill_bytes(&mut token)?;
        if is_zero(&token) {
            return Err(ClusterBootstrapError::InvalidConfiguration);
        }
        self.bootstrap_token = BootstrapToken {
            token,
            expires_at_us,
            scope: u32::MAX,
            used: false,
            revoked: false,
        };
        self.credential_epoch = self
            .credential_epoch
            .checked_add(1)
            .ok_or(ClusterBootstrapError::Conflict)?;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(ClusterBootstrapError::Conflict)?;
        Ok(())
    }

    pub fn revoke_bootstrap_credentials(&mut self) -> Result<(), ClusterBootstrapError> {
        if self.bootstrap_token.revoked {
            return Ok(());
        }
        self.bootstrap_token.revoked = true;
        self.credential_epoch = self
            .credential_epoch
            .checked_add(1)
            .ok_or(ClusterBootstrapError::Conflict)?;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(ClusterBootstrapError::Conflict)?;
        Ok(())
    }

    pub fn publish_advertisement(
        &mut self,
        now_us: u64,
        lifetime_us: u64,
    ) -> Result<BootstrapAdvertisement, ClusterBootstrapError> {
        if self.lifecycle != BootstrapLifecycle::Active || lifetime_us == 0 {
            return Err(ClusterBootstrapError::Conflict);
        }
        let expires_at_us = now_us
            .checked_add(lifetime_us)
            .ok_or(ClusterBootstrapError::InvalidConfiguration)?;
        self.advertisement_sequence = self
            .advertisement_sequence
            .checked_add(1)
            .ok_or(ClusterBootstrapError::Conflict)?;
        let mut advertisement = BootstrapAdvertisement {
            cluster_id: self.cluster_id,
            node_id: self.node.node_id,
            root_key_id: self.root.key_id,
            sequence: self.advertisement_sequence,
            expires_at_us,
            endpoint: self.endpoint,
            transports: self.node.transports,
            protocols: self.node.protocols,
            signature: [0; BOOTSTRAP_SIGNATURE_BYTES],
        };
        advertisement.signature = self.root.sign(&advertisement.material());
        Ok(advertisement)
    }

    pub const fn encoded_len() -> usize {
        BOOTSTRAP_ENCODED_BYTES
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, ClusterBootstrapError> {
        if output.len() < Self::encoded_len() {
            return Err(ClusterBootstrapError::BufferTooSmall {
                required: Self::encoded_len(),
            });
        }
        output[..Self::encoded_len()].fill(0);
        output[..8].copy_from_slice(CLUSTER_BOOTSTRAP_MAGIC);
        output[8..10].copy_from_slice(&CLUSTER_BOOTSTRAP_FORMAT_VERSION.to_le_bytes());
        let mut writer = Writer::new(&mut output[10..CHECKSUM_OFFSET]);
        writer.put_u8(self.lifecycle as u8);
        writer.put_u8(self.admission_policy as u8);
        writer.put_u8(u8::from(self.attestation_required));
        writer.put_u8(self.node.transports.bits());
        writer.put_u32(self.node.capabilities.bits());
        writer.put_u16(self.node.protocols.control_version);
        writer.put_u16(self.node.protocols.data_version);
        writer.put_u16(self.node.protocols.minimum_version);
        writer.put_u16(self.quorum.voting_members);
        writer.put_u16(self.quorum.required_votes);
        writer.put_u64(self.generation);
        writer.put_u64(self.created_at_us);
        writer.put_u64(self.advertisement_sequence);
        writer.put_u64(self.credential_epoch);
        writer.put_bytes(&self.cluster_id.raw());
        writer.put_bytes(&self.name.raw_bytes());
        writer.put_u16(self.name.len());
        writer.put_bytes(&self.description.raw_bytes());
        writer.put_u16(self.description.len());
        writer.put_bytes(&self.node.node_id);
        writer.put_bytes(&self.node.attestation.measurement);
        writer.put_u8(u8::from(self.node.attestation.verified));
        writer.put_text(&self.endpoint);
        writer.put_bytes(&self.root.key_id);
        writer.put_bytes(&self.root.key);
        writer.put_u64(self.root.epoch);
        writer.put_bytes(&self.bootstrap_token.token);
        writer.put_u64(self.bootstrap_token.expires_at_us);
        writer.put_u32(self.bootstrap_token.scope);
        writer.put_u8(u8::from(self.bootstrap_token.used));
        writer.put_u8(u8::from(self.bootstrap_token.revoked));
        writer.put_bytes(&self.request_fingerprint);
        let digest = checksum(&output[..CHECKSUM_OFFSET]);
        output[CHECKSUM_OFFSET..].copy_from_slice(&digest.to_le_bytes());
        Ok(Self::encoded_len())
    }

    pub fn decode(input: &[u8]) -> Result<Self, ClusterBootstrapError> {
        let checksum_bytes: [u8; 8] = input
            .get(CHECKSUM_OFFSET..Self::encoded_len())
            .ok_or(ClusterBootstrapError::Corrupt)?
            .try_into()
            .map_err(|_| ClusterBootstrapError::Corrupt)?;
        if input.len() < Self::encoded_len()
            || &input[..8] != CLUSTER_BOOTSTRAP_MAGIC
            || u16::from_le_bytes([input[8], input[9]]) != CLUSTER_BOOTSTRAP_FORMAT_VERSION
            || u64::from_le_bytes(checksum_bytes) != checksum(&input[..CHECKSUM_OFFSET])
        {
            return Err(ClusterBootstrapError::Corrupt);
        }
        let mut reader = Reader::new(&input[10..CHECKSUM_OFFSET]);
        let lifecycle =
            BootstrapLifecycle::from_raw(reader.get_u8()?).ok_or(ClusterBootstrapError::Corrupt)?;
        let admission_policy =
            AdmissionPolicy::from_raw(reader.get_u8()?).ok_or(ClusterBootstrapError::Corrupt)?;
        let attestation_required = reader.get_bool()?;
        let transports = TransportSet::from_bits(reader.get_u8()?);
        let capabilities = NodeCapabilities::from_bits(reader.get_u32()?);
        let protocols = ProtocolCompatibility {
            control_version: reader.get_u16()?,
            data_version: reader.get_u16()?,
            minimum_version: reader.get_u16()?,
        };
        let quorum = QuorumPolicy {
            voting_members: reader.get_u16()?,
            required_votes: reader.get_u16()?,
        };
        let generation = reader.get_u64()?;
        let created_at_us = reader.get_u64()?;
        let advertisement_sequence = reader.get_u64()?;
        let credential_epoch = reader.get_u64()?;
        let cluster_id = ClusterId::new(reader.get_array()?).map_err(map_metadata_error)?;
        let name_bytes = reader.get_array::<MAX_CLUSTER_NAME_BYTES>()?;
        let name_len = reader.get_u16()? as usize;
        let description_bytes = reader.get_array::<MAX_CLUSTER_DESCRIPTION_BYTES>()?;
        let description_len = reader.get_u16()? as usize;
        let node_id = reader.get_array()?;
        let measurement = reader.get_array()?;
        let attested = reader.get_bool()?;
        let endpoint = reader.get_text::<MAX_BOOTSTRAP_ENDPOINT_BYTES>()?;
        let key_id = reader.get_array()?;
        let key = reader.get_array()?;
        let epoch = reader.get_u64()?;
        let token = reader.get_array()?;
        let expires_at_us = reader.get_u64()?;
        let scope = reader.get_u32()?;
        let used = reader.get_bool()?;
        let revoked = reader.get_bool()?;
        let request_fingerprint = reader.get_array()?;
        let name = MetadataText::from_raw(name_bytes, name_len)?;
        let description = MetadataText::from_raw(description_bytes, description_len)?;
        let state = Self {
            cluster_id,
            name,
            description,
            node: BootstrapNodeRecord {
                node_id,
                capabilities,
                transports,
                protocols,
                attestation: AttestationEvidence {
                    measurement,
                    verified: attested,
                },
            },
            root: ClusterRootIdentity {
                cluster_id,
                key_id,
                key,
                epoch,
            },
            admission_policy,
            attestation_required,
            quorum,
            bootstrap_token: BootstrapToken {
                token,
                expires_at_us,
                scope,
                used,
                revoked,
            },
            endpoint,
            lifecycle,
            generation,
            created_at_us,
            advertisement_sequence,
            credential_epoch,
            request_fingerprint,
        };
        state.validate_persisted()
    }

    pub fn save_to_synfs<const BLOCKS: usize>(
        &self,
        filesystem: &mut synos_synfs::SynFs<BLOCKS>,
        staging: &mut [u8],
    ) -> Result<synos_synfs::TransactionCommit, ClusterBootstrapError> {
        let length = self.encode(staging)?;
        let mut transaction = filesystem.transaction();
        for directory in ["/system", "/system/cluster"] {
            match transaction.create_directory(directory, true) {
                Ok(_) | Err(synos_synfs::Error::AlreadyExists) => {}
                Err(_) => return Err(ClusterBootstrapError::Persistence),
            }
        }
        transaction
            .write(CLUSTER_BOOTSTRAP_STATE_FILE, &staging[..length])
            .map_err(|_| ClusterBootstrapError::Persistence)?;
        transaction
            .commit()
            .map_err(|_| ClusterBootstrapError::Persistence)
    }

    pub fn load_from_synfs<const BLOCKS: usize>(
        filesystem: &synos_synfs::SynFs<BLOCKS>,
        staging: &mut [u8],
    ) -> Result<Self, ClusterBootstrapError> {
        if staging.len() < Self::encoded_len() {
            return Err(ClusterBootstrapError::BufferTooSmall {
                required: Self::encoded_len(),
            });
        }
        let read = filesystem
            .read(CLUSTER_BOOTSTRAP_STATE_FILE, staging)
            .map_err(|_| ClusterBootstrapError::Persistence)?;
        Self::decode(&staging[..read.bytes_read])
    }

    fn validate_persisted(&self) -> Result<Self, ClusterBootstrapError> {
        if self.root.cluster_id != self.cluster_id
            || is_zero(&self.root.key)
            || is_zero(&self.root.key_id)
            || is_zero(&self.bootstrap_token.token)
            || self.generation == 0
            || self.credential_epoch == 0
        {
            return Err(ClusterBootstrapError::Corrupt);
        }
        self.quorum.validate()?;
        self.node.protocols.validate()?;
        if self.name.is_empty() || self.node.transports.bits() == 0 {
            return Err(ClusterBootstrapError::Corrupt);
        }
        Ok(*self)
    }
}

fn map_metadata_error(error: ClusterMetadataError) -> ClusterBootstrapError {
    match error {
        ClusterMetadataError::AlreadyExists => ClusterBootstrapError::AlreadyExists,
        ClusterMetadataError::BufferTooSmall { required } => {
            ClusterBootstrapError::BufferTooSmall { required }
        }
        ClusterMetadataError::Capacity => ClusterBootstrapError::Conflict,
        ClusterMetadataError::Corrupt => ClusterBootstrapError::Corrupt,
        ClusterMetadataError::InvalidValue => ClusterBootstrapError::InvalidConfiguration,
        ClusterMetadataError::NotFound | ClusterMetadataError::NotRestorable => {
            ClusterBootstrapError::NotFound
        }
        ClusterMetadataError::Persistence => ClusterBootstrapError::Persistence,
        ClusterMetadataError::Conflict
        | ClusterMetadataError::SplitBrain
        | ClusterMetadataError::StaleGeneration => ClusterBootstrapError::Conflict,
    }
}

fn hmac_sha256(
    key: &[u8; BOOTSTRAP_SIGNATURE_BYTES],
    message: &[u8],
) -> [u8; BOOTSTRAP_SIGNATURE_BYTES] {
    let mut normalized = [0; 64];
    normalized[..key.len()].copy_from_slice(key);
    let mut inner = [0; 256];
    for (index, byte) in normalized.iter().enumerate() {
        inner[index] = byte ^ 0x36;
    }
    inner[64..64 + message.len()].copy_from_slice(message);
    let inner_hash = ContentId::hash(&inner[..64 + message.len()]);
    let mut outer = [0; 96];
    for (index, byte) in normalized.iter().enumerate() {
        outer[index] = byte ^ 0x5c;
    }
    outer[64..].copy_from_slice(inner_hash.as_bytes());
    *ContentId::hash(&outer).as_bytes()
}

fn constant_time_equal(
    left: &[u8; BOOTSTRAP_SIGNATURE_BYTES],
    right: &[u8; BOOTSTRAP_SIGNATURE_BYTES],
) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn is_zero<const N: usize>(bytes: &[u8; N]) -> bool {
    bytes.iter().all(|byte| *byte == 0)
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

struct Writer<'a> {
    output: &'a mut [u8],
    cursor: usize,
}

impl<'a> Writer<'a> {
    fn new(output: &'a mut [u8]) -> Self {
        Self { output, cursor: 0 }
    }

    fn bytes(&self) -> &[u8] {
        &self.output[..self.cursor]
    }

    fn put_u8(&mut self, value: u8) {
        self.output[self.cursor] = value;
        self.cursor += 1;
    }

    fn put_u16(&mut self, value: u16) {
        self.output[self.cursor..self.cursor + 2].copy_from_slice(&value.to_le_bytes());
        self.cursor += 2;
    }

    fn put_u32(&mut self, value: u32) {
        self.output[self.cursor..self.cursor + 4].copy_from_slice(&value.to_le_bytes());
        self.cursor += 4;
    }

    fn put_i64(&mut self, value: i64) {
        self.put_u64(value as u64);
    }

    fn put_u64(&mut self, value: u64) {
        self.output[self.cursor..self.cursor + 8].copy_from_slice(&value.to_le_bytes());
        self.cursor += 8;
    }

    fn put_bytes(&mut self, value: &[u8]) {
        self.output[self.cursor..self.cursor + value.len()].copy_from_slice(value);
        self.cursor += value.len();
    }

    fn put_text<const CAPACITY: usize>(&mut self, value: &MetadataText<CAPACITY>) {
        let length = value.as_bytes().len();
        self.put_u16(length as u16);
        self.output[self.cursor..self.cursor + CAPACITY].fill(0);
        self.output[self.cursor..self.cursor + length].copy_from_slice(value.as_bytes());
        self.cursor += CAPACITY;
    }

    fn put_optional_id(&mut self, value: Option<ClusterId>) {
        match value {
            Some(value) => {
                self.put_u8(1);
                self.put_bytes(&value.raw());
            }
            None => self.put_u8(0),
        }
    }
}

struct Reader<'a> {
    input: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, cursor: 0 }
    }

    fn get_u8(&mut self) -> Result<u8, ClusterBootstrapError> {
        let value = *self
            .input
            .get(self.cursor)
            .ok_or(ClusterBootstrapError::Corrupt)?;
        self.cursor += 1;
        Ok(value)
    }

    fn get_bool(&mut self) -> Result<bool, ClusterBootstrapError> {
        match self.get_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ClusterBootstrapError::Corrupt),
        }
    }

    fn get_u16(&mut self) -> Result<u16, ClusterBootstrapError> {
        Ok(u16::from_le_bytes(self.get_array()?))
    }

    fn get_u32(&mut self) -> Result<u32, ClusterBootstrapError> {
        Ok(u32::from_le_bytes(self.get_array()?))
    }

    fn get_u64(&mut self) -> Result<u64, ClusterBootstrapError> {
        Ok(u64::from_le_bytes(self.get_array()?))
    }

    fn get_array<const N: usize>(&mut self) -> Result<[u8; N], ClusterBootstrapError> {
        let end = self
            .cursor
            .checked_add(N)
            .ok_or(ClusterBootstrapError::Corrupt)?;
        let bytes = self
            .input
            .get(self.cursor..end)
            .ok_or(ClusterBootstrapError::Corrupt)?;
        let mut output = [0; N];
        output.copy_from_slice(bytes);
        self.cursor = end;
        Ok(output)
    }

    fn get_text<const CAPACITY: usize>(
        &mut self,
    ) -> Result<MetadataText<CAPACITY>, ClusterBootstrapError> {
        let length = self.get_u16()? as usize;
        let bytes = self.get_array::<CAPACITY>()?;
        MetadataText::from_raw(bytes, length)
    }
}

trait MetadataTextExt<const CAPACITY: usize> {
    fn len(&self) -> u16;
    fn raw_bytes(&self) -> [u8; CAPACITY];
    fn from_raw(
        bytes: [u8; CAPACITY],
        length: usize,
    ) -> Result<MetadataText<CAPACITY>, ClusterBootstrapError>;
}

impl<const CAPACITY: usize> MetadataTextExt<CAPACITY> for MetadataText<CAPACITY> {
    fn len(&self) -> u16 {
        self.as_bytes().len() as u16
    }

    fn raw_bytes(&self) -> [u8; CAPACITY] {
        let mut output = [0; CAPACITY];
        output[..self.as_bytes().len()].copy_from_slice(self.as_bytes());
        output
    }

    fn from_raw(
        bytes: [u8; CAPACITY],
        length: usize,
    ) -> Result<MetadataText<CAPACITY>, ClusterBootstrapError> {
        if length > CAPACITY {
            return Err(ClusterBootstrapError::Corrupt);
        }
        let value =
            core::str::from_utf8(&bytes[..length]).map_err(|_| ClusterBootstrapError::Corrupt)?;
        MetadataText::new(value).map_err(|_| ClusterBootstrapError::Corrupt)
    }
}
