use core::fmt;

use synos_fabric::NodeId;
use synos_kernel::{
    AddressSpaceId, CapabilityHandle, CapabilityObject, CapabilitySpace, ExecutionPersona,
    IdentityId, RightIdentifier, Rights,
};
use synos_observability::{EventField, Level, audit_event, field};

pub const MAX_USERNAME_BYTES: usize = 32;
pub const MAX_CREDENTIAL_BYTES: usize = 96;
pub const MAX_CREDENTIALS_PER_USER: usize = 4;
pub const MAX_INITIAL_CAPABILITIES: usize = 8;
pub const MAX_USER_RIGHTS: usize = 16;
pub const DEFAULT_USER_CAPACITY: usize = 64;
pub const DEFAULT_CHALLENGE_CAPACITY: usize = 16;
pub const RESERVED_USERNAMES: &[&str] = &[
    ".",
    "..",
    "account",
    "anonymous",
    "daemon",
    "guest",
    "kernel",
    "nobody",
    "operator",
    "root",
    "service",
    "system",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Username {
    bytes: [u8; MAX_USERNAME_BYTES],
    length: u8,
}

impl Username {
    pub fn new(username: &str) -> Result<Self, AuthError> {
        let source = username.as_bytes();
        if source.is_empty()
            || source.len() > MAX_USERNAME_BYTES
            || source.contains(&0)
            || !source
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-$".contains(byte))
        {
            return Err(AuthError::InvalidRecord)
        }
        let mut bytes = [0; MAX_USERNAME_BYTES];
        for (slot, byte) in bytes.iter_mut().zip(source.iter().copied()) {
            *slot = byte.to_ascii_lowercase();
        }
        if RESERVED_USERNAMES.contains(&core::str::from_utf8(&bytes[..source.len()]).unwrap()) {
            return Err(AuthError::InvalidRecord)
        }
        Ok(Self {
            bytes,
            length: source.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.length as usize]).expect("Username invariant")
    }

    fn matches(&self, other: &str) -> bool {
        self.as_str().eq_ignore_ascii_case(other)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatabaseScope {
    Local,
    NodeLocal(NodeId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CredentialKind {
    Passkey = 1,
    Tpm20 = 2,
    SshKey = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CredentialId(u32);

impl CredentialId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Credential {
    pub id: CredentialId,
    pub kind: CredentialKind,
    public_material: [u8; MAX_CREDENTIAL_BYTES],
    public_material_length: u8,
    sign_count: u32,
}

impl fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credential")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("sign_count", &self.sign_count)
            .finish()
    }
}

impl Credential {
    pub fn new(
        id: CredentialId,
        kind: CredentialKind,
        public_material: &[u8],
    ) -> Result<Self, AuthError> {
        if public_material.is_empty() || public_material.len() > MAX_CREDENTIAL_BYTES {
            return Err(AuthError::InvalidRecord)
        }
        let mut stored = [0; MAX_CREDENTIAL_BYTES];
        stored[..public_material.len()].copy_from_slice(public_material);
        Ok(Self {
            id,
            kind,
            public_material: stored,
            public_material_length: public_material.len() as u8,
            sign_count: 0,
        })
    }

    pub fn new_passkey(
        id: CredentialId,
        cose_public_key: &[u8],
        sign_count: u32,
    ) -> Result<Self, AuthError> {
        let mut credential = Self::new(id, CredentialKind::Passkey, cose_public_key)?;
        credential.sign_count = sign_count;
        Ok(credential)
    }

    pub fn new_tpm20(
        id: CredentialId,
        public_material: &[u8],
    ) -> Result<Self, AuthError> {
        Self::new(id, CredentialKind::Tpm20, public_material)
    }

    pub fn new_ssh_key(
        id: CredentialId,
        public_key: &[u8],
    ) -> Result<Self, AuthError> {
        Self::new(id, CredentialKind::SshKey, public_key)
    }

    pub fn public_material(&self) -> &[u8] {
        &self.public_material[..self.public_material_length as usize]
    }

    pub const fn kind(&self) -> CredentialKind {
        self.kind
    }

    pub const fn sign_count(&self) -> u32 {
        self.sign_count
    }
}

pub trait CredentialVerifier {
    /// Verify passkey, TPM 2.0, or SSH proof with the platform crypto service.
    fn verify(
        &mut self,
        kind: CredentialKind,
        public_material: &[u8],
        challenge: &[u8],
        response: &[u8],
    ) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitialCapability {
    pub object: CapabilityObject,
    pub rights: Rights,
}

#[derive(Clone, Copy)]
pub struct UserRecord {
    pub identity: IdentityId,
    pub username: Username,
    pub scope: DatabaseScope,
    pub enabled: bool,
    credentials: [Option<Credential>; MAX_CREDENTIALS_PER_USER],
    rights: [Option<RightIdentifier>; MAX_USER_RIGHTS],
    initial_capabilities: [Option<InitialCapability>; MAX_INITIAL_CAPABILITIES],
}

impl fmt::Debug for UserRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserRecord")
            .field("identity", &self.identity)
            .field("username", &self.username)
            .field("scope", &self.scope)
            .field("enabled", &self.enabled)
            .field("credential_count", &self.credentials().count())
            .field("right_count", &self.rights().count())
            .field("capability_count", &self.initial_capabilities().count())
            .finish()
    }
}

impl UserRecord {
    pub const fn new(identity: IdentityId, username: Username, scope: DatabaseScope) -> Self {
        Self {
            identity,
            username,
            scope,
            enabled: true,
            credentials: [None; MAX_CREDENTIALS_PER_USER],
            rights: [None; MAX_USER_RIGHTS],
            initial_capabilities: [None; MAX_INITIAL_CAPABILITIES],
        }
    }

    pub fn credentials(&self) -> impl Iterator<Item = Credential> + '_ {
        self.credentials.iter().flatten().copied()
    }

    pub fn credential_kind(&self, id: CredentialId) -> Result<CredentialKind, AuthError> {
        self.credential(id)
            .map(|credential| credential.kind())
            .ok_or(AuthError::CredentialNotFound)
    }

    pub fn rights(&self) -> impl Iterator<Item = RightIdentifier> + '_ {
        self.rights.iter().flatten().copied()
    }

    pub fn initial_capabilities(&self) -> impl Iterator<Item = InitialCapability> + '_ {
        self.initial_capabilities.iter().flatten().copied()
    }

    pub fn add_credential(&mut self, credential: Credential) -> Result<(), AuthError> {
        if self
            .credentials()
            .any(|existing| existing.id == credential.id)
        {
            return Err(AuthError::AlreadyExists)
        }
        let slot = self
            .credentials
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(AuthError::Capacity)?;
        *slot = Some(credential);
        Ok(())
    }

    pub fn assign_right(&mut self, right: RightIdentifier) -> Result<(), AuthError> {
        if self.rights().any(|existing| existing == right) {
            return Ok(())
        }
        let slot = self
            .rights
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(AuthError::Capacity)?;
        *slot = Some(right);
        Ok(())
    }

    pub fn grant_initial_capability(
        &mut self,
        capability: InitialCapability,
    ) -> Result<(), AuthError> {
        if capability.rights.is_empty() {
            return Err(AuthError::InvalidRecord)
        }
        let slot = self
            .initial_capabilities
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(AuthError::Capacity)?;
        *slot = Some(capability);
        Ok(())
    }

    fn credential(&self, id: CredentialId) -> Option<Credential> {
        self.credentials().find(|credential| credential.id == id)
    }

    pub fn ssh_key_credential(&self, public_key: &[u8]) -> Option<CredentialId> {
        self.credentials()
            .find(|credential| {
                credential.kind == CredentialKind::SshKey
                    && credential.public_material() == public_key
            })
            .map(|credential| credential.id)
    }

    pub fn record_passkey_use(
        &mut self,
        id: CredentialId,
        sign_count: u32,
    ) -> Result<(), AuthError> {
        let credential = self
            .credentials
            .iter_mut()
            .flatten()
            .find(|credential| credential.id == id)
            .ok_or(AuthError::CredentialNotFound)?;
        if credential.kind != CredentialKind::Passkey {
            return Err(AuthError::InvalidRecord)
        }
        if credential.sign_count != 0
            && sign_count != 0
            && sign_count <= credential.sign_count
        {
            return Err(AuthError::VerificationFailed)
        }
        if sign_count != 0 {
            credential.sign_count = sign_count
        }
        Ok(())
    }

    pub fn passkey_sign_count(&self, id: CredentialId) -> Result<u32, AuthError> {
        let credential = self
            .credential(id)
            .ok_or(AuthError::CredentialNotFound)?;
        if credential.kind != CredentialKind::Passkey {
            return Err(AuthError::InvalidRecord)
        }
        Ok(credential.sign_count)
    }
}

