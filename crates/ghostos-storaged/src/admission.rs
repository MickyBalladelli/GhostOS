use ghostos_auth::CapabilityKey;
use ghostos_fabric::NodeId;
use ghostos_durability::{CrashBoundary, CrashDomain, InterruptionInjector, NoInterruption};
use ghostos_status::{IntoStatus, Status};

use crate::{
    AdmissionPolicy, AttestationEvidence, BootstrapAdvertisement, ClusterId,
    ClusterBootstrapError, EntropySource, NodeCapabilities, ProtocolCompatibility, TransportSet,
};

pub const MAX_ADMISSION_MEMBERS: usize = 32;
pub const MAX_ADMISSION_INVITATIONS: usize = 32;
pub const MAX_DISCOVERY_CANDIDATES: usize = 32;
pub const MAX_ADMISSION_AUDIT: usize = 128;
pub const MAX_ADMISSION_ENDPOINT_BYTES: usize = 96;
pub const MAX_ADDRESS_SPACE_LAYOUT_BYTES: usize = 64;
pub const ADMISSION_AUDIT_STATE_FILE: &str = "/system/cluster/admission-audit.dat";
pub const ADMISSION_AUDIT_MAGIC: &[u8; 8] = b"SYNADIT1";
pub const ADMISSION_AUDIT_FORMAT_VERSION: u16 = 1;
pub const INVITATION_SCOPE_JOIN: u32 = 1 << 0;
pub const INVITATION_SCOPE_REJOIN: u32 = 1 << 1;
pub const INVITATION_SCOPE_RECONCILE: u32 = 1 << 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionEndpoint {
    bytes: [u8; MAX_ADMISSION_ENDPOINT_BYTES],
    len: u16,
}

