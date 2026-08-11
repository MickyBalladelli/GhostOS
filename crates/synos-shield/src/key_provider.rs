//! Opaque, hardware-backed key-provider boundary.
//!
//! The authority in this module owns policy and metadata only. A provider owns
//! private key material in a hardware device or isolated service and exposes
//! only bounded operations over opaque [`KeyHandle`] values. In particular,
//! this API has no import, export, debug, snapshot, or crash-capsule path for
//! private key bytes.

use synos_status::{IntoStatus, Status};

use crate::Error;

pub const KEY_ID_BYTES: usize = 16;
pub const PROVIDER_ID_BYTES: usize = 16;
pub const DEFAULT_APPROVER_CAPACITY: usize = 8;
pub const DEFAULT_KEY_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderId([u8; PROVIDER_ID_BYTES]);

impl ProviderId {
    pub const fn new(bytes: [u8; PROVIDER_ID_BYTES]) -> Result<Self, KeyProviderError> {
        if all_zero(&bytes) {
            Err(KeyProviderError::InvalidIdentifier)
        } else {
            Ok(Self(bytes))
        }
    }

    pub const fn from_bytes(bytes: [u8; PROVIDER_ID_BYTES]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; PROVIDER_ID_BYTES] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyId([u8; KEY_ID_BYTES]);

impl KeyId {
    pub const fn new(bytes: [u8; KEY_ID_BYTES]) -> Result<Self, KeyProviderError> {
        if all_zero(&bytes) {
            Err(KeyProviderError::InvalidIdentifier)
        } else {
            Ok(Self(bytes))
        }
    }

    pub const fn from_bytes(bytes: [u8; KEY_ID_BYTES]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; KEY_ID_BYTES] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyHandle {
    provider: ProviderId,
    key: KeyId,
    generation: u64,
}

impl KeyHandle {
    pub const fn new(provider: ProviderId, key: KeyId, generation: u64) -> Result<Self, KeyProviderError> {
        if generation == 0 {
            Err(KeyProviderError::InvalidGeneration)
        } else {
            Ok(Self { provider, key, generation })
        }
    }

    pub const fn provider(self) -> ProviderId {
        self.provider
    }