/// Storage boundary for SYSUAF.DAT-style local authorization records.
pub trait AuthorizationStore {
    fn load(&mut self, scope: DatabaseScope, username: &str) -> Option<UserRecord>;
    fn store(&mut self, record: &UserRecord) -> Result<(), AuthError>;
}

#[derive(Clone, Copy)]
pub struct AuthorizationDatabase<const CAPACITY: usize = DEFAULT_USER_CAPACITY> {
    records: [Option<UserRecord>; CAPACITY],
}

impl<const CAPACITY: usize> AuthorizationDatabase<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            records: [None; CAPACITY],
        }
    }

    pub fn insert(&mut self, record: UserRecord) -> Result<(), AuthError> {
        if self.records.iter().flatten().any(|existing| {
            existing.identity == record.identity
                || (existing.scope == record.scope
                    && existing
                        .username
                        .as_str()
                        .eq_ignore_ascii_case(record.username.as_str()))
        }) {
            return Err(AuthError::AlreadyExists)
        }
        let slot = self
            .records
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(AuthError::Capacity)?;
        *slot = Some(record);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.records.iter().flatten().count()
    }

    pub const fn is_empty(&self) -> bool {
        let mut index = 0;
        while index < CAPACITY {
            if self.records[index].is_some() {
                return false
            }
            index += 1
        }
        true
    }

    pub fn records(&self) -> impl Iterator<Item = UserRecord> + '_ {
        self.records.iter().flatten().copied()
    }

    pub fn record(&self, identity: IdentityId) -> Result<UserRecord, AuthError> {
        self.records
            .iter()
            .flatten()
            .find(|record| record.identity == identity)
            .copied()
            .ok_or(AuthError::UserNotFound)
    }

    pub fn import<S: AuthorizationStore>(
        &mut self,
        store: &mut S,
        scope: DatabaseScope,
        username: &str,
    ) -> Result<(), AuthError> {
        let record = store
            .load(scope, username)
            .ok_or(AuthError::UserNotFound)?;
        self.insert(record)
    }

    pub fn persist<S: AuthorizationStore>(
        &self,
        store: &mut S,
        identity: IdentityId,
    ) -> Result<(), AuthError> {
        let record = self
            .records
            .iter()
            .flatten()
            .find(|record| record.identity == identity)
            .ok_or(AuthError::UserNotFound)?;
        store.store(record)
    }

    pub fn record_mut(&mut self, identity: IdentityId) -> Result<&mut UserRecord, AuthError> {
        self.records
            .iter_mut()
            .flatten()
            .find(|record| record.identity == identity)
            .ok_or(AuthError::UserNotFound)
    }

    pub fn replace(&mut self, record: UserRecord) -> Result<(), AuthError> {
        let slot = self
            .records
            .iter()
            .position(|entry| entry.is_some_and(|existing| existing.identity == record.identity))
            .ok_or(AuthError::UserNotFound)?;
        if self.records.iter().flatten().any(|existing| {
            existing.identity != record.identity
                && existing.scope == record.scope
                && existing
                    .username
                    .as_str()
                    .eq_ignore_ascii_case(record.username.as_str())
        }) {
            return Err(AuthError::AlreadyExists)
        }
        self.records[slot] = Some(record);
        Ok(())
    }

    pub fn remove(&mut self, identity: IdentityId) -> Result<UserRecord, AuthError> {
        let slot = self
            .records
            .iter()
            .position(|entry| entry.is_some_and(|record| record.identity == identity))
            .ok_or(AuthError::UserNotFound)?;
        self.records[slot].take().ok_or(AuthError::UserNotFound)
    }

    fn login_record(&self, username: &str, node: NodeId) -> Option<UserRecord> {
        self.records
            .iter()
            .flatten()
            .find(|record| {
                record.enabled
                    && record.scope == DatabaseScope::NodeLocal(node)
                    && record.username.matches(username)
            })
            .copied()
            .or_else(|| {
                self.records
                    .iter()
                    .flatten()
                    .find(|record| {
                        record.enabled
                            && record.scope == DatabaseScope::Local
                            && record.username.matches(username)
                    })
                    .copied()
            })
    }

    pub(crate) fn login_record_for_ssh(
        &self,
        username: &str,
        node: NodeId,
        public_key: &[u8],
    ) -> Option<UserRecord> {
        self.records
            .iter()
            .flatten()
            .find(|record| {
                record.enabled
                    && record.scope == DatabaseScope::NodeLocal(node)
                    && record.username.matches(username)
                    && record.ssh_key_credential(public_key).is_some()
            })
            .copied()
            .or_else(|| {
                self.records
                    .iter()
                    .flatten()
                    .find(|record| {
                        record.enabled
                            && record.scope == DatabaseScope::Local
                            && record.username.matches(username)
                            && record.ssh_key_credential(public_key).is_some()
                    })
                    .copied()
            })
    }

    fn identity_record(&self, identity: IdentityId) -> Option<UserRecord> {
        self.records
            .iter()
            .flatten()
            .find(|record| record.enabled && record.identity == identity)
            .copied()
    }
}