impl AdmissionEndpoint {
    pub fn new(value: &str) -> Result<Self, AdmissionError> {
        if value.is_empty()
            || value.len() > MAX_ADMISSION_ENDPOINT_BYTES
            || value.bytes().any(|byte| byte == 0 || byte.is_ascii_whitespace())
        {
            return Err(AdmissionError::InvalidEndpoint)
        }
        let mut bytes = [0; MAX_ADMISSION_ENDPOINT_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            len: value.len() as u16,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressSpaceLayout {
    pub local_base: u64,
    pub remote_base: u64,
    pub length: u64,
    pub page_size: u32,
}

impl AddressSpaceLayout {
    pub fn validate(self) -> Result<(), AdmissionError> {
        if self.length == 0
            || self.page_size == 0
            || !self.local_base.is_multiple_of(self.page_size as u64)
            || !self.remote_base.is_multiple_of(self.page_size as u64)
            || !self.length.is_multiple_of(self.page_size as u64)
        {
            return Err(AdmissionError::IncompatibleLayout)
        }
        self.local_base
            .checked_add(self.length)
            .ok_or(AdmissionError::IncompatibleLayout)?;
        self.remote_base
            .checked_add(self.length)
            .ok_or(AdmissionError::IncompatibleLayout)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecurityPolicy {
    pub require_mutual_identity: bool,
    pub require_certificate: bool,
    pub require_attestation: bool,
    pub encrypt_control_plane: bool,
    pub encrypt_data_plane: bool,
    pub allowed_attestation_roots: u8,
}

impl SecurityPolicy {
    pub const DEFAULT: Self = Self {
        require_mutual_identity: true,
        require_certificate: true,
        require_attestation: false,
        encrypt_control_plane: true,
        encrypt_data_plane: true,
        allowed_attestation_roots: u8::MAX,
    };

    fn allows_root(self, root: AttestationRoot) -> bool {
        self.allowed_attestation_roots & (1 << root as u8) != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AttestationRoot {
    Tpm2 = 0,
    TrustZone = 1,
    AmdSevSnp = 2,
    IntelTdx = 3,
    NvidiaTee = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeAttestation {
    pub root: AttestationRoot,
    pub evidence: AttestationEvidence,
}

impl NodeAttestation {
    pub const NONE: Self = Self {
        root: AttestationRoot::Tpm2,
        evidence: AttestationEvidence::NONE,
    };

    fn valid_for(self, policy: SecurityPolicy) -> bool {
        policy.allows_root(self.root)
            && self.evidence.verified
            && self.evidence.measurement.iter().any(|byte| *byte != 0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeIdentity {
    pub node: NodeId,
    pub fingerprint: [u8; 32],
    pub key: CapabilityKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityProof {
    pub node: NodeId,
    pub fingerprint: [u8; 32],
    pub challenge: [u8; 32],
    pub signature: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JoinChallenge {
    pub cluster: ClusterId,
    pub node: NodeId,
    pub challenge: [u8; 32],
    pub expires_at_us: u64,
}

impl NodeIdentity {
    pub fn prove(self, challenge: [u8; 32]) -> Result<IdentityProof, AdmissionError> {
        Ok(IdentityProof {
            node: self.node,
            fingerprint: self.fingerprint,
            challenge,
            signature: self
                .key
                .authenticate(&challenge)
                .map_err(|_| AdmissionError::IdentityFailure)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrustedIdentity {
    pub node: NodeId,
    pub fingerprint: [u8; 32],
    pub key: CapabilityKey,
}

impl TrustedIdentity {
    fn verify(&self, proof: IdentityProof, challenge: [u8; 32]) -> Result<(), AdmissionError> {
        if proof.node != self.node
            || proof.fingerprint != self.fingerprint
            || proof.challenge != challenge
        {
            return Err(AdmissionError::IdentityFailure)
        }
        self.key
            .verify_authenticator(&challenge, &proof.signature)
            .map_err(|_| AdmissionError::IdentityFailure)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolOffer {
    pub protocols: ProtocolCompatibility,
    pub capabilities: NodeCapabilities,
    pub transports: TransportSet,
    pub layout: AddressSpaceLayout,
    pub features: u64,
    pub security: SecurityPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NegotiatedAdmission {
    pub control_version: u16,
    pub data_version: u16,
    pub capabilities: NodeCapabilities,
    pub transports: TransportSet,
    pub layout: AddressSpaceLayout,
    pub features: u64,
    pub security: SecurityPolicy,
}

impl ProtocolOffer {
    fn negotiate(self, other: Self) -> Result<NegotiatedAdmission, AdmissionError> {
        self.layout.validate()?;
        other.layout.validate()?;
        if self.layout.page_size != other.layout.page_size
            || self.layout.length != other.layout.length
        {
            return Err(AdmissionError::IncompatibleLayout)
        }
        let control_version = self
            .protocols
            .control_version
            .min(other.protocols.control_version);
        let data_version = self.protocols.data_version.min(other.protocols.data_version);
        if control_version < self.protocols.minimum_version
            || control_version < other.protocols.minimum_version
            || data_version < self.protocols.minimum_version
            || data_version < other.protocols.minimum_version
        {
            return Err(AdmissionError::ProtocolMismatch)
        }
        let transports = TransportSet::from_bits(self.transports.bits() & other.transports.bits());
        if transports.bits() == 0 {
            return Err(AdmissionError::TransportUnavailable)
        }
        let capabilities = NodeCapabilities::from_bits(self.capabilities.bits() & other.capabilities.bits());
        let security = SecurityPolicy {
            require_mutual_identity: self.security.require_mutual_identity
                || other.security.require_mutual_identity,
            require_certificate: self.security.require_certificate || other.security.require_certificate,
            require_attestation: self.security.require_attestation || other.security.require_attestation,
            encrypt_control_plane: self.security.encrypt_control_plane
                || other.security.encrypt_control_plane,
            encrypt_data_plane: self.security.encrypt_data_plane || other.security.encrypt_data_plane,
            allowed_attestation_roots: self.security.allowed_attestation_roots
                & other.security.allowed_attestation_roots,
        };
        if security.allowed_attestation_roots == 0 {
            return Err(AdmissionError::AttestationRejected)
        }
        Ok(NegotiatedAdmission {
            control_version,
            data_version,
            capabilities,
            transports,
            layout: self.layout,
            features: self.features & other.features,
            security,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionInvitation {
    pub cluster: ClusterId,
    pub token: [u8; 32],
    pub expires_at_us: u64,
    pub scope: u32,
    pub target: Option<NodeId>,
    pub fingerprint: Option<[u8; 32]>,
    pub one_time: bool,
    pub used: bool,
    pub revoked: bool,
    pub decision: InvitationDecision,
}

impl AdmissionInvitation {
    pub fn usable(
        &self,
        cluster: ClusterId,
        node: NodeId,
        fingerprint: [u8; 32],
        requested_scope: u32,
        now_us: u64,
    ) -> Result<(), AdmissionError> {
        if self.cluster != cluster
            || self.token.iter().all(|byte| *byte == 0)
            || self.used && self.one_time
            || self.revoked
            || self.decision != InvitationDecision::Pending
            || now_us >= self.expires_at_us
            || requested_scope == 0
            || self.scope & requested_scope != requested_scope
            || self.target.is_some_and(|target| target != node)
            || self.fingerprint.is_some_and(|expected| expected != fingerprint)
        {
            return Err(if self.revoked {
                AdmissionError::InvitationRevoked
            } else if now_us >= self.expires_at_us {
                AdmissionError::InvitationExpired
            } else {
                AdmissionError::InvitationInvalid
            })
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvitationDecision {
    Pending,
    Approved,
    Rejected,
    Revoked,
    Used,
    Expired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipState {
    Pending,
    Approved,
    Rejected,
    Joined,
    Draining,
    Left,
    Fenced,
    Expelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipReason {
    Requested,
    Approved,
    Rejected,
    IdentityMismatch,
    InvitationExpired,
    ProtocolMismatch,
    QuorumUnavailable,
    Drained,
    Forced,
    Fenced,
    Rejoined,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MembershipRecord {
    pub node: NodeId,
    pub state: MembershipState,
    pub reason: MembershipReason,
    pub role: u8,
    pub voting: bool,
    pub fingerprint: [u8; 32],
    pub endpoint: AdmissionEndpoint,
    pub negotiated: NegotiatedAdmission,
    pub generation: u64,
    pub last_seen_generation: u64,
    pub membership_epoch: u64,
    pub invitation_token: Option<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipChangeKind {
    Pending,
    Approve,
    Reject,
    Join,
    Drain,
    Leave,
    Fence,
    Expel,
    Rejoin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MembershipChange {
    pub node: NodeId,
    pub kind: MembershipChangeKind,
    pub state: MembershipState,
    pub reason: MembershipReason,
    pub epoch: u64,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuditRecord {
    pub sequence: u64,
    pub node: NodeId,
    pub kind: MembershipChangeKind,
    pub state: MembershipState,
    pub reason: MembershipReason,
    pub epoch: u64,
    pub at_us: u64,
    pub durable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalError {
    BufferTooSmall { required: usize },
    Corrupt,
    Persistence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionAuditJournal<const CAPACITY: usize = MAX_ADMISSION_AUDIT> {
    pub cluster: ClusterId,
    pub epoch: u64,
    pub generation: u64,
    pub records: [Option<AuditRecord>; CAPACITY],
}

impl<const CAPACITY: usize> AdmissionAuditJournal<CAPACITY> {
    pub const fn encoded_len() -> usize {
        56 + CAPACITY * 40
    }

    pub fn from_workflow<const MEMBERS: usize, const INVITATIONS: usize>(
        workflow: &AdmissionWorkflow<MEMBERS, INVITATIONS, CAPACITY>,
    ) -> Self {
        Self {
            cluster: workflow.cluster,
            epoch: workflow.epoch,
            generation: workflow.generation,
            records: workflow.audit,
        }
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, JournalError> {
        let required = Self::encoded_len();
        if output.len() < required {
            return Err(JournalError::BufferTooSmall { required })
        }
        output[..required].fill(0);
        output[..8].copy_from_slice(ADMISSION_AUDIT_MAGIC);
        output[8..10].copy_from_slice(&ADMISSION_AUDIT_FORMAT_VERSION.to_le_bytes());
        output[16..32].copy_from_slice(&self.cluster.raw());
        output[32..40].copy_from_slice(&self.epoch.to_le_bytes());
        output[40..48].copy_from_slice(&self.generation.to_le_bytes());
        for (index, record) in self.records.iter().enumerate() {
            let start = 56 + index * 40;
            if let Some(record) = record {
                output[start] = 1;
                output[start + 1..start + 5].copy_from_slice(&record.node.raw().to_le_bytes());
                output[start + 5] = change_kind_byte(record.kind);
                output[start + 6] = membership_state_byte(record.state);
                output[start + 7] = membership_reason_byte(record.reason);
                output[start + 8..start + 16].copy_from_slice(&record.epoch.to_le_bytes());
                output[start + 16..start + 24].copy_from_slice(&record.at_us.to_le_bytes());
                output[start + 24..start + 32].copy_from_slice(&record.sequence.to_le_bytes());
                output[start + 32] = u8::from(record.durable);
            }
        }
        let checksum = journal_checksum(&output[..required]);
        output[48..56].copy_from_slice(&checksum.to_le_bytes());
        Ok(required)
    }

    pub fn decode(input: &[u8]) -> Result<Self, JournalError> {
        let required = Self::encoded_len();
        if input.len() < required
            || &input[..8] != ADMISSION_AUDIT_MAGIC
            || u16::from_le_bytes([input[8], input[9]]) != ADMISSION_AUDIT_FORMAT_VERSION
        {
            return Err(JournalError::Corrupt)
        }
        let stored = u64::from_le_bytes(input[48..56].try_into().map_err(|_| JournalError::Corrupt)?);
        if journal_checksum(&input[..required]) != stored {
            return Err(JournalError::Corrupt)
        }
        let cluster = ClusterId::new(input[16..32].try_into().map_err(|_| JournalError::Corrupt)?)
            .map_err(|_| JournalError::Corrupt)?;
        let epoch = u64::from_le_bytes(input[32..40].try_into().map_err(|_| JournalError::Corrupt)?);
        let generation = u64::from_le_bytes(input[40..48].try_into().map_err(|_| JournalError::Corrupt)?);
        let mut records = [None; CAPACITY];
        for (index, record) in records.iter_mut().enumerate() {
            let start = 56 + index * 40;
            match input[start] {
                0 => {}
                1 => {
                    let node = NodeId::new(u32::from_le_bytes(
                        input[start + 1..start + 5]
                            .try_into()
                            .map_err(|_| JournalError::Corrupt)?,
                    ))
                    .ok_or(JournalError::Corrupt)?;
                    *record = Some(AuditRecord {
                        sequence: u64::from_le_bytes(
                            input[start + 24..start + 32]
                                .try_into()
                                .map_err(|_| JournalError::Corrupt)?,
                        ),
                        node,
                        kind: change_kind_from_byte(input[start + 5]).ok_or(JournalError::Corrupt)?,
                        state: membership_state_from_byte(input[start + 6]).ok_or(JournalError::Corrupt)?,
                        reason: membership_reason_from_byte(input[start + 7]).ok_or(JournalError::Corrupt)?,
                        epoch: u64::from_le_bytes(
                            input[start + 8..start + 16]
                                .try_into()
                                .map_err(|_| JournalError::Corrupt)?,
                        ),
                        at_us: u64::from_le_bytes(
                            input[start + 16..start + 24]
                                .try_into()
                                .map_err(|_| JournalError::Corrupt)?,
                        ),
                        durable: match input[start + 32] {
                            0 => false,
                            1 => true,
                            _ => return Err(JournalError::Corrupt),
                        },
                    })
                }
                _ => return Err(JournalError::Corrupt),
            }
        }
        Ok(Self {
            cluster,
            epoch,
            generation,
            records,
        })
    }

    pub fn save_to_ghostfs<const BLOCKS: usize>(
        &self,
        filesystem: &mut ghostos_ghostfs::SynFs<BLOCKS>,
        staging: &mut [u8],
    ) -> Result<ghostos_ghostfs::TransactionCommit, JournalError> {
        let length = self.encode(staging)?;
        let mut transaction = filesystem.transaction();
        for directory in ["/system", "/system/cluster"] {
            match transaction.create_directory(directory, true) {
                Ok(_) | Err(ghostos_ghostfs::Error::AlreadyExists) => {}
                Err(_) => return Err(JournalError::Persistence),
            }
        }
        transaction
            .write(ADMISSION_AUDIT_STATE_FILE, &staging[..length])
            .map_err(|_| JournalError::Persistence)?;
        transaction.commit().map_err(|_| JournalError::Persistence)
    }
}

impl IntoStatus for JournalError {
    fn status(self) -> Status {
        match self {
            Self::BufferTooSmall { .. } => Status::NO_SPACE,
            Self::Corrupt => Status::CORRUPT,
            Self::Persistence => Status::BUSY,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuorumReceipt {
    pub acknowledged_votes: u16,
    pub required_votes: u16,
    pub audit_sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryCandidate {
    pub advertisement: BootstrapAdvertisement,
    pub source: DiscoverySource,
    pub observed_at_us: u64,
    pub last_sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoverySource {
    ConfiguredEndpoint,
    MeshGossip,
    Mdns,
    Broadcast,
    ExplicitAddress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryDirectory<const CAPACITY: usize = MAX_DISCOVERY_CANDIDATES> {
    candidates: [Option<DiscoveryCandidate>; CAPACITY],
}

impl<const CAPACITY: usize> DiscoveryDirectory<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            candidates: [None; CAPACITY],
        }
    }

    pub fn observe(
        &mut self,
        advertisement: BootstrapAdvertisement,
        source: DiscoverySource,
        now_us: u64,
        root: crate::ClusterRootIdentity,
    ) -> Result<(), AdmissionError> {
        advertisement
            .verify(root, now_us)
            .map_err(AdmissionError::Bootstrap)?;
        if let Some(candidate) = self
            .candidates
            .iter_mut()
            .flatten()
            .find(|candidate| candidate.advertisement.cluster_id == advertisement.cluster_id)
        {
            if advertisement.sequence <= candidate.last_sequence {
                return Err(AdmissionError::Replay)
            }
            candidate.advertisement = advertisement;
            candidate.source = source;
            candidate.observed_at_us = now_us;
            candidate.last_sequence = advertisement.sequence;
            return Ok(())
        }
        let slot = self
            .candidates
            .iter_mut()
            .find(|candidate| candidate.is_none())
            .ok_or(AdmissionError::DiscoveryCapacity)?;
        *slot = Some(DiscoveryCandidate {
            advertisement,
            source,
            observed_at_us: now_us,
            last_sequence: advertisement.sequence,
        });
        Ok(())
    }

    pub fn candidates(&self, now_us: u64) -> impl Iterator<Item = DiscoveryCandidate> + '_ {
        self.candidates
            .iter()
            .flatten()
            .filter(move |candidate| candidate.advertisement.expires_at_us > now_us)
            .copied()
    }

    pub fn find(&self, cluster: ClusterId, now_us: u64) -> Option<DiscoveryCandidate> {
        self.candidates(now_us)
            .find(|candidate| candidate.advertisement.cluster_id == cluster)
    }
}

impl<const CAPACITY: usize> Default for DiscoveryDirectory<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeavePlan {
    pub drain: bool,
    pub force: bool,
    pub confirm: bool,
    pub reconcile: bool,
    pub timeout_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaveOutcome {
    pub node: NodeId,
    pub state: MembershipState,
    pub epoch: u64,
    pub forced: bool,
    pub reconciled: bool,
    pub receipt: QuorumReceipt,
}

pub trait LeaveHooks {
    fn drain_workloads(&mut self, node: NodeId, timeout_us: u64) -> Result<(), AdmissionError>;
    fn release_dlm_leases(&mut self, node: NodeId) -> Result<(), AdmissionError>;
    fn flush_remote_pages(&mut self, node: NodeId) -> Result<(), AdmissionError>;
    fn reconcile_ghostfs(&mut self, node: NodeId) -> Result<(), AdmissionError>;
    fn revoke_capabilities(&mut self, node: NodeId) -> Result<(), AdmissionError>;
    fn close_ipc_streams(&mut self, node: NodeId) -> Result<(), AdmissionError>;
    fn fence(&mut self, node: NodeId, epoch: u64) -> Result<(), AdmissionError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    InvalidConfiguration,
    InvalidEndpoint,
    InvitationExpired,
    InvitationInvalid,
    InvitationRevoked,
    InvitationCapacity,
    IdentityFailure,
    CertificateMismatch,
    AttestationRejected,
    ProtocolMismatch,
    TransportUnavailable,
    IncompatibleLayout,
    DuplicateIdentity,
    UnknownNode,
    InvalidState,
    QuorumUnavailable,
    AuditCapacity,
    DiscoveryCapacity,
    Replay,
    ForcedActionNotAuthorized,
    DrainFailed,
    ReconciliationFailed,
    Interrupted,
    Bootstrap(ClusterBootstrapError),
}

impl IntoStatus for AdmissionError {
    fn status(self) -> Status {
        match self {
            Self::InvitationCapacity | Self::AuditCapacity | Self::DiscoveryCapacity => Status::NO_SPACE,
            Self::InvitationExpired
            | Self::InvitationInvalid
            | Self::InvitationRevoked
            | Self::CertificateMismatch
            | Self::IdentityFailure
            | Self::AttestationRejected
            | Self::ForcedActionNotAuthorized => Status::ACCESS_DENIED,
            Self::UnknownNode => Status::NOT_FOUND,
            Self::QuorumUnavailable | Self::DrainFailed | Self::ReconciliationFailed => Status::BUSY,
            Self::DuplicateIdentity | Self::Replay | Self::InvalidState => Status::CONFLICT,
            Self::Bootstrap(error) => error.status(),
            Self::Interrupted => Status::BUSY,
            Self::InvalidConfiguration
            | Self::InvalidEndpoint
            | Self::ProtocolMismatch
            | Self::TransportUnavailable
            | Self::IncompatibleLayout => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy)]
pub struct AdmissionWorkflow<
    const MEMBERS: usize = MAX_ADMISSION_MEMBERS,
    const INVITATIONS: usize = MAX_ADMISSION_INVITATIONS,
    const AUDIT: usize = MAX_ADMISSION_AUDIT,
> {
    pub cluster: ClusterId,
    pub policy: AdmissionPolicy,
    pub quorum_required: u16,
    pub quorum_available: u16,
    pub security: SecurityPolicy,
    pub epoch: u64,
    pub generation: u64,
    invitations: [Option<AdmissionInvitation>; INVITATIONS],
    trusted: [Option<TrustedIdentity>; MEMBERS],
    challenges: [Option<JoinChallenge>; MEMBERS],
    members: [Option<MembershipRecord>; MEMBERS],
    audit: [Option<AuditRecord>; AUDIT],
    next_audit: u64,
}

impl<const MEMBERS: usize, const INVITATIONS: usize, const AUDIT: usize>
    AdmissionWorkflow<MEMBERS, INVITATIONS, AUDIT>
{
    pub fn new(
        cluster: ClusterId,
        policy: AdmissionPolicy,
        quorum_required: u16,
        security: SecurityPolicy,
    ) -> Result<Self, AdmissionError> {
        if quorum_required == 0 {
            return Err(AdmissionError::InvalidConfiguration)
        }
        Ok(Self {
            cluster,
            policy,
            quorum_required,
            quorum_available: quorum_required,
            security,
            epoch: 1,
            generation: 1,
            invitations: [None; INVITATIONS],
            trusted: [None; MEMBERS],
            challenges: [None; MEMBERS],
            members: [None; MEMBERS],
            audit: [None; AUDIT],
            next_audit: 1,
        })
    }

    pub fn set_quorum_available(&mut self, votes: u16) {
        self.quorum_available = votes
    }

    pub fn register_identity(&mut self, identity: TrustedIdentity) -> Result<(), AdmissionError> {
        if self
            .trusted
            .iter()
            .flatten()
            .any(|entry| entry.node == identity.node || entry.fingerprint == identity.fingerprint)
        {
            return Err(AdmissionError::DuplicateIdentity)
        }
        let slot = self
            .trusted
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(AdmissionError::InvitationCapacity)?;
        *slot = Some(identity);
        Ok(())
    }

    pub fn begin_join(
        &mut self,
        node: NodeId,
        now_us: u64,
        lifetime_us: u64,
        entropy: &mut impl EntropySource,
    ) -> Result<JoinChallenge, AdmissionError> {
        if lifetime_us == 0 || self.trusted.iter().flatten().all(|entry| entry.node != node) {
            return Err(AdmissionError::IdentityFailure)
        }
        let expires_at_us = now_us
            .checked_add(lifetime_us)
            .ok_or(AdmissionError::InvalidConfiguration)?;
        let mut challenge = [0; 32];
        entropy
            .fill_bytes(&mut challenge)
            .map_err(AdmissionError::Bootstrap)?;
        if challenge.iter().all(|byte| *byte == 0) {
            return Err(AdmissionError::InvalidConfiguration)
        }
        let issued = JoinChallenge {
            cluster: self.cluster,
            node,
            challenge,
            expires_at_us,
        };
        let slot = self
            .challenges
            .iter_mut()
            .find(|entry| entry.is_none() || entry.is_some_and(|entry| entry.node == node))
            .ok_or(AdmissionError::InvitationCapacity)?;
        *slot = Some(issued);
        Ok(issued)
    }

    pub fn create_invitation(
        &mut self,
        expires_at_us: u64,
        scope: u32,
        target: Option<NodeId>,
        fingerprint: Option<[u8; 32]>,
        one_time: bool,
        entropy: &mut impl EntropySource,
    ) -> Result<AdmissionInvitation, AdmissionError> {
        if expires_at_us == 0 || scope == 0 || scope & INVITATION_SCOPE_JOIN == 0 {
            return Err(AdmissionError::InvalidConfiguration)
        }
        let mut token = [0; 32];
        entropy
            .fill_bytes(&mut token)
            .map_err(AdmissionError::Bootstrap)?;
        if token.iter().all(|byte| *byte == 0) {
            return Err(AdmissionError::InvalidConfiguration)
        }
        let invitation = AdmissionInvitation {
            cluster: self.cluster,
            token,
            expires_at_us,
            scope,
            target,
            fingerprint,
            one_time,
            used: false,
            revoked: false,
            decision: InvitationDecision::Pending,
        };
        let slot = self
            .invitations
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(AdmissionError::InvitationCapacity)?;
        *slot = Some(invitation);
        self.generation = self.generation.saturating_add(1);
        Ok(invitation)
    }

    pub fn invitation(&self, token: [u8; 32]) -> Option<AdmissionInvitation> {
        self.invitations
            .iter()
            .flatten()
            .find(|invitation| invitation.token == token)
            .copied()
    }

    pub fn revoke_invitation(&mut self, token: [u8; 32]) -> Result<(), AdmissionError> {
        let invitation = self
            .invitations
            .iter_mut()
            .flatten()
            .find(|invitation| invitation.token == token)
            .ok_or(AdmissionError::InvitationInvalid)?;
        invitation.revoked = true;
        invitation.decision = InvitationDecision::Revoked;
        self.generation = self.generation.saturating_add(1);
        Ok(())
    }

    pub fn invitation_decision(
        &self,
        token: [u8; 32],
        now_us: u64,
    ) -> Result<InvitationDecision, AdmissionError> {
        let invitation = self.invitation(token).ok_or(AdmissionError::InvitationInvalid)?;
        Ok(if invitation.revoked {
            InvitationDecision::Revoked
        } else if invitation.used {
            InvitationDecision::Used
        } else if invitation.decision != InvitationDecision::Pending {
            invitation.decision
        } else if now_us >= invitation.expires_at_us {
            InvitationDecision::Expired
        } else {
            InvitationDecision::Pending
        })
    }

    pub fn request_join(
        &mut self,
        request: JoinRequest,
        cluster_offer: ProtocolOffer,
        now_us: u64,
    ) -> Result<MembershipRecord, AdmissionError> {
        let mut no_interruption = NoInterruption;
        self.request_join_with_interruption(request, cluster_offer, now_us, &mut no_interruption)
    }

    pub fn request_join_with_interruption<I: InterruptionInjector>(
        &mut self,
        request: JoinRequest,
        cluster_offer: ProtocolOffer,
        now_us: u64,
        injector: &mut I,
    ) -> Result<MembershipRecord, AdmissionError> {
        let trusted = self
            .trusted
            .iter()
            .flatten()
            .find(|identity| identity.node == request.node)
            .copied()
            .ok_or(AdmissionError::IdentityFailure)?;
        let challenge = self.take_challenge(request.node, request.challenge, now_us)?;
        trusted.verify(request.identity, challenge)?;
        if self.security.require_certificate && request.identity.fingerprint == [0; 32] {
            return Err(AdmissionError::CertificateMismatch)
        }
        if (self.security.require_attestation || self.policy == AdmissionPolicy::Attested)
            && !request.attestation.valid_for(self.security)
        {
            return Err(AdmissionError::AttestationRejected)
        }
        if request.requested_scope == 0 {
            return Err(AdmissionError::InvitationInvalid)
        }
        let invitation = match self.policy {
            AdmissionPolicy::Open => None,
            AdmissionPolicy::Invitation | AdmissionPolicy::Attested => {
                let token = request.token.ok_or(AdmissionError::InvitationInvalid)?;
                let invitation = self
                    .invitations
                    .iter()
                    .flatten()
                    .find(|invitation| invitation.token == token)
                    .copied()
                    .ok_or(AdmissionError::InvitationInvalid)?;
                invitation.usable(
                    self.cluster,
                    request.node,
                    request.identity.fingerprint,
                    request.requested_scope,
                    now_us,
                )?;
                Some(invitation)
            }
        };
        let negotiated = request.offer.negotiate(cluster_offer)?;
        self.ensure_commit_ready()?;
        if self
            .members
            .iter()
            .flatten()
            .any(|member| member.node == request.node
                && !matches!(member.state, MembershipState::Left | MembershipState::Expelled))
        {
            return Err(AdmissionError::DuplicateIdentity)
        }
        let record = MembershipRecord {
            node: request.node,
            state: MembershipState::Pending,
            reason: MembershipReason::Requested,
            role: request.role,
            voting: request.voting,
            fingerprint: request.identity.fingerprint,
            endpoint: request.endpoint,
            negotiated,
            generation: self.generation,
            last_seen_generation: request.last_seen_generation,
            membership_epoch: self.epoch,
            invitation_token: request.token,
        };
        let slot = self
            .members
            .iter_mut()
            .find(|member| member.is_none())
            .ok_or(AdmissionError::InvitationCapacity)?;
        *slot = Some(record);
        if let Some(invitation) = invitation {
            if invitation.one_time {
                if let Some(stored) = self
                    .invitations
                    .iter_mut()
                    .flatten()
                    .find(|stored| stored.token == invitation.token)
                {
                    stored.used = true;
                }
            }
        }
        self.commit_pending_with_interruption(request.node, now_us, injector)?;
        Ok(record)
    }

    pub fn approve(&mut self, node: NodeId, now_us: u64) -> Result<QuorumReceipt, AdmissionError> {
        let mut no_interruption = NoInterruption;
        self.approve_with_interruption(node, now_us, &mut no_interruption)
    }

    pub fn approve_with_interruption<I: InterruptionInjector>(
        &mut self,
        node: NodeId,
        now_us: u64,
        injector: &mut I,
    ) -> Result<QuorumReceipt, AdmissionError> {
        let receipt = self.transition_with_interruption(node, MembershipState::Approved, MembershipReason::Approved, MembershipChangeKind::Approve, now_us, injector)?;
        self.set_invitation_decision(node, InvitationDecision::Approved);
        Ok(receipt)
    }

    pub fn reject(&mut self, node: NodeId, now_us: u64) -> Result<QuorumReceipt, AdmissionError> {
        let mut no_interruption = NoInterruption;
        self.reject_with_interruption(node, now_us, &mut no_interruption)
    }

    pub fn reject_with_interruption<I: InterruptionInjector>(
        &mut self,
        node: NodeId,
        now_us: u64,
        injector: &mut I,
    ) -> Result<QuorumReceipt, AdmissionError> {
        let receipt = self.transition_with_interruption(node, MembershipState::Rejected, MembershipReason::Rejected, MembershipChangeKind::Reject, now_us, injector)?;
        self.set_invitation_decision(node, InvitationDecision::Rejected);
        Ok(receipt)
    }

    pub fn join_approved(
        &mut self,
        node: NodeId,
        now_us: u64,
    ) -> Result<QuorumReceipt, AdmissionError> {
        let mut no_interruption = NoInterruption;
        self.join_approved_with_interruption(node, now_us, &mut no_interruption)
    }

    pub fn join_approved_with_interruption<I: InterruptionInjector>(
        &mut self,
        node: NodeId,
        now_us: u64,
        injector: &mut I,
    ) -> Result<QuorumReceipt, AdmissionError> {
        self.transition_with_interruption(node, MembershipState::Joined, MembershipReason::Approved, MembershipChangeKind::Join, now_us, injector)
    }

    pub fn member(&self, node: NodeId) -> Option<MembershipRecord> {
        self.members.iter().flatten().find(|member| member.node == node).copied()
    }

    pub fn members(&self) -> impl Iterator<Item = MembershipRecord> + '_ {
        self.members.iter().flatten().copied()
    }

    pub fn audits(&self) -> impl Iterator<Item = AuditRecord> + '_ {
        self.audit.iter().flatten().copied()
    }

    pub fn leave(
        &mut self,
        node: NodeId,
        plan: LeavePlan,
        now_us: u64,
        hooks: &mut impl LeaveHooks,
    ) -> Result<LeaveOutcome, AdmissionError> {
        let mut no_interruption = NoInterruption;
        self.leave_with_interruption(node, plan, now_us, hooks, &mut no_interruption)
    }

    pub fn leave_with_interruption<I: InterruptionInjector>(
        &mut self,
        node: NodeId,
        plan: LeavePlan,
        now_us: u64,
        hooks: &mut impl LeaveHooks,
        injector: &mut I,
    ) -> Result<LeaveOutcome, AdmissionError> {
        let current = self.member(node).ok_or(AdmissionError::UnknownNode)?;
        if !matches!(current.state, MembershipState::Joined | MembershipState::Approved) {
            return Err(AdmissionError::InvalidState)
        }
        if plan.force && !plan.confirm {
            return Err(AdmissionError::ForcedActionNotAuthorized)
        }
        if !plan.force && !plan.drain {
            return Err(AdmissionError::InvalidConfiguration)
        }
        if plan.force {
            self.epoch = self.epoch.checked_add(1).ok_or(AdmissionError::InvalidConfiguration)?;
            hooks.fence(node, self.epoch)?;
            hooks.revoke_capabilities(node)?;
            hooks.close_ipc_streams(node)?;
            let receipt = self.transition_with_epoch_with_interruption(
                node,
                MembershipState::Fenced,
                MembershipReason::Forced,
                MembershipChangeKind::Fence,
                now_us,
                injector,
            )?;
            return Ok(LeaveOutcome {
                node,
                state: MembershipState::Fenced,
                epoch: self.epoch,
                forced: true,
                reconciled: false,
                receipt,
            })
        }
        self.transition_with_interruption(node, MembershipState::Draining, MembershipReason::Requested, MembershipChangeKind::Drain, now_us, injector)?;
        hooks.drain_workloads(node, plan.timeout_us)?;
        hooks.release_dlm_leases(node)?;
        hooks.flush_remote_pages(node)?;
        if plan.reconcile {
            hooks.reconcile_ghostfs(node)?;
        }
        hooks.revoke_capabilities(node)?;
        hooks.close_ipc_streams(node)?;
        let receipt = self.transition_with_interruption(node, MembershipState::Left, MembershipReason::Drained, MembershipChangeKind::Leave, now_us, injector)?;
        Ok(LeaveOutcome {
            node,
            state: MembershipState::Left,
            epoch: self.epoch,
            forced: false,
            reconciled: plan.reconcile,
            receipt,
        })
    }

    pub fn rejoin(
        &mut self,
        request: JoinRequest,
        cluster_offer: ProtocolOffer,
        now_us: u64,
    ) -> Result<QuorumReceipt, AdmissionError> {
        let mut no_interruption = NoInterruption;
        self.rejoin_with_interruption(request, cluster_offer, now_us, &mut no_interruption)
    }

    pub fn rejoin_with_interruption<I: InterruptionInjector>(
        &mut self,
        request: JoinRequest,
        cluster_offer: ProtocolOffer,
        now_us: u64,
        injector: &mut I,
    ) -> Result<QuorumReceipt, AdmissionError> {
        let old = self.member(request.node).ok_or(AdmissionError::UnknownNode)?;
        if !matches!(old.state, MembershipState::Joined | MembershipState::Left | MembershipState::Fenced) {
            return Err(AdmissionError::InvalidState)
        }
        let negotiated = request.offer.negotiate(cluster_offer)?;
        self.ensure_commit_ready()?;
        let trusted = self
            .trusted
            .iter()
            .flatten()
            .find(|identity| identity.node == request.node)
            .copied()
            .ok_or(AdmissionError::IdentityFailure)?;
        let challenge = self.take_challenge(request.node, request.challenge, now_us)?;
        trusted.verify(request.identity, challenge)?;
        if (self.security.require_attestation || self.policy == AdmissionPolicy::Attested)
            && !request.attestation.valid_for(self.security)
        {
            return Err(AdmissionError::AttestationRejected)
        }
        if request.last_seen_generation < old.generation {
            return Err(AdmissionError::Replay)
        }
        if self.policy != AdmissionPolicy::Open {
            let token = request.token.ok_or(AdmissionError::InvitationInvalid)?;
            let invitation = self
                .invitations
                .iter()
                .flatten()
                .find(|invitation| invitation.token == token)
                .copied()
                .ok_or(AdmissionError::InvitationInvalid)?;
            invitation.usable(
                self.cluster,
                request.node,
                request.identity.fingerprint,
                request.requested_scope | INVITATION_SCOPE_REJOIN,
                now_us,
            )?;
            if invitation.one_time {
                if let Some(stored) = self
                    .invitations
                    .iter_mut()
                    .flatten()
                    .find(|stored| stored.token == invitation.token)
                {
                    stored.used = true;
                    stored.decision = InvitationDecision::Used;
                }
            }
        }
        if let Some(member) = self.members.iter_mut().flatten().find(|member| member.node == request.node) {
            member.state = MembershipState::Joined;
            member.reason = MembershipReason::Rejoined;
            member.negotiated = negotiated;
            member.endpoint = request.endpoint;
            member.generation = self.generation;
            member.last_seen_generation = request.last_seen_generation;
            member.membership_epoch = self.epoch;
        }
        self.commit_change_with_interruption(
            MembershipChangeKind::Rejoin,
            request.node,
            MembershipState::Joined,
            MembershipReason::Rejoined,
            now_us,
            injector,
        )
    }

    pub fn force_remove(
        &mut self,
        node: NodeId,
        confirm: bool,
        now_us: u64,
        hooks: &mut impl LeaveHooks,
    ) -> Result<LeaveOutcome, AdmissionError> {
        let mut no_interruption = NoInterruption;
        self.force_remove_with_interruption(node, confirm, now_us, hooks, &mut no_interruption)
    }

    pub fn force_remove_with_interruption<I: InterruptionInjector>(
        &mut self,
        node: NodeId,
        confirm: bool,
        now_us: u64,
        hooks: &mut impl LeaveHooks,
        injector: &mut I,
    ) -> Result<LeaveOutcome, AdmissionError> {
        if !confirm {
            return Err(AdmissionError::ForcedActionNotAuthorized)
        }
        let current = self.member(node).ok_or(AdmissionError::UnknownNode)?;
        if current.state == MembershipState::Expelled {
            return Err(AdmissionError::InvalidState)
        }
        self.epoch = self.epoch.checked_add(1).ok_or(AdmissionError::InvalidConfiguration)?;
        hooks.fence(node, self.epoch)?;
        hooks.revoke_capabilities(node)?;
        hooks.close_ipc_streams(node)?;
        let receipt = self.transition_with_epoch_with_interruption(
            node,
            MembershipState::Expelled,
            MembershipReason::Forced,
            MembershipChangeKind::Expel,
            now_us,
            injector,
        )?;
        Ok(LeaveOutcome {
            node,
            state: MembershipState::Expelled,
            epoch: self.epoch,
            forced: true,
            reconciled: false,
            receipt,
        })
    }

    fn commit_pending_with_interruption<I: InterruptionInjector>(
        &mut self,
        node: NodeId,
        now_us: u64,
        injector: &mut I,
    ) -> Result<QuorumReceipt, AdmissionError> {
        self.commit_change_with_interruption(
            MembershipChangeKind::Pending,
            node,
            MembershipState::Pending,
            MembershipReason::Requested,
            now_us,
            injector,
        )
        .map(|_| QuorumReceipt {
            acknowledged_votes: self.quorum_available,
            required_votes: self.quorum_required,
            audit_sequence: self.next_audit.saturating_sub(1),
        })
    }

    fn transition_with_interruption<I: InterruptionInjector>(
        &mut self,
        node: NodeId,
        state: MembershipState,
        reason: MembershipReason,
        kind: MembershipChangeKind,
        now_us: u64,
        injector: &mut I,
    ) -> Result<QuorumReceipt, AdmissionError> {
        let member = self.member(node).ok_or(AdmissionError::UnknownNode)?;
        if !valid_transition(member.state, state) {
            return Err(AdmissionError::InvalidState)
        }
        self.ensure_commit_ready()?;
        if let Some(member) = self.members.iter_mut().flatten().find(|member| member.node == node) {
            member.state = state;
            member.reason = reason;
            member.generation = self.generation;
            member.membership_epoch = self.epoch;
        }
        self.transition_with_epoch_with_interruption(node, state, reason, kind, now_us, injector)
    }

    fn transition_with_epoch_with_interruption<I: InterruptionInjector>(
        &mut self,
        node: NodeId,
        state: MembershipState,
        reason: MembershipReason,
        kind: MembershipChangeKind,
        now_us: u64,
        injector: &mut I,
    ) -> Result<QuorumReceipt, AdmissionError> {
        self.commit_change_with_interruption(kind, node, state, reason, now_us, injector)
    }

    pub fn commit_change_with_interruption<I: InterruptionInjector>(
        &mut self,
        kind: MembershipChangeKind,
        node: NodeId,
        state: MembershipState,
        reason: MembershipReason,
        now_us: u64,
        injector: &mut I,
    ) -> Result<QuorumReceipt, AdmissionError> {
        self.ensure_commit_ready()?;
        let sequence = self.next_audit;
        let slot = self
            .audit
            .iter_mut()
            .find(|record| record.is_none())
            .ok_or(AdmissionError::AuditCapacity)?;
        *slot = Some(AuditRecord {
            sequence,
            node,
            kind,
            state,
            reason,
            epoch: self.epoch,
            at_us: now_us,
            durable: true,
        });
        self.next_audit = self.next_audit.saturating_add(1);
        self.generation = self.generation.saturating_add(1);
        if injector.checkpoint(CrashDomain::Storage, CrashBoundary::JournalRecord) {
            return Err(AdmissionError::Interrupted)
        }
        Ok(QuorumReceipt {
            acknowledged_votes: self.quorum_available,
            required_votes: self.quorum_required,
            audit_sequence: sequence,
        })
    }

    fn ensure_commit_ready(&self) -> Result<(), AdmissionError> {
        if self.quorum_available < self.quorum_required {
            return Err(AdmissionError::QuorumUnavailable)
        }
        if !self.audit.iter().any(|record| record.is_none()) {
            return Err(AdmissionError::AuditCapacity)
        }
        Ok(())
    }

    fn set_invitation_decision(&mut self, node: NodeId, decision: InvitationDecision) {
        let Some(member) = self.member(node) else { return };
        let Some(token) = member.invitation_token else { return };
        if let Some(invitation) = self
            .invitations
            .iter_mut()
            .flatten()
            .find(|invitation| invitation.token == token)
        {
            invitation.decision = decision;
        }
    }

    fn take_challenge(
        &mut self,
        node: NodeId,
        challenge: [u8; 32],
        now_us: u64,
    ) -> Result<[u8; 32], AdmissionError> {
        let slot = self
            .challenges
            .iter_mut()
            .find(|entry| entry.is_some_and(|entry| entry.node == node))
            .ok_or(AdmissionError::IdentityFailure)?;
        let issued = slot.take().ok_or(AdmissionError::IdentityFailure)?;
        if issued.cluster != self.cluster
            || issued.challenge != challenge
            || now_us >= issued.expires_at_us
        {
            return Err(AdmissionError::Replay)
        }
        Ok(challenge)
    }
}

impl<const MEMBERS: usize, const INVITATIONS: usize, const AUDIT: usize> Default
    for AdmissionWorkflow<MEMBERS, INVITATIONS, AUDIT>
{
    fn default() -> Self {
        Self::new(
            ClusterId::from_valid_raw([1; 16]),
            AdmissionPolicy::Invitation,
            1,
            SecurityPolicy::DEFAULT,
        )
        .unwrap_or_else(|_| Self {
            cluster: ClusterId::from_valid_raw([1; 16]),
            policy: AdmissionPolicy::Invitation,
            quorum_required: 1,
            quorum_available: 1,
            security: SecurityPolicy::DEFAULT,
            epoch: 1,
            generation: 1,
            invitations: [None; INVITATIONS],
            trusted: [None; MEMBERS],
            challenges: [None; MEMBERS],
            members: [None; MEMBERS],
            audit: [None; AUDIT],
            next_audit: 1,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JoinRequest {
    pub node: NodeId,
    pub identity: IdentityProof,
    pub challenge: [u8; 32],
    pub token: Option<[u8; 32]>,
    pub requested_scope: u32,
    pub attestation: NodeAttestation,
    pub offer: ProtocolOffer,
    pub endpoint: AdmissionEndpoint,
    pub role: u8,
    pub voting: bool,
    pub last_seen_generation: u64,
}

fn valid_transition(from: MembershipState, to: MembershipState) -> bool {
    matches!(
        (from, to),
        (MembershipState::Pending, MembershipState::Approved)
            | (MembershipState::Pending, MembershipState::Rejected)
            | (MembershipState::Approved, MembershipState::Joined)
            | (MembershipState::Joined, MembershipState::Draining)
            | (MembershipState::Draining, MembershipState::Left)
            | (MembershipState::Joined, MembershipState::Fenced)
            | (MembershipState::Joined, MembershipState::Expelled)
            | (MembershipState::Approved, MembershipState::Fenced)
            | (MembershipState::Approved, MembershipState::Expelled)
    )
}

fn change_kind_byte(kind: MembershipChangeKind) -> u8 {
    match kind {
        MembershipChangeKind::Pending => 1,
        MembershipChangeKind::Approve => 2,
        MembershipChangeKind::Reject => 3,
        MembershipChangeKind::Join => 4,
        MembershipChangeKind::Drain => 5,
        MembershipChangeKind::Leave => 6,
        MembershipChangeKind::Fence => 7,
        MembershipChangeKind::Expel => 8,
        MembershipChangeKind::Rejoin => 9,
    }
}

fn change_kind_from_byte(value: u8) -> Option<MembershipChangeKind> {
    Some(match value {
        1 => MembershipChangeKind::Pending,
        2 => MembershipChangeKind::Approve,
        3 => MembershipChangeKind::Reject,
        4 => MembershipChangeKind::Join,
        5 => MembershipChangeKind::Drain,
        6 => MembershipChangeKind::Leave,
        7 => MembershipChangeKind::Fence,
        8 => MembershipChangeKind::Expel,
        9 => MembershipChangeKind::Rejoin,
        _ => return None,
    })
}

fn membership_state_byte(state: MembershipState) -> u8 {
    match state {
        MembershipState::Pending => 1,
        MembershipState::Approved => 2,
        MembershipState::Rejected => 3,
        MembershipState::Joined => 4,
        MembershipState::Draining => 5,
        MembershipState::Left => 6,
        MembershipState::Fenced => 7,
        MembershipState::Expelled => 8,
    }
}

fn membership_state_from_byte(value: u8) -> Option<MembershipState> {
    Some(match value {
        1 => MembershipState::Pending,
        2 => MembershipState::Approved,
        3 => MembershipState::Rejected,
        4 => MembershipState::Joined,
        5 => MembershipState::Draining,
        6 => MembershipState::Left,
        7 => MembershipState::Fenced,
        8 => MembershipState::Expelled,
        _ => return None,
    })
}

fn membership_reason_byte(reason: MembershipReason) -> u8 {
    match reason {
        MembershipReason::Requested => 1,
        MembershipReason::Approved => 2,
        MembershipReason::Rejected => 3,
        MembershipReason::IdentityMismatch => 4,
        MembershipReason::InvitationExpired => 5,
        MembershipReason::ProtocolMismatch => 6,
        MembershipReason::QuorumUnavailable => 7,
        MembershipReason::Drained => 8,
        MembershipReason::Forced => 9,
        MembershipReason::Fenced => 10,
        MembershipReason::Rejoined => 11,
    }
}

fn membership_reason_from_byte(value: u8) -> Option<MembershipReason> {
    Some(match value {
        1 => MembershipReason::Requested,
        2 => MembershipReason::Approved,
        3 => MembershipReason::Rejected,
        4 => MembershipReason::IdentityMismatch,
        5 => MembershipReason::InvitationExpired,
        6 => MembershipReason::ProtocolMismatch,
        7 => MembershipReason::QuorumUnavailable,
        8 => MembershipReason::Drained,
        9 => MembershipReason::Forced,
        10 => MembershipReason::Fenced,
        11 => MembershipReason::Rejoined,
        _ => return None,
    })
}

fn journal_checksum(input: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for (index, byte) in input.iter().enumerate() {
        if (48..56).contains(&index) {
            continue
        }
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3_u64);
    }
    hash
}
