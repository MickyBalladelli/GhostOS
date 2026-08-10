use synos_auth::CapabilityKey;
use synos_durability::{CrashBoundary, CrashDomain, InterruptionInjector, NoInterruption};
use synos_fabric::NodeId;
use synos_observability::{CorrelationId, next_correlation_id};
use synos_status::{IntoStatus, Status};

use crate::{ClusterId, NodeAttestation, NodeCapabilities};

pub const MAX_CLUSTER_CAVEATS: usize = 4;
pub const MAX_CLUSTER_KEYS: usize = 8;
pub const MAX_TRUST_ROOTS: usize = 8;
pub const MAX_REVOKED_ENTRIES: usize = 64;
pub const DEFAULT_SECURITY_AUDIT_CAPACITY: usize = 256;
pub const MAX_SECURE_PAYLOAD: usize = 256;
pub const SECURE_FRAME_HEADER_BYTES: usize = 26;
pub const SECURE_FRAME_TAG_BYTES: usize = 32;
pub const SECURE_FRAME_WIRE_BYTES: usize =
    SECURE_FRAME_HEADER_BYTES + MAX_SECURE_PAYLOAD + SECURE_FRAME_TAG_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ClusterRole {
    Administrator = 1,
    Operator = 2,
    Auditor = 3,
    NodeOwner = 4,
    Workload = 5,
    ReadOnly = 6,
}

impl ClusterRole {
    pub const fn operations(self) -> ClusterOperations {
        match self {
            Self::Administrator => ClusterOperations::ALL,
            Self::Operator => ClusterOperations::from_bits(
                ClusterOperations::JOIN.bits()
                    | ClusterOperations::LEAVE.bits()
                    | ClusterOperations::REMOVE.bits()
                    | ClusterOperations::MODIFY.bits()
                    | ClusterOperations::INVITE.bits()
                    | ClusterOperations::FENCE.bits()
                    | ClusterOperations::RESOURCE_MANAGE.bits()
                    | ClusterOperations::OBSERVE.bits(),
            ),
            Self::Auditor | Self::ReadOnly => ClusterOperations::OBSERVE,
            Self::NodeOwner => ClusterOperations::JOIN
                .union(ClusterOperations::LEAVE)
                .union(ClusterOperations::RESOURCE_MANAGE)
                .union(ClusterOperations::OBSERVE),
            Self::Workload => ClusterOperations::RESOURCE_MANAGE,
        }
    }