impl<const CAPACITY: usize> Default for AuthorizationDatabase<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AuthenticationChallenge {
    pub identity: IdentityId,
    pub credential: CredentialId,
    pub nonce: u64,
    pub expires_at_us: u64,
}

impl fmt::Debug for AuthenticationChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticationChallenge")
            .field("identity", &self.identity)
            .field("credential", &self.credential)
            .field("expires_at_us", &self.expires_at_us)
            .finish()
    }
}

impl AuthenticationChallenge {
    pub fn bytes(self) -> [u8; 28] {
        let mut bytes = [0; 28];
        bytes[0..8].copy_from_slice(&self.identity.raw().to_be_bytes());
        bytes[8..12].copy_from_slice(&self.credential.raw().to_be_bytes());
        bytes[12..20].copy_from_slice(&self.nonce.to_be_bytes());
        bytes[20..28].copy_from_slice(&self.expires_at_us.to_be_bytes());
        bytes
    }
}

#[derive(Clone, Copy)]
struct PendingChallenge {
    challenge: AuthenticationChallenge,
    record: UserRecord,
}

pub struct AuthDaemon<
    const USERS: usize = DEFAULT_USER_CAPACITY,
    const CHALLENGES: usize = DEFAULT_CHALLENGE_CAPACITY,
