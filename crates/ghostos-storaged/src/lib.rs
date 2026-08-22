#![no_std]
#![forbid(unsafe_code)]

pub use ghostos_service_scale::{
    EffectReceipt, HandoffReceipt, HandoffToken, InstanceId, InstanceState, JoinReceipt, RequestId,
    RouteDecision, ServiceKind, SessionId, SessionState, ScaleError, ScaleSnapshot, StorageScale,
};

pub use ghostos_numa::{NumaCounters, NumaDecision, NumaReport, NumaTopology, NumaTopologyError, PlacementKind, PlacementLocality};

mod bootstrap;
mod admission;
mod cache;
mod capability;
mod cluster;
mod failure;
mod membership;
mod protocol;
mod remote;
mod security;
mod sharding;
mod service;
mod state;
mod tiering;

pub use bootstrap::{
    AdmissionPolicy, AttestationEvidence, BOOTSTRAP_ENCODED_BYTES, BOOTSTRAP_SIGNATURE_BYTES,
    BootstrapAdvertisement, BootstrapChecks, BootstrapLifecycle, BootstrapNodeRecord,
    BootstrapToken, CLUSTER_BOOTSTRAP_FORMAT_VERSION, CLUSTER_BOOTSTRAP_MAGIC,
    CLUSTER_BOOTSTRAP_STATE_FILE, ClockHealth, ClusterBootstrapError, ClusterBootstrapState,
    ClusterCreateRequest, ClusterRootIdentity, EntropySource, MAX_BOOTSTRAP_ENDPOINT_BYTES,
    NodeCapabilities, ProtocolCompatibility, QuorumPolicy, SeedEntropy, TransportSet,
};
pub use admission::{
    AdmissionAuditJournal, AdmissionEndpoint, AdmissionError, AdmissionInvitation,
    AdmissionWorkflow, AddressSpaceLayout, AuditRecord, AttestationRoot, DiscoveryCandidate,
    DiscoveryDirectory, DiscoverySource, IdentityProof, InvitationDecision, JoinChallenge,
    JoinRequest, JournalError, LeaveHooks, LeaveOutcome, LeavePlan, MembershipChange, MembershipChangeKind,
    MembershipReason, MembershipRecord, MembershipState, NegotiatedAdmission, NodeAttestation,
    NodeIdentity, ProtocolOffer, QuorumReceipt, SecurityPolicy, TrustedIdentity,
    ADMISSION_AUDIT_FORMAT_VERSION, ADMISSION_AUDIT_MAGIC, ADMISSION_AUDIT_STATE_FILE,
    INVITATION_SCOPE_JOIN, INVITATION_SCOPE_RECONCILE,
    INVITATION_SCOPE_REJOIN, MAX_ADDRESS_SPACE_LAYOUT_BYTES, MAX_ADMISSION_AUDIT,
    MAX_ADMISSION_ENDPOINT_BYTES, MAX_ADMISSION_INVITATIONS, MAX_ADMISSION_MEMBERS,
    MAX_DISCOVERY_CANDIDATES,
};
pub use cache::{CacheError, CacheMode, CowCache, RemoteFileBackend};
pub use capability::{CapabilityError, StorageCapability, StorageRights};
pub use cluster::{
    Certificate, ClusterId, ClusterLifecycle, ClusterMetadata, ClusterMetadataCatalog,
    ClusterMetadataError, ClusterMetadataSnapshot, ClusterPatch, Invitation, MembershipIntent,
    MetadataText, TrustedPeer, CLUSTER_METADATA_FORMAT_VERSION, CLUSTER_METADATA_MAGIC,
    CLUSTER_METADATA_STATE_FILE, CLUSTER_ID_BYTES, MAX_CERTIFICATES, MAX_CLUSTER_ALIASES,
    MAX_CLUSTER_DESCRIPTION_BYTES, MAX_CLUSTER_NAME_BYTES, MAX_CLUSTERS, MAX_INVITATIONS,
    MAX_TRUSTED_PEERS, NODE_ID_BYTES, TOKEN_BYTES,
};
pub use failure::{
    ActionRequest, ClusterHealth, ClusterSample, FailureController, FailureError, FailureEvent,
    FailureKind, FailureReport, NodeHealthSample, OperatorAction, RecoveryHooks, RecoveryReceipt,
    RecoveryState, MAX_FAILURE_EVENTS, MAX_RECOVERY_NODES,
};
pub use membership::{
    propagate_membership_epoch, propagate_membership_epoch_with_admission, ConsensusCommit,
    ConsensusProposal, ElectionResult, MemberHealth,
    MemberRole, MemberSpec, MembershipAdvertisement, MembershipEpochConsumer, MembershipError,
    MembershipLabel, MembershipOperation, MembershipRegistry, MembershipSnapshot, QuorumView,
    RegistryMember, MAX_CONSENSUS_LOG, MAX_MEMBERSHIP_LABEL_BYTES, MAX_MEMBERSHIP_REGISTRY,
    MEMBERSHIP_FORMAT_VERSION, MEMBERSHIP_HEADER_BYTES, MEMBERSHIP_MAGIC, MEMBERSHIP_RECORD_BYTES,
    MEMBERSHIP_STATE_FILE,
};
pub use protocol::{
    BlockTransport, Endpoint, EndpointError, FabricTransport, NfsMinorVersion, ObjectKey,
    Protocol, ProtocolFeatures, S3Range, SmbDialect,
};
pub use remote::{
    IscsiSession, IscsiState, NvmeCommand, NvmeQueue, NvmeTarget, PnfsClient, PnfsDataServer,
    PnfsLayout, S3GetRequest, SmbChannel, SmbSession, SmbSessionState,
};
pub use security::{
    ClusterAuditRecord, ClusterCapability, ClusterCapabilityCaveat, ClusterKey, ClusterKeyring,
    ClusterOperation, ClusterOperations, ClusterRole, ClusterRoleSet, ClusterSecurityAudit,
    ClusterSecurityAuthority, ClusterSecurityPolicy, NodeCertificate, RevocationEntry,
    RevocationKind, SecureChannel, SecureFrame, SecurityDecision, SecurityError, TrafficClass,
    TrustRoot, TrustRootStore, DEFAULT_SECURITY_AUDIT_CAPACITY, MAX_CLUSTER_CAVEATS,
    MAX_CLUSTER_KEYS, MAX_REVOKED_ENTRIES, MAX_SECURE_PAYLOAD, MAX_TRUST_ROOTS,
    SECURE_FRAME_HEADER_BYTES, SECURE_FRAME_TAG_BYTES, SECURE_FRAME_WIRE_BYTES,
};
pub use sharding::{
    ClusterShardCoordinator, ConsistencyContract, RecoveryEvidence, ShardError, ShardId,
    ShardNamespace, ShardRecord, ShardRoute, ShardState, ShardWriteReceipt, MAX_SHARD_REPLICAS,
};
pub use service::{
    Completion, IoOperation, IoRequest, MountError, MountId, MountInfo, MountOptions,
    MountState, StorageBatchReport, StorageDaemon, StorageError, StoragePath, MAX_MOUNTS,
    MAX_PENDING_IO, MAX_STORAGE_BATCH,
};
pub use state::{MountCatalog, MountStateError, MOUNTS_STATE_FILE, MOUNTS_STATE_MAGIC};
pub use tiering::{
    CostPolicy, DurabilityClass, DurabilityPolicy, HeatPolicy, TierBackend, TierMoveKind,
    TierMovePlan, TierMoveReceipt, TierMoveRecord, TierMoveState, TierObject, TieringError,
    TieringManager, TieringPolicy, StorageTier, MAX_TIER_MOVES, MAX_TIER_OBJECTS,
    TIERING_FORMAT_VERSION, TIERING_HEADER_BYTES, TIERING_MAGIC, TIERING_STATE_FILE,
};