    pub const fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::Administrator,
            2 => Self::Operator,
            3 => Self::Auditor,
            4 => Self::NodeOwner,
            5 => Self::Workload,
            6 => Self::ReadOnly,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ClusterRoleSet(u8);

impl ClusterRoleSet {
    pub const ADMINISTRATOR: Self = Self(1 << 0);
    pub const OPERATOR: Self = Self(1 << 1);
    pub const AUDITOR: Self = Self(1 << 2);
    pub const NODE_OWNER: Self = Self(1 << 3);
    pub const WORKLOAD: Self = Self(1 << 4);
    pub const READ_ONLY: Self = Self(1 << 5);
    pub const ALL: Self = Self(0x3f);

    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits == 0 || bits & !Self::ALL.0 != 0 {
            None
        } else {
            Some(Self(bits))
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, role: ClusterRole) -> bool {
        self.0 & role_bit(role) != 0
    }
}

const fn role_bit(role: ClusterRole) -> u8 {
    1 << (role as u8 - 1)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ClusterOperation {
    Create = 1,
    Join = 2,
    Leave = 3,
    Remove = 4,
    Modify = 5,
    Invite = 6,
    Fence = 7,
    ResourceManage = 8,
    Observe = 9,
    Delegate = 10,
}

impl ClusterOperation {
    pub const fn bit(self) -> u16 {
        1 << (self as u8 - 1)
    }

    pub const fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::Create,
            2 => Self::Join,
            3 => Self::Leave,
            4 => Self::Remove,
            5 => Self::Modify,
            6 => Self::Invite,
            7 => Self::Fence,
            8 => Self::ResourceManage,
            9 => Self::Observe,
            10 => Self::Delegate,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ClusterOperations(u16);

impl ClusterOperations {
    pub const CREATE: Self = Self(1 << 0);
    pub const JOIN: Self = Self(1 << 1);
    pub const LEAVE: Self = Self(1 << 2);
    pub const REMOVE: Self = Self(1 << 3);
    pub const MODIFY: Self = Self(1 << 4);
    pub const INVITE: Self = Self(1 << 5);
    pub const FENCE: Self = Self(1 << 6);
    pub const RESOURCE_MANAGE: Self = Self(1 << 7);
    pub const OBSERVE: Self = Self(1 << 8);
    pub const DELEGATE: Self = Self(1 << 9);
    pub const ALL: Self = Self((1 << 10) - 1);

    pub const fn from_bits(bits: u16) -> Self {
        Self(bits & Self::ALL.0)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn for_operation(operation: ClusterOperation) -> Self {
        Self(operation.bit())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterCapabilityCaveat {
    pub operations: ClusterOperations,
    pub expires_at_us: u64,
}

impl ClusterCapabilityCaveat {
    fn bytes(self) -> [u8; 16] {
        let mut bytes = [0; 16];
        bytes[..2].copy_from_slice(&self.operations.bits().to_be_bytes());
        bytes[8..16].copy_from_slice(&self.expires_at_us.to_be_bytes());
        bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterCapability {
    pub issuer: NodeId,
    pub subject: NodeId,
    pub cluster: ClusterId,
    pub role: ClusterRole,
    pub operations: ClusterOperations,
    pub not_before_us: u64,
    pub expires_at_us: u64,
    pub key_epoch: u64,
    pub nonce: u64,
    caveats: [Option<ClusterCapabilityCaveat>; MAX_CLUSTER_CAVEATS],
    tag: [u8; 32],
}

impl ClusterCapability {
    pub fn issue(
        key: CapabilityKey,
        issuer: NodeId,
        subject: NodeId,
        cluster: ClusterId,
        role: ClusterRole,
        operations: ClusterOperations,
        not_before_us: u64,
        expires_at_us: u64,
        key_epoch: u64,
        nonce: u64,
    ) -> Result<Self, SecurityError> {
        if operations.is_empty()
            || !role.operations().contains(operations)
            || expires_at_us <= not_before_us
            || key_epoch == 0
            || nonce == 0
        {
            return Err(SecurityError::InvalidCapability)
        }
        let mut capability = Self {
            issuer,
            subject,
            cluster,
            role,
            operations,
            not_before_us,
            expires_at_us,
            key_epoch,
            nonce,
            caveats: [None; MAX_CLUSTER_CAVEATS],
            tag: [0; 32],
        };
        capability.tag = key
            .authenticate(&capability.base_bytes())
            .map_err(|_| SecurityError::InvalidCapability)?;
        Ok(capability)
    }

    pub const fn key_epoch(self) -> u64 {
        self.key_epoch
    }

    pub const fn nonce(self) -> u64 {
        self.nonce
    }

    pub fn caveats(&self) -> impl Iterator<Item = ClusterCapabilityCaveat> + '_ {
        self.caveats.iter().flatten().copied()
    }

    pub fn effective_operations(&self) -> ClusterOperations {
        self.caveats().fold(self.operations, |operations, caveat| {
            operations.intersection(caveat.operations)
        })
    }

    pub fn effective_expiry(&self) -> u64 {
        self.caveats().fold(self.expires_at_us, |expiry, caveat| {
            expiry.min(caveat.expires_at_us)
        })
    }

    pub fn attenuate(
        mut self,
        caveat: ClusterCapabilityCaveat,
    ) -> Result<Self, SecurityError> {
        if caveat.operations.is_empty()
            || !self.effective_operations().contains(caveat.operations)
            || caveat.expires_at_us > self.effective_expiry()
        {
            return Err(SecurityError::AccessDenied)
        }
        let slot = self
            .caveats
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(SecurityError::CaveatCapacity)?;
        self.tag = CapabilityKey::new(self.tag)
            .authenticate(&caveat.bytes())
            .map_err(|_| SecurityError::InvalidCapability)?;
        *slot = Some(caveat);
        Ok(self)
    }

    pub fn verify(
        &self,
        key: CapabilityKey,
        actor: NodeId,
        operation: ClusterOperation,
        now_us: u64,
    ) -> Result<(), SecurityError> {
        if actor != self.subject
            || !self
                .effective_operations()
                .contains(ClusterOperations::for_operation(operation))
            || now_us < self.not_before_us
            || now_us >= self.effective_expiry()
            || !self.valid_caveats()
        {
            return Err(SecurityError::AccessDenied)
        }
        let mut expected = key
            .authenticate(&self.base_bytes())
            .map_err(|_| SecurityError::InvalidCapability)?;
        for caveat in self.caveats() {
            expected = CapabilityKey::new(expected)
                .authenticate(&caveat.bytes())
                .map_err(|_| SecurityError::InvalidCapability)?;
        }
        if constant_time_equal(&expected, &self.tag) {
            Ok(())
        } else {
            Err(SecurityError::InvalidSignature)
        }
    }

    fn valid_caveats(&self) -> bool {
        let mut empty = false;
        let mut operations = self.operations;
        let mut expiry = self.expires_at_us;
        for entry in self.caveats {
            let Some(caveat) = entry else {
                empty = true;
                continue
            };
            if empty
                || caveat.operations.is_empty()
                || !operations.contains(caveat.operations)
                || caveat.expires_at_us > expiry
            {
                return false
            }
            operations = caveat.operations;
            expiry = caveat.expires_at_us;
        }
        true
    }

    fn base_bytes(&self) -> [u8; 96] {
        let mut bytes = [0; 96];
        bytes[..4].copy_from_slice(b"SYCC");
        bytes[4..20].copy_from_slice(&self.cluster.raw());
        bytes[20..24].copy_from_slice(&self.issuer.raw().to_be_bytes());
        bytes[24..28].copy_from_slice(&self.subject.raw().to_be_bytes());
        bytes[28] = self.role as u8;
        bytes[30..32].copy_from_slice(&self.operations.bits().to_be_bytes());
        bytes[32..40].copy_from_slice(&self.not_before_us.to_be_bytes());
        bytes[40..48].copy_from_slice(&self.expires_at_us.to_be_bytes());
        bytes[48..56].copy_from_slice(&self.key_epoch.to_be_bytes());
        bytes[56..64].copy_from_slice(&self.nonce.to_be_bytes());
        bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrustRoot {
    pub id: [u8; 16],
    pub key: CapabilityKey,
    pub allowed_roles: ClusterRoleSet,
    pub allowed_attestation_roots: u8,
    pub revoked: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeCertificate {
    pub id: [u8; 32],
    pub cluster: ClusterId,
    pub node: NodeId,
    pub fingerprint: [u8; 32],
    pub root_id: [u8; 16],
    pub role: ClusterRole,
    pub issued_at_us: u64,
    pub expires_at_us: u64,
    pub revoked: bool,
    pub signature: [u8; 32],
}

impl NodeCertificate {
    pub fn issue(
        root: TrustRoot,
        id: [u8; 32],
        cluster: ClusterId,
        node: NodeId,
        fingerprint: [u8; 32],
        role: ClusterRole,
        issued_at_us: u64,
        expires_at_us: u64,
    ) -> Result<Self, SecurityError> {
        if root.revoked
            || id.iter().all(|byte| *byte == 0)
            || fingerprint.iter().all(|byte| *byte == 0)
            || root.id.iter().all(|byte| *byte == 0)
            || !root.allowed_roles.contains(role)
            || expires_at_us <= issued_at_us
        {
            return Err(SecurityError::InvalidCertificate)
        }
        let mut certificate = Self {
            id,
            cluster,
            node,
            fingerprint,
            root_id: root.id,
            role,
            issued_at_us,
            expires_at_us,
            revoked: false,
            signature: [0; 32],
        };
        certificate.signature = root
            .key
            .authenticate(&certificate.material())
            .map_err(|_| SecurityError::InvalidCertificate)?;
        Ok(certificate)
    }

    fn verify(&self, root: TrustRoot, cluster: ClusterId, now_us: u64) -> Result<(), SecurityError> {
        if self.cluster != cluster
            || self.root_id != root.id
            || root.revoked
            || self.revoked
            || now_us < self.issued_at_us
            || now_us >= self.expires_at_us
            || !root.allowed_roles.contains(self.role)
        {
            return Err(SecurityError::AccessDenied)
        }
        let expected = root
            .key
            .authenticate(&self.material())
            .map_err(|_| SecurityError::InvalidCertificate)?;
        if constant_time_equal(&expected, &self.signature) {
            Ok(())
        } else {
            Err(SecurityError::InvalidSignature)
        }
    }

    fn material(&self) -> [u8; 128] {
        let mut bytes = [0; 128];
        bytes[..4].copy_from_slice(b"SYNC");
        bytes[4..20].copy_from_slice(&self.cluster.raw());
        bytes[20..52].copy_from_slice(&self.id);
        bytes[52..56].copy_from_slice(&self.node.raw().to_be_bytes());
        bytes[56..88].copy_from_slice(&self.fingerprint);
        bytes[88..104].copy_from_slice(&self.root_id);
        bytes[104] = self.role as u8;
        bytes[112..120].copy_from_slice(&self.issued_at_us.to_be_bytes());
        bytes[120..128].copy_from_slice(&self.expires_at_us.to_be_bytes());
        bytes
    }
}

#[derive(Clone, Copy)]
pub struct TrustRootStore<const CAPACITY: usize = MAX_TRUST_ROOTS> {
    roots: [Option<TrustRoot>; CAPACITY],
}

impl<const CAPACITY: usize> TrustRootStore<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            roots: [None; CAPACITY],
        }
    }

    pub fn add(&mut self, root: TrustRoot) -> Result<(), SecurityError> {
        if root.id.iter().all(|byte| *byte == 0)
            || root.allowed_roles.bits() == 0
            || root.allowed_attestation_roots == 0
            || self.roots.iter().flatten().any(|entry| entry.id == root.id)
        {
            return Err(SecurityError::InvalidTrustRoot)
        }
        let slot = self
            .roots
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(SecurityError::TrustRootCapacity)?;
        *slot = Some(root);
        Ok(())
    }

    pub fn revoke(&mut self, id: [u8; 16]) -> Result<(), SecurityError> {
        let root = self
            .roots
            .iter_mut()
            .flatten()
            .find(|root| root.id == id)
            .ok_or(SecurityError::TrustRootNotFound)?;
        root.revoked = true;
        Ok(())
    }

    pub fn root(&self, id: [u8; 16]) -> Option<TrustRoot> {
        self.roots.iter().flatten().find(|root| root.id == id).copied()
    }

    pub fn verify(
        &self,
        certificate: NodeCertificate,
        cluster: ClusterId,
        now_us: u64,
    ) -> Result<(), SecurityError> {
        let root = self
            .root(certificate.root_id)
            .ok_or(SecurityError::TrustRootNotFound)?;
        certificate.verify(root, cluster, now_us)
    }

    pub fn roots(&self) -> impl Iterator<Item = TrustRoot> + '_ {
        self.roots.iter().flatten().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterSecurityPolicy {
    pub required_capabilities: NodeCapabilities,
    pub require_certificate: bool,
    pub require_attestation: bool,
    pub require_encryption: bool,
    pub allowed_attestation_roots: u8,
}

impl ClusterSecurityPolicy {
    pub const STRICT: Self = Self {
        required_capabilities: NodeCapabilities::CONTROL_PLANE
            .with(NodeCapabilities::PERSISTENT_METADATA)
            .with(NodeCapabilities::NETWORK)
            .with(NodeCapabilities::CLOCK),
        require_certificate: true,
        require_attestation: true,
        require_encryption: true,
        allowed_attestation_roots: u8::MAX,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterKey {
    pub epoch: u64,
    pub id: [u8; 16],
    pub issued_at_us: u64,
    pub key: CapabilityKey,
    pub revoked: bool,
}

#[derive(Clone, Copy)]
pub struct ClusterKeyring<const CAPACITY: usize = MAX_CLUSTER_KEYS> {
    keys: [Option<ClusterKey>; CAPACITY],
    current_epoch: u64,
}

impl<const CAPACITY: usize> ClusterKeyring<CAPACITY> {
    pub fn new(key: CapabilityKey, id: [u8; 16], issued_at_us: u64) -> Result<Self, SecurityError> {
        if CAPACITY == 0 || id.iter().all(|byte| *byte == 0) {
            return Err(SecurityError::InvalidKey)
        }
        let mut keyring = Self {
            keys: [None; CAPACITY],
            current_epoch: 1,
        };
        keyring.keys[0] = Some(ClusterKey {
            epoch: 1,
            id,
            issued_at_us,
            key,
            revoked: false,
        });
        Ok(keyring)
    }

    pub const fn current_epoch(&self) -> u64 {
        self.current_epoch
    }

    pub fn rotate(
        &mut self,
        key: CapabilityKey,
        id: [u8; 16],
        issued_at_us: u64,
    ) -> Result<u64, SecurityError> {
        if id.iter().all(|byte| *byte == 0)
            || self.keys.iter().flatten().any(|entry| entry.id == id)
        {
            return Err(SecurityError::InvalidKey)
        }
        let epoch = self
            .current_epoch
            .checked_add(1)
            .ok_or(SecurityError::KeyCapacity)?;
        let slot = self
            .keys
            .iter_mut()
            .find(|entry| entry.is_none() || entry.is_some_and(|entry| entry.revoked))
            .ok_or(SecurityError::KeyCapacity)?;
        *slot = Some(ClusterKey {
            epoch,
            id,
            issued_at_us,
            key,
            revoked: false,
        });
        self.current_epoch = epoch;
        Ok(epoch)
    }

    pub fn revoke_epoch(&mut self, epoch: u64) -> Result<(), SecurityError> {
        if epoch == self.current_epoch {
            return Err(SecurityError::CannotRevokeCurrentKey)
        }
        let key = self
            .keys
            .iter_mut()
            .flatten()
            .find(|entry| entry.epoch == epoch)
            .ok_or(SecurityError::KeyNotFound)?;
        key.revoked = true;
        Ok(())
    }

    fn key(&self, epoch: u64) -> Option<ClusterKey> {
        self.keys
            .iter()
            .flatten()
            .find(|entry| entry.epoch == epoch && !entry.revoked)
            .copied()
    }

    pub fn keys(&self) -> impl Iterator<Item = ClusterKey> + '_ {
        self.keys.iter().flatten().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RevocationKind {
    Node = 1,
    Invitation = 2,
    Certificate = 3,
    Capability = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevocationEntry {
    pub kind: RevocationKind,
    pub id: [u8; 32],
    pub at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityDecision {
    Allowed,
    Denied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterAuditRecord {
    pub sequence: u64,
    pub correlation: CorrelationId,
    pub actor: NodeId,
    pub target: Option<NodeId>,
    pub operation: ClusterOperation,
    pub role: ClusterRole,
    pub decision: SecurityDecision,
    pub at_us: u64,
    pub key_epoch: u64,
}

#[derive(Clone, Copy)]
pub struct ClusterSecurityAudit<const CAPACITY: usize = DEFAULT_SECURITY_AUDIT_CAPACITY> {
    records: [Option<ClusterAuditRecord>; CAPACITY],
    next_sequence: u64,
}

impl<const CAPACITY: usize> ClusterSecurityAudit<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            records: [None; CAPACITY],
            next_sequence: 1,
        }
    }

    fn record(
        &mut self,
        actor: NodeId,
        target: Option<NodeId>,
        operation: ClusterOperation,
        role: ClusterRole,
        decision: SecurityDecision,
        at_us: u64,
        key_epoch: u64,
    ) -> Result<(), SecurityError> {
        let slot = self
            .records
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(SecurityError::AuditCapacity)?;
        let sequence = self.next_sequence;
        *slot = Some(ClusterAuditRecord {
            sequence,
            correlation: next_correlation_id(actor.raw()),
            actor,
            target,
            operation,
            role,
            decision,
            at_us,
            key_epoch,
        });
        self.next_sequence = self.next_sequence.saturating_add(1);
        Ok(())
    }

    pub fn records(&self) -> impl Iterator<Item = ClusterAuditRecord> + '_ {
        self.records.iter().flatten().copied()
    }
}

#[derive(Clone, Copy)]
pub struct ClusterSecurityAuthority<
    const ROOTS: usize = MAX_TRUST_ROOTS,
    const KEYS: usize = MAX_CLUSTER_KEYS,
    const AUDIT: usize = DEFAULT_SECURITY_AUDIT_CAPACITY,
    const REVOKED: usize = MAX_REVOKED_ENTRIES,
> {
    pub cluster: ClusterId,
    pub policy: ClusterSecurityPolicy,
    pub keyring: ClusterKeyring<KEYS>,
    pub trust_roots: TrustRootStore<ROOTS>,
    revoked: [Option<RevocationEntry>; REVOKED],
    pub audit: ClusterSecurityAudit<AUDIT>,
}

impl<const ROOTS: usize, const KEYS: usize, const AUDIT: usize, const REVOKED: usize>
    ClusterSecurityAuthority<ROOTS, KEYS, AUDIT, REVOKED>
{
    pub fn new(
        cluster: ClusterId,
        key: CapabilityKey,
        key_id: [u8; 16],
        policy: ClusterSecurityPolicy,
        now_us: u64,
    ) -> Result<Self, SecurityError> {
        Ok(Self {
            cluster,
            policy,
            keyring: ClusterKeyring::new(key, key_id, now_us)?,
            trust_roots: TrustRootStore::new(),
            revoked: [None; REVOKED],
            audit: ClusterSecurityAudit::new(),
        })
    }

    pub fn bootstrap_capability(
        &self,
        subject: NodeId,
        role: ClusterRole,
        expires_at_us: u64,
        nonce: u64,
    ) -> Result<ClusterCapability, SecurityError> {
        let key = self
            .keyring
            .key(self.keyring.current_epoch())
            .ok_or(SecurityError::KeyNotFound)?;
        ClusterCapability::issue(
            key.key,
            subject,
            subject,
            self.cluster,
            role,
            role.operations(),
            0,
            expires_at_us,
            key.epoch,
            nonce,
        )
    }

    pub fn delegate(
        &mut self,
        parent: &ClusterCapability,
        issuer: NodeId,
        subject: NodeId,
        role: ClusterRole,
        operations: ClusterOperations,
        expires_at_us: u64,
        now_us: u64,
        nonce: u64,
    ) -> Result<ClusterCapability, SecurityError> {
        self.authorize(parent, issuer, ClusterOperation::Delegate, Some(subject), now_us)?;
        if !role.operations().contains(operations) || !parent.effective_operations().contains(operations) {
            return Err(SecurityError::AccessDenied)
        }
        let key = self
            .keyring
            .key(self.keyring.current_epoch())
            .ok_or(SecurityError::KeyNotFound)?;
        ClusterCapability::issue(
            key.key,
            issuer,
            subject,
            self.cluster,
            role,
            operations,
            now_us,
            expires_at_us,
            key.epoch,
            nonce,
        )
    }

    pub fn authorize(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        operation: ClusterOperation,
        target: Option<NodeId>,
        now_us: u64,
    ) -> Result<(), SecurityError> {
        let key = self.keyring.key(capability.key_epoch);
        let result = if capability.cluster != self.cluster
            || self.is_revoked(RevocationKind::Node, node_bytes(actor))
            || self.is_revoked(RevocationKind::Capability, u64_bytes(capability.nonce))
        {
            Err(SecurityError::AccessDenied)
        } else if let Some(key) = key {
            capability.verify(key.key, actor, operation, now_us)
        } else {
            Err(SecurityError::KeyNotFound)
        };
        let audit = self.audit.record(
            actor,
            target,
            operation,
            capability.role,
            if result.is_ok() {
                SecurityDecision::Allowed
            } else {
                SecurityDecision::Denied
            },
            now_us,
            capability.key_epoch,
        );
        audit?;
        result
    }

    pub fn admit_node(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        certificate: Option<NodeCertificate>,
        attestation: NodeAttestation,
        capabilities: NodeCapabilities,
        now_us: u64,
    ) -> Result<ClusterRole, SecurityError> {
        self.authorize(capability, actor, ClusterOperation::Join, Some(actor), now_us)?;
        let result = self.verify_admission(actor, certificate, attestation, capabilities, now_us);
        self.audit.record(
            actor,
            Some(actor),
            ClusterOperation::Join,
            certificate.map_or(capability.role, |certificate| certificate.role),
            if result.is_ok() {
                SecurityDecision::Allowed
            } else {
                SecurityDecision::Denied
            },
            now_us,
            capability.key_epoch,
        )?;
        result
    }

    pub fn rotate_key(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        key: CapabilityKey,
        id: [u8; 16],
        now_us: u64,
    ) -> Result<u64, SecurityError> {
        let mut no_interruption = NoInterruption;
        self.rotate_key_with_interruption(capability, actor, key, id, now_us, &mut no_interruption)
    }

    pub fn rotate_key_with_interruption<I: InterruptionInjector>(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        key: CapabilityKey,
        id: [u8; 16],
        now_us: u64,
        injector: &mut I,
    ) -> Result<u64, SecurityError> {
        self.authorize(capability, actor, ClusterOperation::Modify, None, now_us)?;
        let epoch = self.keyring.rotate(key, id, now_us)?;
        if injector.checkpoint(CrashDomain::Storage, CrashBoundary::CapabilityChange) {
            return Err(SecurityError::Interrupted)
        }
        Ok(epoch)
    }

    pub fn revoke_key(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        epoch: u64,
        now_us: u64,
    ) -> Result<(), SecurityError> {
        let mut no_interruption = NoInterruption;
        self.revoke_key_with_interruption(capability, actor, epoch, now_us, &mut no_interruption)
    }

    pub fn revoke_key_with_interruption<I: InterruptionInjector>(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        epoch: u64,
        now_us: u64,
        injector: &mut I,
    ) -> Result<(), SecurityError> {
        self.authorize(capability, actor, ClusterOperation::Modify, None, now_us)?;
        self.keyring.revoke_epoch(epoch)?;
        if injector.checkpoint(CrashDomain::Storage, CrashBoundary::CapabilityChange) {
            return Err(SecurityError::Interrupted)
        }
        Ok(())
    }

    pub fn open_channel(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        remote: NodeId,
        initiator: bool,
        epoch: u64,
        now_us: u64,
    ) -> Result<SecureChannel, SecurityError> {
        self.authorize(capability, actor, ClusterOperation::Observe, Some(remote), now_us)?;
        if self.policy.require_encryption || self.keyring.key(epoch).is_some() {
            let key = self.keyring.key(epoch).ok_or(SecurityError::KeyNotFound)?;
            SecureChannel::new(key.key, actor, remote, initiator, epoch)
        } else {
            Err(SecurityError::InvalidKey)
        }
    }

    pub fn revoke_node(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        node: NodeId,
        now_us: u64,
    ) -> Result<(), SecurityError> {
        let mut no_interruption = NoInterruption;
        self.revoke_node_with_interruption(capability, actor, node, now_us, &mut no_interruption)
    }

    pub fn revoke_node_with_interruption<I: InterruptionInjector>(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        node: NodeId,
        now_us: u64,
        injector: &mut I,
    ) -> Result<(), SecurityError> {
        self.authorize(capability, actor, ClusterOperation::Fence, Some(node), now_us)?;
        self.revoke_with_interruption(RevocationKind::Node, node_bytes(node), now_us, injector)
    }

    pub fn revoke_invitation(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        token: [u8; 32],
        now_us: u64,
    ) -> Result<(), SecurityError> {
        let mut no_interruption = NoInterruption;
        self.revoke_invitation_with_interruption(capability, actor, token, now_us, &mut no_interruption)
    }

    pub fn revoke_invitation_with_interruption<I: InterruptionInjector>(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        token: [u8; 32],
        now_us: u64,
        injector: &mut I,
    ) -> Result<(), SecurityError> {
        self.authorize(capability, actor, ClusterOperation::Invite, None, now_us)?;
        self.revoke_with_interruption(RevocationKind::Invitation, token, now_us, injector)
    }

    pub fn revoke_certificate(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        certificate: [u8; 32],
        now_us: u64,
    ) -> Result<(), SecurityError> {
        let mut no_interruption = NoInterruption;
        self.revoke_certificate_with_interruption(capability, actor, certificate, now_us, &mut no_interruption)
    }

    pub fn revoke_certificate_with_interruption<I: InterruptionInjector>(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        certificate: [u8; 32],
        now_us: u64,
        injector: &mut I,
    ) -> Result<(), SecurityError> {
        self.authorize(capability, actor, ClusterOperation::Modify, None, now_us)?;
        self.revoke_with_interruption(RevocationKind::Certificate, certificate, now_us, injector)
    }

    pub fn revoke_capability(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        nonce: u64,
        now_us: u64,
    ) -> Result<(), SecurityError> {
        let mut no_interruption = NoInterruption;
        self.revoke_capability_with_interruption(capability, actor, nonce, now_us, &mut no_interruption)
    }

    pub fn revoke_capability_with_interruption<I: InterruptionInjector>(
        &mut self,
        capability: &ClusterCapability,
        actor: NodeId,
        nonce: u64,
        now_us: u64,
        injector: &mut I,
    ) -> Result<(), SecurityError> {
        self.authorize(capability, actor, ClusterOperation::Modify, None, now_us)?;
        self.revoke_with_interruption(
            RevocationKind::Capability,
            u64_bytes(nonce),
            now_us,
            injector,
        )
    }

    fn verify_admission(
        &self,
        actor: NodeId,
        certificate: Option<NodeCertificate>,
        attestation: NodeAttestation,
        capabilities: NodeCapabilities,
        now_us: u64,
    ) -> Result<ClusterRole, SecurityError> {
        if !capabilities.contains(self.policy.required_capabilities) {
            return Err(SecurityError::MissingNodeCapability)
        }
        let role = if let Some(certificate) = certificate {
            if certificate.node != actor
                || self.is_revoked(RevocationKind::Certificate, certificate.id)
            {
                return Err(SecurityError::AccessDenied)
            }
            self.trust_roots.verify(certificate, self.cluster, now_us)?;
            let root = self
                .trust_roots
                .root(certificate.root_id)
                .ok_or(SecurityError::TrustRootNotFound)?;
            if root.allowed_attestation_roots & (1 << attestation.root as u8) == 0 {
                return Err(SecurityError::AttestationRejected)
            }
            certificate.role
        } else if self.policy.require_certificate {
            return Err(SecurityError::CertificateRequired)
        } else {
            ClusterRole::NodeOwner
        };
        if self.policy.require_attestation {
            if !attestation.evidence.verified
                || attestation.evidence.measurement.iter().all(|byte| *byte == 0)
                || self.policy.allowed_attestation_roots & (1 << attestation.root as u8) == 0
            {
                return Err(SecurityError::AttestationRejected)
            }
        }
        Ok(role)
    }

    fn revoke_with_interruption<I: InterruptionInjector>(
        &mut self,
        kind: RevocationKind,
        id: [u8; 32],
        at_us: u64,
        injector: &mut I,
    ) -> Result<(), SecurityError> {
        if self.is_revoked(kind, id) {
            return Err(SecurityError::AlreadyRevoked)
        }
        let slot = self
            .revoked
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(SecurityError::RevocationCapacity)?;
        *slot = Some(RevocationEntry { kind, id, at_us });
        if injector.checkpoint(CrashDomain::Storage, CrashBoundary::CapabilityChange) {
            return Err(SecurityError::Interrupted)
        }
        Ok(())
    }

    fn is_revoked(&self, kind: RevocationKind, id: [u8; 32]) -> bool {
        self.revoked
            .iter()
            .flatten()
            .any(|entry| entry.kind == kind && entry.id == id)
    }

    pub fn revocations(&self) -> impl Iterator<Item = RevocationEntry> + '_ {
        self.revoked.iter().flatten().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TrafficClass {
    Membership = 1,
    ControlPlane = 2,
    Dlm = 3,
    Dsm = 4,
    Ipc = 5,
    Telemetry = 6,
}

impl TrafficClass {
    const fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::Membership,
            2 => Self::ControlPlane,
            3 => Self::Dlm,
            4 => Self::Dsm,
            5 => Self::Ipc,
            6 => Self::Telemetry,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecureFrame {
    pub class: TrafficClass,
    pub epoch: u64,
    pub sequence: u64,
    pub payload_len: u16,
    pub ciphertext: [u8; MAX_SECURE_PAYLOAD],
    pub tag: [u8; SECURE_FRAME_TAG_BYTES],
}

impl SecureFrame {
    pub fn encode(&self, output: &mut [u8]) -> Result<(), SecurityError> {
        if output.len() < SECURE_FRAME_WIRE_BYTES || self.payload_len as usize > MAX_SECURE_PAYLOAD {
            return Err(SecurityError::BufferTooSmall)
        }
        output[..SECURE_FRAME_WIRE_BYTES].fill(0);
        output[..4].copy_from_slice(b"SYNS");
        output[4] = 1;
        output[5] = self.class as u8;
        output[8..16].copy_from_slice(&self.epoch.to_be_bytes());
        output[16..24].copy_from_slice(&self.sequence.to_be_bytes());
        output[24..26].copy_from_slice(&self.payload_len.to_be_bytes());
        output[SECURE_FRAME_HEADER_BYTES..SECURE_FRAME_HEADER_BYTES + MAX_SECURE_PAYLOAD]
            .copy_from_slice(&self.ciphertext);
        output[SECURE_FRAME_HEADER_BYTES + MAX_SECURE_PAYLOAD..SECURE_FRAME_WIRE_BYTES]
            .copy_from_slice(&self.tag);
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<Self, SecurityError> {
        if input.len() < SECURE_FRAME_WIRE_BYTES
            || &input[..4] != b"SYNS"
            || input[4] != 1
            || u16::from_be_bytes([input[24], input[25]]) as usize > MAX_SECURE_PAYLOAD
        {
            return Err(SecurityError::InvalidFrame)
        }
        let class = TrafficClass::from_raw(input[5]).ok_or(SecurityError::InvalidFrame)?;
        let mut ciphertext = [0; MAX_SECURE_PAYLOAD];
        ciphertext.copy_from_slice(
            &input[SECURE_FRAME_HEADER_BYTES..SECURE_FRAME_HEADER_BYTES + MAX_SECURE_PAYLOAD],
        );
        let mut tag = [0; SECURE_FRAME_TAG_BYTES];
        tag.copy_from_slice(&input[SECURE_FRAME_HEADER_BYTES + MAX_SECURE_PAYLOAD..SECURE_FRAME_WIRE_BYTES]);
        Ok(Self {
            class,
            epoch: u64::from_be_bytes(input[8..16].try_into().map_err(|_| SecurityError::InvalidFrame)?),
            sequence: u64::from_be_bytes(input[16..24].try_into().map_err(|_| SecurityError::InvalidFrame)?),
            payload_len: u16::from_be_bytes([input[24], input[25]]),
            ciphertext,
            tag,
        })
    }

    fn header(&self) -> [u8; SECURE_FRAME_HEADER_BYTES] {
        let mut header = [0; SECURE_FRAME_HEADER_BYTES];
        header[..4].copy_from_slice(b"SYNS");
        header[4] = 1;
        header[5] = self.class as u8;
        header[8..16].copy_from_slice(&self.epoch.to_be_bytes());
        header[16..24].copy_from_slice(&self.sequence.to_be_bytes());
        header[24..26].copy_from_slice(&self.payload_len.to_be_bytes());
        header
    }
}

#[derive(Clone, Copy)]
pub struct SecureChannel {
    send_key: CapabilityKey,
    receive_key: CapabilityKey,
    epoch: u64,
    next_send: u64,
    last_received: u64,
}

impl SecureChannel {
    pub fn new(
        shared_key: CapabilityKey,
        local: NodeId,
        remote: NodeId,
        initiator: bool,
        epoch: u64,
    ) -> Result<Self, SecurityError> {
        if epoch == 0 {
            return Err(SecurityError::InvalidKey)
        }
        let low = local.raw().min(remote.raw());
        let high = local.raw().max(remote.raw());
        let mut base = [0; 32];
        base[..8].copy_from_slice(b"SYNCH001");
        base[8..12].copy_from_slice(&low.to_be_bytes());
        base[12..16].copy_from_slice(&high.to_be_bytes());
        base[16..24].copy_from_slice(&epoch.to_be_bytes());
        let mut send_material = base;
        let mut receive_material = base;
        send_material[24] = if initiator { 0 } else { 1 };
        receive_material[24] = if initiator { 1 } else { 0 };
        let send_key = CapabilityKey::new(
            shared_key
                .authenticate(&send_material)
                .map_err(|_| SecurityError::InvalidKey)?,
        );
        let receive_key = CapabilityKey::new(
            shared_key
                .authenticate(&receive_material)
                .map_err(|_| SecurityError::InvalidKey)?,
        );
        Ok(Self {
            send_key,
            receive_key,
            epoch,
            next_send: 1,
            last_received: 0,
        })
    }

    pub fn seal(&mut self, class: TrafficClass, payload: &[u8]) -> Result<SecureFrame, SecurityError> {
        if payload.len() > MAX_SECURE_PAYLOAD || self.next_send == 0 {
            return Err(SecurityError::BufferTooSmall)
        }
        let mut frame = SecureFrame {
            class,
            epoch: self.epoch,
            sequence: self.next_send,
            payload_len: payload.len() as u16,
            ciphertext: [0; MAX_SECURE_PAYLOAD],
            tag: [0; SECURE_FRAME_TAG_BYTES],
        };
        xor_payload(
            self.send_key,
            frame.sequence,
            payload,
            &mut frame.ciphertext[..payload.len()],
        )?;
        frame.tag = tag(self.send_key, &frame)?;
        self.next_send = self.next_send.checked_add(1).ok_or(SecurityError::SequenceExhausted)?;
        Ok(frame)
    }

    pub fn open(&mut self, frame: SecureFrame, output: &mut [u8]) -> Result<usize, SecurityError> {
        let length = frame.payload_len as usize;
        if frame.epoch != self.epoch
            || length > MAX_SECURE_PAYLOAD
            || output.len() < length
            || frame.sequence <= self.last_received
            || !constant_time_equal(&tag(self.receive_key, &frame)?, &frame.tag)
        {
            return Err(SecurityError::InvalidFrame)
        }
        xor_payload(
            self.receive_key,
            frame.sequence,
            &frame.ciphertext[..length],
            &mut output[..length],
        )?;
        self.last_received = frame.sequence;
        Ok(length)
    }
}

fn xor_payload(
    key: CapabilityKey,
    sequence: u64,
    input: &[u8],
    output: &mut [u8],
) -> Result<(), SecurityError> {
    for (block, chunk) in input.chunks(32).enumerate() {
        let mut material = [0; 24];
        material[..8].copy_from_slice(b"SYNCRYPT");
        material[8..16].copy_from_slice(&sequence.to_be_bytes());
        material[16..24].copy_from_slice(&(block as u64).to_be_bytes());
        let stream = key
            .authenticate(&material)
            .map_err(|_| SecurityError::InvalidKey)?;
        for (index, byte) in chunk.iter().enumerate() {
            output[block * 32 + index] = *byte ^ stream[index];
        }
    }
    Ok(())
}

fn tag(key: CapabilityKey, frame: &SecureFrame) -> Result<[u8; SECURE_FRAME_TAG_BYTES], SecurityError> {
    let mut material = [0; SECURE_FRAME_HEADER_BYTES + MAX_SECURE_PAYLOAD];
    material[..SECURE_FRAME_HEADER_BYTES].copy_from_slice(&frame.header());
    material[SECURE_FRAME_HEADER_BYTES..SECURE_FRAME_HEADER_BYTES + frame.payload_len as usize]
        .copy_from_slice(&frame.ciphertext[..frame.payload_len as usize]);
    key.authenticate(&material[..SECURE_FRAME_HEADER_BYTES + frame.payload_len as usize])
        .map_err(|_| SecurityError::InvalidKey)
}

fn node_bytes(node: NodeId) -> [u8; 32] {
    u64_bytes(node.raw() as u64)
}

fn u64_bytes(value: u64) -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes[..8].copy_from_slice(&value.to_be_bytes());
    bytes
}

fn constant_time_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityError {
    AccessDenied,
    AlreadyRevoked,
    AttestationRejected,
    AuditCapacity,
    BufferTooSmall,
    CannotRevokeCurrentKey,
    CaveatCapacity,
    CertificateRequired,
    InvalidCapability,
    InvalidCertificate,
    InvalidFrame,
    InvalidKey,
    InvalidSignature,
    KeyCapacity,
    KeyNotFound,
    MissingNodeCapability,
    RevocationCapacity,
    SequenceExhausted,
    TrustRootCapacity,
    TrustRootNotFound,
    InvalidTrustRoot,
    Interrupted,
}

impl IntoStatus for SecurityError {
    fn status(self) -> Status {
        match self {
            Self::AccessDenied
            | Self::AttestationRejected
            | Self::CertificateRequired
            | Self::InvalidSignature
            | Self::MissingNodeCapability
            | Self::TrustRootNotFound => Status::ACCESS_DENIED,
            Self::AlreadyRevoked | Self::CannotRevokeCurrentKey => Status::CONFLICT,
            Self::AuditCapacity
            | Self::CaveatCapacity
            | Self::KeyCapacity
            | Self::RevocationCapacity
            | Self::TrustRootCapacity => Status::NO_SPACE,
            Self::BufferTooSmall => Status::NO_SPACE,
            Self::InvalidCapability
            | Self::InvalidCertificate
            | Self::InvalidFrame
            | Self::InvalidKey
            | Self::KeyNotFound
            | Self::SequenceExhausted
            | Self::InvalidTrustRoot => Status::INVALID_ARGUMENT,
            Self::Interrupted => Status::BUSY,
        }
    }
}