> {
    database: AuthorizationDatabase<USERS>,
    pending: [Option<PendingChallenge>; CHALLENGES],
    next_nonce: u64,
}

impl<const USERS: usize, const CHALLENGES: usize> AuthDaemon<USERS, CHALLENGES> {
    pub const fn new(database: AuthorizationDatabase<USERS>, boot_nonce: u64) -> Self {
        Self {
            database,
            pending: [None; CHALLENGES],
            next_nonce: boot_nonce,
        }
    }

    pub const fn database(&self) -> &AuthorizationDatabase<USERS> {
        &self.database
    }

    pub fn database_mut(&mut self) -> &mut AuthorizationDatabase<USERS> {
        &mut self.database
    }

    pub fn begin_authentication(
        &mut self,
        username: &str,
        node: NodeId,
        credential: CredentialId,
        now_us: u64,
        lifetime_us: u64,
    ) -> Result<AuthenticationChallenge, AuthError> {
        self.begin_authentication_for_kind(
            username,
            node,
            credential,
            None,
            now_us,
            lifetime_us,
        )
    }

    pub(crate) fn begin_authentication_for_kind(
        &mut self,
        username: &str,
        node: NodeId,
        credential: CredentialId,
        required_kind: Option<CredentialKind>,
        now_us: u64,
        lifetime_us: u64,
    ) -> Result<AuthenticationChallenge, AuthError> {
        if lifetime_us == 0 {
            return Err(AuthError::InvalidChallenge)
        }
        self.pending
            .iter_mut()
            .filter(|entry| {
                entry.is_some_and(|pending| pending.challenge.expires_at_us <= now_us)
            })
            .for_each(|entry| *entry = None);
        let record = self
            .database
            .login_record(username, node)
            .ok_or(AuthError::UserNotFound)?;
        let selected = record
            .credential(credential)
            .ok_or(AuthError::CredentialNotFound)?;
        if required_kind.is_some_and(|kind| kind != selected.kind) {
            return Err(AuthError::CredentialNotFound)
        }
        self.next_nonce = self.next_nonce.wrapping_add(1).max(1);
        let challenge = AuthenticationChallenge {
            identity: record.identity,
            credential,
            nonce: self.next_nonce,
            expires_at_us: now_us.saturating_add(lifetime_us),
        };
        let slot = self
            .pending
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(AuthError::Capacity)?;
        *slot = Some(PendingChallenge { challenge, record });
        Ok(challenge)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_authentication<V: CredentialVerifier>(
        &mut self,
        challenge: AuthenticationChallenge,
        response: &[u8],
        verifier: &mut V,
        login_address_space: AddressSpaceId,
        now_us: u64,
        session_lifetime_us: u64,
    ) -> Result<Session, AuthError> {
        let slot = self
            .pending
            .iter()
            .position(|entry| entry.is_some_and(|pending| pending.challenge == challenge))
            .ok_or(AuthError::InvalidChallenge)?;
        let pending = self.pending[slot].take().expect("pending challenge");
        if now_us >= challenge.expires_at_us || session_lifetime_us == 0 {
            return Err(AuthError::InvalidChallenge)
        }
        let record = self
            .database
            .identity_record(pending.record.identity)
            .ok_or(AuthError::UserNotFound)?;
        let credential = record
            .credential(challenge.credential)
            .ok_or(AuthError::CredentialNotFound)?;
        if !verifier.verify(
            credential.kind,
            credential.public_material(),
            &challenge.bytes(),
            response,
        ) {
            audit_event!(
                Level::Warn,
                EventField::unsigned(field::AUTH_ACTION, 1),
                EventField::unsigned(field::IDENTITY, record.identity.raw()),
                EventField::unsigned(field::CALLER, login_address_space.raw() as u64),
                EventField::status(synos_status::Status::ACCESS_DENIED),
            );
            return Err(AuthError::VerificationFailed)
        }
        audit_event!(
            Level::Info,
            EventField::unsigned(field::AUTH_ACTION, 1),
            EventField::unsigned(field::IDENTITY, record.identity.raw()),
            EventField::unsigned(field::CALLER, login_address_space.raw() as u64),
            EventField::status(synos_status::Status::NORMAL),
        );
        Ok(Session {
            record,
            login_address_space,
            expires_at_us: now_us.saturating_add(session_lifetime_us),
        })
    }
}

#[derive(Clone, Copy)]
pub struct Session {
    record: UserRecord,
    pub login_address_space: AddressSpaceId,
    pub expires_at_us: u64,
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Session")
            .field("identity", &self.identity())
            .field("login_address_space", &self.login_address_space)
            .field("expires_at_us", &self.expires_at_us)
            .finish()
    }
}