    pub const fn key_id(self) -> KeyId {
        self.key
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Isolation {
    HardwareBacked,
    IsolatedService,
    ProcessMemory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyPurpose {
    Attestation,
    PackageSigning,
    ClusterTransport,
    BackupEncryption,
    Recovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuorumPolicy {
    threshold: u8,
    members: u8,
}

impl QuorumPolicy {
    pub const fn new(threshold: u8, members: u8) -> Result<Self, KeyProviderError> {
        if threshold == 0 || members == 0 || threshold > members {
            Err(KeyProviderError::InvalidQuorum)
        } else {
            Ok(Self { threshold, members })
        }
    }

    pub const fn threshold(self) -> u8 {
        self.threshold
    }

    pub const fn members(self) -> u8 {
        self.members
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApproverId(u64);

impl ApproverId {
    pub const fn new(raw: u64) -> Result<Self, KeyProviderError> {
        if raw == 0 {
            Err(KeyProviderError::InvalidIdentifier)
        } else {
            Ok(Self(raw))
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// A provider verifies this proof inside its trusted boundary. It is an
/// approval signature or equivalent public authorization, never a private key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApprovalProof([u8; 32]);

impl ApprovalProof {
    pub const fn new(bytes: [u8; 32]) -> Result<Self, KeyProviderError> {
        if all_zero(&bytes) {
            Err(KeyProviderError::InvalidApproval)
        } else {
            Ok(Self(bytes))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalOperation {
    Rotation,
    Recovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApprovalScope {
    pub operation: ApprovalOperation,
    pub request_id: u64,
    pub key: KeyHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryAuthorization {
    pub ceremony_id: u64,
    pub key: KeyHandle,
    pub approvals: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OfflineRevocation {
    pub command_id: u64,
    pub key: KeyHandle,
    pub epoch: u64,
    pub authorization: ApprovalProof,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyState {
    Active,
    Recovery,
    Revoked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyMetadata {
    pub handle: KeyHandle,
    pub purpose: KeyPurpose,
    pub state: KeyState,
    pub quorum: QuorumPolicy,
    pub revocation_epoch: u64,
}

/// The provider contract deliberately contains no secret-bearing field.
/// Implementations must keep private material in the hardware or isolated
/// service and must not copy it into this process, logs, snapshots, or crash
/// capsules.
pub trait KeyProvider {
    fn provider_id(&self) -> ProviderId;

    fn isolation(&self) -> Isolation;

    /// Generate a key in the provider and return only its opaque handle.
    fn generate_key(
        &mut self,
        purpose: KeyPurpose,
        generation: u64,
    ) -> Result<KeyHandle, KeyProviderError>;

    /// Sign inside the provider. The output is a public signature, not key material.
    fn sign(
        &mut self,
        key: KeyHandle,
        message: &[u8],
        signature: &mut [u8],
    ) -> Result<usize, KeyProviderError>;

    fn verify_approval(
        &self,
        scope: ApprovalScope,
        approver: ApproverId,
        proof: ApprovalProof,
    ) -> Result<(), KeyProviderError>;

    fn commit_rotation(
        &mut self,
        old_key: KeyHandle,
        new_key: KeyHandle,
    ) -> Result<(), KeyProviderError>;

    /// Complete recovery in the provider. Shares, if any, stay inside the
    /// ceremony boundary and are never passed to the authority.
    fn complete_recovery(
        &mut self,
        authorization: RecoveryAuthorization,
    ) -> Result<(), KeyProviderError>;

    /// Validate and execute an offline emergency command in the provider.
    fn emergency_revoke(&mut self, command: OfflineRevocation) -> Result<(), KeyProviderError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyProviderError {
    Capacity,
    InvalidApproval,
    InvalidGeneration,
    InvalidIdentifier,
    InvalidQuorum,
    InvalidRequest,
    NotFound,
    ProviderRejected,
    SignatureBufferTooSmall,
    Unauthorized,
}

impl IntoStatus for KeyProviderError {
    fn status(self) -> Status {
        match self {
            Self::Capacity | Self::SignatureBufferTooSmall => Status::NO_SPACE,
            Self::InvalidApproval
            | Self::InvalidGeneration
            | Self::InvalidIdentifier
            | Self::InvalidQuorum
            | Self::InvalidRequest => Status::INVALID_ARGUMENT,
            Self::NotFound => Status::NOT_FOUND,
            Self::ProviderRejected | Self::Unauthorized => Status::ACCESS_DENIED,
        }
    }
}

#[derive(Clone, Copy)]
struct ApprovalSet<const APPROVERS: usize> {
    ids: [Option<ApproverId>; APPROVERS],
    count: u8,
}

impl<const APPROVERS: usize> ApprovalSet<APPROVERS> {
    const fn new() -> Self {
        Self { ids: [None; APPROVERS], count: 0 }
    }

    fn add(&mut self, approver: ApproverId) -> Result<(), KeyProviderError> {
        if self.ids.iter().flatten().any(|entry| *entry == approver) {
            return Err(KeyProviderError::InvalidApproval)
        }
        let slot = self
            .ids
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(KeyProviderError::Capacity)?;
        *slot = Some(approver);
        self.count = self.count.saturating_add(1);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct RotationRequest<const APPROVERS: usize> {
    request_id: u64,
    old_key: KeyHandle,
    new_key: KeyHandle,
    quorum: QuorumPolicy,
    expires_at_us: u64,
    approvals: ApprovalSet<APPROVERS>,
}

#[derive(Clone, Copy)]
struct RecoveryRequest<const APPROVERS: usize> {
    ceremony_id: u64,
    key: KeyHandle,
    quorum: QuorumPolicy,
    expires_at_us: u64,
    approvals: ApprovalSet<APPROVERS>,
}

/// Fixed-capacity policy gate for an isolated provider.
pub struct KeyAuthority<
    P,
    const KEYS: usize = DEFAULT_KEY_CAPACITY,
    const APPROVERS: usize = DEFAULT_APPROVER_CAPACITY,
> {
    provider: P,
    keys: [Option<KeyMetadata>; KEYS],
    rotations: [Option<RotationRequest<APPROVERS>>; KEYS],
    recoveries: [Option<RecoveryRequest<APPROVERS>>; KEYS],
}

impl<P, const KEYS: usize, const APPROVERS: usize> KeyAuthority<P, KEYS, APPROVERS>
where
    P: KeyProvider,
{
    pub fn new(provider: P) -> Result<Self, KeyProviderError> {
        if KEYS == 0 || APPROVERS == 0 {
            return Err(KeyProviderError::InvalidRequest)
        }
        if provider.isolation() == Isolation::ProcessMemory {
            return Err(KeyProviderError::ProviderRejected)
        }
        Ok(Self {
            provider,
            keys: [None; KEYS],
            rotations: [None; KEYS],
            recoveries: [None; KEYS],
        })
    }

    pub fn provider(&self) -> &P {
        &self.provider
    }

    pub fn provider_mut(&mut self) -> &mut P {
        &mut self.provider
    }

    pub fn register(
        &mut self,
        handle: KeyHandle,
        purpose: KeyPurpose,
        quorum: QuorumPolicy,
    ) -> Result<KeyMetadata, KeyProviderError> {
        if handle.provider() != self.provider.provider_id() {
            return Err(KeyProviderError::ProviderRejected)
        }
        if self.keys.iter().flatten().any(|entry| entry.handle == handle) {
            return Err(KeyProviderError::InvalidRequest)
        }
        let metadata = KeyMetadata {
            handle,
            purpose,
            state: KeyState::Active,
            quorum,
            revocation_epoch: 0,
        };
        let slot = self
            .keys
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(KeyProviderError::Capacity)?;
        *slot = Some(metadata);
        Ok(metadata)
    }

    pub fn generate_and_register(
        &mut self,
        purpose: KeyPurpose,
        quorum: QuorumPolicy,
    ) -> Result<KeyMetadata, KeyProviderError> {
        let handle = self.provider.generate_key(purpose, 1)?;
        self.register(handle, purpose, quorum)
    }

    pub fn sign(
        &mut self,
        handle: KeyHandle,
        message: &[u8],
        signature: &mut [u8],
    ) -> Result<usize, KeyProviderError> {
        let metadata = self.metadata(handle).ok_or(KeyProviderError::NotFound)?;
        if metadata.state != KeyState::Active {
            return Err(KeyProviderError::Unauthorized)
        }
        self.provider.sign(handle, message, signature)
    }

    pub fn begin_rotation(
        &mut self,
        request_id: u64,
        old_key: KeyHandle,
        new_key: KeyHandle,
        expires_at_us: u64,
    ) -> Result<(), KeyProviderError> {
        if request_id == 0 || expires_at_us == 0 || old_key == new_key {
            return Err(KeyProviderError::InvalidRequest)
        }
        if new_key.provider() != self.provider.provider_id()
            || new_key.generation() != old_key.generation().saturating_add(1)
        {
            return Err(KeyProviderError::InvalidGeneration)
        }
        let metadata = self.metadata(old_key).ok_or(KeyProviderError::NotFound)?;
        if metadata.state != KeyState::Active
            || self.rotation(request_id).is_some()
            || self
                .rotations
                .iter()
                .flatten()
                .any(|entry| entry.old_key == old_key)
        {
            return Err(KeyProviderError::InvalidRequest)
        }
        let slot = self
            .rotations
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(KeyProviderError::Capacity)?;
        *slot = Some(RotationRequest {
            request_id,
            old_key,
            new_key,
            quorum: metadata.quorum,
            expires_at_us,
            approvals: ApprovalSet::new(),
        });
        Ok(())
    }

    pub fn approve_rotation(
        &mut self,
        request_id: u64,
        approver: ApproverId,
        proof: ApprovalProof,
        now_us: u64,
    ) -> Result<u8, KeyProviderError> {
        let request = self
            .rotations
            .iter_mut()
            .flatten()
            .find(|entry| entry.request_id == request_id)
            .ok_or(KeyProviderError::NotFound)?;
        if now_us > request.expires_at_us {
            return Err(KeyProviderError::InvalidRequest)
        }
        if request.approvals.count >= request.quorum.members() {
            return Err(KeyProviderError::InvalidApproval)
        }
        self.provider.verify_approval(
            ApprovalScope {
                operation: ApprovalOperation::Rotation,
                request_id,
                key: request.old_key,
            },
            approver,
            proof,
        )?;
        request.approvals.add(approver)?;
        Ok(request.approvals.count)
    }

    pub fn commit_rotation(
        &mut self,
        request_id: u64,
        now_us: u64,
    ) -> Result<KeyMetadata, KeyProviderError> {
        let index = self
            .rotations
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.request_id == request_id))
            .ok_or(KeyProviderError::NotFound)?;
        let request = self.rotations[index].ok_or(KeyProviderError::NotFound)?;
        if now_us > request.expires_at_us {
            return Err(KeyProviderError::InvalidRequest)
        }
        if request.approvals.count < request.quorum.threshold() {
            return Err(KeyProviderError::Unauthorized)
        }
        self.provider.commit_rotation(request.old_key, request.new_key)?;
        let key = self
            .keys
            .iter_mut()
            .flatten()
            .find(|entry| entry.handle == request.old_key)
            .ok_or(KeyProviderError::NotFound)?;
        key.handle = request.new_key;
        key.state = KeyState::Active;
        self.rotations[index] = None;
        Ok(*key)
    }

    pub fn begin_recovery(
        &mut self,
        ceremony_id: u64,
        key: KeyHandle,
        expires_at_us: u64,
    ) -> Result<(), KeyProviderError> {
        if ceremony_id == 0 || expires_at_us == 0 {
            return Err(KeyProviderError::InvalidRequest)
        }
        let metadata = self.metadata(key).ok_or(KeyProviderError::NotFound)?;
        if metadata.state == KeyState::Revoked
            || self.recovery(ceremony_id).is_some()
            || self
                .recoveries
                .iter()
                .flatten()
                .any(|entry| entry.key == key)
        {
            return Err(KeyProviderError::InvalidRequest)
        }
        let slot = self
            .recoveries
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(KeyProviderError::Capacity)?;
        *slot = Some(RecoveryRequest {
            ceremony_id,
            key,
            quorum: metadata.quorum,
            expires_at_us,
            approvals: ApprovalSet::new(),
        });
        let key = self
            .keys
            .iter_mut()
            .flatten()
            .find(|entry| entry.handle == key)
            .ok_or(KeyProviderError::NotFound)?;
        key.state = KeyState::Recovery;
        Ok(())
    }

    pub fn approve_recovery(
        &mut self,
        ceremony_id: u64,
        approver: ApproverId,
        proof: ApprovalProof,
        now_us: u64,
    ) -> Result<u8, KeyProviderError> {
        let request = self
            .recoveries
            .iter_mut()
            .flatten()
            .find(|entry| entry.ceremony_id == ceremony_id)
            .ok_or(KeyProviderError::NotFound)?;
        if now_us > request.expires_at_us {
            return Err(KeyProviderError::InvalidRequest)
        }
        if request.approvals.count >= request.quorum.members() {
            return Err(KeyProviderError::InvalidApproval)
        }
        self.provider.verify_approval(
            ApprovalScope {
                operation: ApprovalOperation::Recovery,
                request_id: ceremony_id,
                key: request.key,
            },
            approver,
            proof,
        )?;
        request.approvals.add(approver)?;
        Ok(request.approvals.count)
    }

    pub fn complete_recovery(
        &mut self,
        ceremony_id: u64,
        now_us: u64,
    ) -> Result<KeyMetadata, KeyProviderError> {
        let index = self
            .recoveries
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.ceremony_id == ceremony_id))
            .ok_or(KeyProviderError::NotFound)?;
        let request = self.recoveries[index].ok_or(KeyProviderError::NotFound)?;
        if now_us > request.expires_at_us {
            return Err(KeyProviderError::InvalidRequest)
        }
        if request.approvals.count < request.quorum.threshold() {
            return Err(KeyProviderError::Unauthorized)
        }
        self.provider.complete_recovery(RecoveryAuthorization {
            ceremony_id,
            key: request.key,
            approvals: request.approvals.count,
        })?;
        let key = self
            .keys
            .iter_mut()
            .flatten()
            .find(|entry| entry.handle == request.key)
            .ok_or(KeyProviderError::NotFound)?;
        key.state = KeyState::Active;
        self.recoveries[index] = None;
        Ok(*key)
    }

    pub fn emergency_revoke(
        &mut self,
        command: OfflineRevocation,
    ) -> Result<KeyMetadata, KeyProviderError> {
        if command.command_id == 0 || command.epoch == 0 {
            return Err(KeyProviderError::InvalidRequest)
        }
        let key = self
            .keys
            .iter_mut()
            .flatten()
            .find(|entry| entry.handle == command.key)
            .ok_or(KeyProviderError::NotFound)?;
        if command.epoch <= key.revocation_epoch || key.state == KeyState::Revoked {
            return Err(KeyProviderError::InvalidRequest)
        }
        self.provider.emergency_revoke(command)?;
        for request in &mut self.rotations {
            if request.is_some_and(|request| {
                request.old_key == command.key || request.new_key == command.key
            }) {
                *request = None;
            }
        }
        for request in &mut self.recoveries {
            if request.is_some_and(|request| request.key == command.key) {
                *request = None;
            }
        }
        key.revocation_epoch = command.epoch;
        key.state = KeyState::Revoked;
        Ok(*key)
    }

    /// Copy only public metadata. This is the sole inventory snapshot API.
    pub fn snapshot(&self, destination: &mut [KeyMetadata]) -> usize {
        let mut written = 0;
        for metadata in self.keys.iter().flatten() {
            if written == destination.len() {
                break
            }
            destination[written] = *metadata;
            written += 1;
        }
        written
    }

    pub fn metadata(&self, handle: KeyHandle) -> Option<KeyMetadata> {
        self.keys
            .iter()
            .flatten()
            .find(|entry| entry.handle == handle)
            .copied()
    }

    fn rotation(&self, request_id: u64) -> Option<RotationRequest<APPROVERS>> {
        self.rotations
            .iter()
            .flatten()
            .find(|entry| entry.request_id == request_id)
            .copied()
    }

    fn recovery(&self, ceremony_id: u64) -> Option<RecoveryRequest<APPROVERS>> {
        self.recoveries
            .iter()
            .flatten()
            .find(|entry| entry.ceremony_id == ceremony_id)
            .copied()
    }
}

const fn all_zero<const N: usize>(bytes: &[u8; N]) -> bool {
    let mut index = 0;
    while index < N {
        if bytes[index] != 0 {
            return false
        }
        index += 1;
    }
    true
}

impl From<KeyProviderError> for Error {
    fn from(error: KeyProviderError) -> Self {
        match error {
            KeyProviderError::Capacity | KeyProviderError::SignatureBufferTooSmall => Error::Capacity,
            KeyProviderError::NotFound => Error::NotFound,
            KeyProviderError::ProviderRejected | KeyProviderError::Unauthorized => Error::Unauthorized,
            KeyProviderError::InvalidApproval
            | KeyProviderError::InvalidGeneration
            | KeyProviderError::InvalidIdentifier
            | KeyProviderError::InvalidQuorum
            | KeyProviderError::InvalidRequest => Error::InvalidInput,
        }
    }
}