/// Logical storage namespace used by remote mounts.
pub const STORAGE_LOGICAL: &str = "SYS$STORAGE:";

/// Maximum bytes accepted by one remote data operation.
pub const MAX_IO_BYTES: usize = 1024 * 1024;

/// Maximum bytes accepted by one S3 stream segment.
pub const MAX_S3_SEGMENT_BYTES: usize = 1024 * 1024;

/// A netd-owned stream that can hand network buffers to a storage consumer
/// without making the storage daemon own or copy those buffers.
pub trait NetworkBufferStream {
    fn next<'a>(&'a mut self) -> Option<&'a [u8]>;
}

/// Consume S3 data directly from the zero-copy ingress queue owned by
/// `ghostos-netd`. Each packet is loaned only for the duration of `accept`.
pub fn stream_s3_netd<const CAPACITY: usize, const MTU: usize>(
    queue: &mut ghostos_netd::PacketQueue<CAPACITY, MTU>,
    sink: &mut impl S3StreamSink,
) -> Result<u64, StorageError> {
    let mut total = 0u64;
    while let Ok(packet) = queue.dequeue() {
        let buffer = packet.frame();
        sink.accept(buffer).map_err(StorageError::StreamFailed)?;
        total = total.saturating_add(buffer.len() as u64);
    }
    sink.finish().map_err(StorageError::StreamFailed)?;
    Ok(total)
}

/// Consume an S3 body using borrowed buffers supplied by `ghostos-netd`.
pub fn stream_s3_body(
    stream: &mut impl NetworkBufferStream,
    sink: &mut impl S3StreamSink,
) -> Result<u64, StorageError> {
    let mut total = 0u64;
    while let Some(buffer) = stream.next() {
        if buffer.len() > MAX_S3_SEGMENT_BYTES {
            return Err(StorageError::BufferTooLarge)
        }
        sink.accept(buffer).map_err(StorageError::StreamFailed)?;
        total = total.saturating_add(buffer.len() as u64);
    }
    sink.finish().map_err(StorageError::StreamFailed)?;
    Ok(total)
}

pub trait S3StreamSink {
    fn accept(&mut self, buffer: &[u8]) -> Result<(), u16>;
    fn finish(&mut self) -> Result<(), u16>;
}