impl Session {
    pub const fn identity(&self) -> IdentityId {
        self.record.identity
    }

    pub fn persona(&self) -> Result<ExecutionPersona, AuthError> {
        let mut rights = [RightIdentifier::BATCH_JOB; MAX_USER_RIGHTS];
        let mut length = 0;
        for right in self.record.rights() {
            rights[length] = right;
            length += 1
        }
        ExecutionPersona::new(self.record.identity, &rights[..length])
            .map_err(|_| AuthError::Capacity)
    }

    pub fn instantiate_capabilities<const CAPACITY: usize>(
        &self,
        capabilities: &mut CapabilitySpace<CAPACITY>,
        now_us: u64,
    ) -> Result<SessionCapabilities, AuthError> {
        if now_us >= self.expires_at_us {
            return Err(AuthError::SessionExpired)
        }
        let mut issued = SessionCapabilities::new();
        for grant in self.record.initial_capabilities() {
            let handle = match capabilities.mint_root(
                self.login_address_space,
                grant.object,
                grant.rights,
            ) {
                Ok(handle) => handle,
                Err(_) => {
                    issued.rollback(capabilities, self.login_address_space);
                    return Err(AuthError::CapabilityFailure)
                }
            };
            issued.push(handle)?
        }
        Ok(issued)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SessionCapabilities {
    handles: [Option<CapabilityHandle>; MAX_INITIAL_CAPABILITIES],
}

impl SessionCapabilities {
    const fn new() -> Self {
        Self {
            handles: [None; MAX_INITIAL_CAPABILITIES],
        }
    }

    pub fn handles(&self) -> impl Iterator<Item = CapabilityHandle> + '_ {
        self.handles.iter().flatten().copied()
    }

    fn push(&mut self, handle: CapabilityHandle) -> Result<(), AuthError> {
        let slot = self
            .handles
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(AuthError::Capacity)?;
        *slot = Some(handle);
        Ok(())
    }

    fn rollback<const CAPACITY: usize>(
        &mut self,
        capabilities: &mut CapabilitySpace<CAPACITY>,
        owner: AddressSpaceId,
    ) {
        for handle in self.handles.iter_mut().filter_map(Option::take) {
            let _ = capabilities.delete(owner, handle);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthError {
    AlreadyExists,
    Capacity,
    CapabilityFailure,
    CredentialNotFound,
    InvalidChallenge,
    InvalidRecord,
    SessionExpired,
    StorageFailure,
    UserNotFound,
    VerificationFailed,
}
