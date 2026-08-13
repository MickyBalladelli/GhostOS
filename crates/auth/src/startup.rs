//! Security startup composition: persistent policy, first-admin bootstrap,
//! and bounded local login sessions.

use synos_fabric::NodeId;
use synos_kernel::{
    AddressSpaceId, CapabilityObject, IdentityId, RightIdentifier, Rights,
};

use crate::identity::{
    AccountState, AuthDaemon, AuthError, AuthenticationChallenge, AuthorizationDatabase, Credential,
    CredentialId, CredentialKind, CredentialVerifier, DatabaseScope, InitialCapability, Session,
    UserRecord, Username,
};

pub const MAX_GROUPS: usize = 32;
pub const MAX_GROUP_MEMBERS: usize = 64;
pub const MAX_GROUP_RIGHTS: usize = 16;
pub const MAX_ACTIVE_SESSIONS: usize = 32;
pub const MAX_CHALLENGE_LIFETIME_US: u64 = 120_000_000;
pub const DEFAULT_SESSION_LIFETIME_US: u64 = 900_000_000;
pub const MAX_SESSION_LIFETIME_US: u64 = 900_000_000;
pub const DEFAULT_IDLE_TIMEOUT_US: u64 = 300_000_000;
pub const MAX_IDLE_TIMEOUT_US: u64 = 900_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecurityPolicy {
    pub challenge_lifetime_us: u64,
    pub session_lifetime_us: u64,
    pub idle_timeout_us: u64,
    pub allow_self_credential_changes: bool,
}

impl SecurityPolicy {
    pub const fn new(
        challenge_lifetime_us: u64,
        session_lifetime_us: u64,
        idle_timeout_us: u64,
    ) -> Result<Self, StartupError> {
        if challenge_lifetime_us == 0
            || challenge_lifetime_us > MAX_CHALLENGE_LIFETIME_US
            || session_lifetime_us == 0
            || session_lifetime_us > MAX_SESSION_LIFETIME_US
            || idle_timeout_us == 0
            || idle_timeout_us > session_lifetime_us
            || idle_timeout_us > MAX_IDLE_TIMEOUT_US
        {
            return Err(StartupError::InvalidPolicy)
        }
        Ok(Self {
            challenge_lifetime_us,
            session_lifetime_us,
            idle_timeout_us,
            allow_self_credential_changes: true,
        })
    }

    pub const fn with_self_credential_changes(mut self, enabled: bool) -> Self {
        self.allow_self_credential_changes = enabled;
        self
    }
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            challenge_lifetime_us: 120_000_000,
            session_lifetime_us: DEFAULT_SESSION_LIFETIME_US,
            idle_timeout_us: DEFAULT_IDLE_TIMEOUT_US,
            allow_self_credential_changes: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct GroupId(u32);

impl GroupId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroupRecord {
    pub id: GroupId,
    name: [u8; 32],
    name_length: u8,
    rights: [Option<RightIdentifier>; MAX_GROUP_RIGHTS],
    members: [Option<IdentityId>; MAX_GROUP_MEMBERS],
}

impl GroupRecord {
    pub fn new(id: GroupId, name: &str) -> Result<Self, StartupError> {
        if name.is_empty() || name.len() > 32 || !name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || b"._-$".contains(&byte)
        }) {
            return Err(StartupError::InvalidRecord)
        }
        let mut bytes = [0; 32];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Ok(Self {
            id,
            name: bytes,
            name_length: name.len() as u8,
            rights: [None; MAX_GROUP_RIGHTS],
            members: [None; MAX_GROUP_MEMBERS],
        })
    }

    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_length as usize])
            .expect("GroupRecord name invariant")
    }

    pub fn rights(&self) -> impl Iterator<Item = RightIdentifier> + '_ {
        self.rights.iter().flatten().copied()
    }

    pub fn members(&self) -> impl Iterator<Item = IdentityId> + '_ {
        self.members.iter().flatten().copied()
    }

    pub fn grant_right(&mut self, right: RightIdentifier) -> Result<(), StartupError> {
        if self.rights().any(|existing| existing == right) {
            return Ok(())
        }
        let slot = self
            .rights
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(StartupError::Capacity)?;
        *slot = Some(right);
        Ok(())
    }

    pub fn add_member(&mut self, identity: IdentityId) -> Result<(), StartupError> {
        if identity == IdentityId::ANONYMOUS {
            return Err(StartupError::InvalidRecord)
        }
        if self.members().any(|existing| existing == identity) {
            return Ok(())
        }
        let slot = self
            .members
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(StartupError::Capacity)?;
        *slot = Some(identity);
        Ok(())
    }

    fn grants(&self, identity: IdentityId, right: RightIdentifier) -> bool {
        self.members().any(|member| member == identity)
            && self.rights().any(|granted| granted == right)
    }
}

#[derive(Clone, Copy)]
pub struct GroupDirectory<const CAPACITY: usize = MAX_GROUPS> {
    groups: [Option<GroupRecord>; CAPACITY],
}

impl<const CAPACITY: usize> GroupDirectory<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            groups: [None; CAPACITY],
        }
    }

    pub fn insert(&mut self, group: GroupRecord) -> Result<(), StartupError> {
        if self
            .groups
            .iter()
            .flatten()
            .any(|existing| existing.id == group.id || existing.name() == group.name())
        {
            return Err(StartupError::AlreadyExists)
        }
        let slot = self
            .groups
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(StartupError::Capacity)?;
        *slot = Some(group);
        Ok(())
    }

    pub fn groups(&self) -> impl Iterator<Item = GroupRecord> + '_ {
        self.groups.iter().flatten().copied()
    }

    pub fn allows(&self, identity: IdentityId, right: RightIdentifier) -> bool {
        self.groups.iter().flatten().any(|group| group.grants(identity, right))
    }
}

impl<const CAPACITY: usize> Default for GroupDirectory<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub struct SecurityState<
    const USERS: usize = 64,
    const GROUPS: usize = MAX_GROUPS,
> {
    pub database: AuthorizationDatabase<USERS>,
    pub groups: GroupDirectory<GROUPS>,
    pub policy: SecurityPolicy,
    pub generation: u64,
}

impl<const USERS: usize, const GROUPS: usize> SecurityState<USERS, GROUPS> {
    pub const fn new(policy: SecurityPolicy) -> Self {
        Self {
            database: AuthorizationDatabase::new(),
            groups: GroupDirectory::new(),
            policy,
            generation: 1,
        }
    }

    pub fn validate(&self) -> Result<(), StartupError> {
        SecurityPolicy::new(
            self.policy.challenge_lifetime_us,
            self.policy.session_lifetime_us,
            self.policy.idle_timeout_us,
        )?;
        for record in self.database.records() {
            if record.identity == IdentityId::ANONYMOUS
                || (record.account_state() != AccountState::PendingSetup
                    && record.credentials().count() == 0)
            {
                return Err(StartupError::InvalidRecord)
            }
        }
        Ok(())
    }

    pub fn create_first_admin(
        &mut self,
        identity: IdentityId,
        username: &str,
        scope: DatabaseScope,
        credential: Credential,
    ) -> Result<UserRecord, StartupError> {
        if !self.database.is_empty() || identity == IdentityId::ANONYMOUS {
            return Err(StartupError::AlreadyProvisioned)
        }
        if credential.kind() == CredentialKind::SshKey {
            return Err(StartupError::BootstrapCredentialRequired)
        }
        if credential.public_material().is_empty() {
            return Err(StartupError::InvalidRecord)
        }
        let mut record = UserRecord::new(identity, Username::new(username).map_err(|_| StartupError::InvalidRecord)?, scope);
        record
            .add_credential(credential)
            .map_err(StartupError::Authentication)?;
        record
            .assign_right(RightIdentifier::SYSTEM_ADMIN)
            .map_err(StartupError::Authentication)?;
        record
            .grant_initial_capability(InitialCapability {
                object: CapabilityObject::SystemControl,
                rights: Rights::CONTROL,
            })
            .map_err(StartupError::Authentication)?;
        self.database
            .insert(record)
            .map_err(StartupError::Authentication)?;
        self.generation = self.generation.wrapping_add(1).max(1);
        Ok(record)
    }
}

pub trait SecurityStore<const USERS: usize, const GROUPS: usize> {
    /// Load and verify the authenticated persistent security record. Secret
    /// credential responses must never be part of the stored state.
    fn load(
        &mut self,
        state: &mut SecurityState<USERS, GROUPS>,
    ) -> Result<bool, SecurityStoreError>;

    fn store(
        &mut self,
        state: &SecurityState<USERS, GROUPS>,
    ) -> Result<(), SecurityStoreError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityStoreError {
    Corrupt,
    Unavailable,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupError {
    AlreadyExists,
    AlreadyProvisioned,
    Authentication(AuthError),
    BootstrapCredentialRequired,
    Capacity,
    InvalidPolicy,
    InvalidRecord,
    NotProvisioned,
    SessionExpired,
    SessionNotFound,
    SessionRevoked,
    Storage(SecurityStoreError),
    LastAdministrator,
}

#[derive(Clone, Copy)]
pub enum AccountManagementRequest {
    Create(UserRecord),
    Update(UserRecord),
    Rename { identity: IdentityId, username: Username },
    Disable(IdentityId),
    Delete(IdentityId),
}

#[derive(Clone, Copy)]
pub enum AccountManagementResult {
    Created(UserRecord),
    Updated(UserRecord),
    Renamed(UserRecord),
    Disabled(UserRecord),
    Deleted(UserRecord),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct SessionHandle(u64);

impl SessionHandle {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionView {
    pub handle: SessionHandle,
    pub identity: IdentityId,
    pub credential: CredentialId,
    pub authenticated_at_us: u64,
    pub last_activity_us: u64,
    pub expires_at_us: u64,
    pub revocation_epoch: u64,
}

#[derive(Clone, Copy)]
struct ActiveSession {
    handle: SessionHandle,
    session: Session,
    credential: CredentialId,
    authenticated_at_us: u64,
    last_activity_us: u64,
    revocation_epoch: u64,
}

pub struct SessionManager<
    const USERS: usize = 64,
    const CHALLENGES: usize = 16,
    const SESSIONS: usize = MAX_ACTIVE_SESSIONS,
> {
    authd: AuthDaemon<USERS, CHALLENGES>,
    policy: SecurityPolicy,
    active: [Option<ActiveSession>; SESSIONS],
    next_handle: u64,
    revocation_epoch: u64,
}

impl<const USERS: usize, const CHALLENGES: usize, const SESSIONS: usize>
    SessionManager<USERS, CHALLENGES, SESSIONS>
{
    pub const fn new(
        database: AuthorizationDatabase<USERS>,
        policy: SecurityPolicy,
        boot_nonce: u64,
    ) -> Self {
        Self {
            authd: AuthDaemon::new(database, boot_nonce),
            policy,
            active: [None; SESSIONS],
            next_handle: 0,
            revocation_epoch: 1,
        }
    }

    pub fn begin_login(
        &mut self,
        username: &str,
        node: NodeId,
        credential: CredentialId,
        kind: CredentialKind,
        now_us: u64,
    ) -> Result<AuthenticationChallenge, StartupError> {
        self.authd
            .begin_authentication_for_kind(
                username,
                node,
                credential,
                Some(kind),
                now_us,
                self.policy.challenge_lifetime_us,
            )
            .map_err(StartupError::Authentication)
    }

    pub fn complete_login<V: CredentialVerifier>(
        &mut self,
        challenge: AuthenticationChallenge,
        response: &[u8],
        verifier: &mut V,
        login_address_space: AddressSpaceId,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        let session = self
            .authd
            .complete_authentication(
                challenge,
                response,
                verifier,
                login_address_space,
                now_us,
                self.policy.session_lifetime_us,
            )
            .map_err(StartupError::Authentication)?;
        let slot = self
            .active
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(StartupError::Capacity)?;
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        let active = ActiveSession {
            handle: SessionHandle::new(self.next_handle).expect("nonzero session handle"),
            session,
            credential: challenge.credential,
            authenticated_at_us: now_us,
            last_activity_us: now_us,
            revocation_epoch: self.revocation_epoch,
        };
        *slot = Some(active);
        Ok(Self::view_of(active))
    }

    pub fn authorize(
        &mut self,
        handle: SessionHandle,
        right: RightIdentifier,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        let revocation_epoch = self.revocation_epoch;
        let idle_timeout_us = self.policy.idle_timeout_us;
        let active = self.active_session_mut(handle)?;
        if active.revocation_epoch != revocation_epoch {
            return Err(StartupError::SessionRevoked)
        }
        if now_us >= active.session.expires_at_us {
            return Err(StartupError::SessionExpired)
        }
        if now_us.saturating_sub(active.last_activity_us) >= idle_timeout_us {
            return Err(StartupError::SessionExpired)
        }
        if !active.session.persona().map_err(StartupError::Authentication)?.has(right) {
            return Err(StartupError::Authentication(AuthError::VerificationFailed))
        }
        active.last_activity_us = now_us;
        Ok(Self::view_of(*active))
    }

    pub fn touch(
        &mut self,
        handle: SessionHandle,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        let revocation_epoch = self.revocation_epoch;
        let idle_timeout_us = self.policy.idle_timeout_us;
        let active = self.active_session_mut(handle)?;
        if active.revocation_epoch != revocation_epoch {
            return Err(StartupError::SessionRevoked)
        }
        if now_us >= active.session.expires_at_us
            || now_us.saturating_sub(active.last_activity_us) >= idle_timeout_us
        {
            return Err(StartupError::SessionExpired)
        }
        active.last_activity_us = now_us;
        Ok(Self::view_of(*active))
    }

    pub fn logout(&mut self, handle: SessionHandle) -> Result<(), StartupError> {
        let slot = self
            .active
            .iter_mut()
            .find(|entry| entry.is_some_and(|active| active.handle == handle))
            .ok_or(StartupError::SessionNotFound)?;
        *slot = None;
        Ok(())
    }

    pub fn revoke_identity(&mut self, identity: IdentityId) -> usize {
        self.revocation_epoch = self.revocation_epoch.wrapping_add(1).max(1);
        let revocation_epoch = self.revocation_epoch;
        let mut revoked = 0;
        for slot in &mut self.active {
            if slot.is_some_and(|active| active.session.identity() == identity) {
                *slot = None;
                revoked += 1;
            } else if let Some(active) = slot {
                active.revocation_epoch = revocation_epoch;
            }
        }
        revoked
    }

    pub fn expire(&mut self, now_us: u64) -> usize {
        let mut expired = 0;
        for slot in &mut self.active {
            if slot.is_some_and(|active| {
                now_us >= active.session.expires_at_us
                    || now_us.saturating_sub(active.last_activity_us)
                        >= self.policy.idle_timeout_us
            }) {
                *slot = None;
                expired += 1;
            }
        }
        expired
    }

    pub const fn revocation_epoch(&self) -> u64 {
        self.revocation_epoch
    }

    pub const fn authd(&self) -> &AuthDaemon<USERS, CHALLENGES> {
        &self.authd
    }

    pub fn authd_mut(&mut self) -> &mut AuthDaemon<USERS, CHALLENGES> {
        &mut self.authd
    }

    fn active_session_mut(
        &mut self,
        handle: SessionHandle,
    ) -> Result<&mut ActiveSession, StartupError> {
        self.active
            .iter_mut()
            .flatten()
            .find(|active| active.handle == handle)
            .ok_or(StartupError::SessionNotFound)
    }

    const fn view_of(active: ActiveSession) -> SessionView {
        SessionView {
            handle: active.handle,
            identity: active.session.identity(),
            credential: active.credential,
            authenticated_at_us: active.authenticated_at_us,
            last_activity_us: active.last_activity_us,
            expires_at_us: active.session.expires_at_us,
            revocation_epoch: active.revocation_epoch,
        }
    }
}

pub struct BootLoginService<
    const USERS: usize = 64,
    const CHALLENGES: usize = 16,
    const SESSIONS: usize = MAX_ACTIVE_SESSIONS,
    const GROUPS: usize = MAX_GROUPS,
> {
    state: SecurityState<USERS, GROUPS>,
    sessions: SessionManager<USERS, CHALLENGES, SESSIONS>,
    provisioning_required: bool,
}

impl<const USERS: usize, const CHALLENGES: usize, const SESSIONS: usize, const GROUPS: usize>
    BootLoginService<USERS, CHALLENGES, SESSIONS, GROUPS>
{
    pub fn start<S: SecurityStore<USERS, GROUPS>>(
        store: &mut S,
        policy: SecurityPolicy,
        boot_nonce: u64,
    ) -> Result<Self, StartupError> {
        let mut state = SecurityState::new(policy);
        let _loaded = store.load(&mut state).map_err(StartupError::Storage)?;
        state.validate()?;
        let provisioning_required = state.database.is_empty();
        let sessions = SessionManager::new(state.database, state.policy, boot_nonce);
        Ok(Self {
            state,
            sessions,
            provisioning_required,
        })
    }

    pub const fn provisioning_required(&self) -> bool {
        self.provisioning_required
    }

    pub fn create_first_admin<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        identity: IdentityId,
        username: &str,
        scope: DatabaseScope,
        credential: Credential,
    ) -> Result<UserRecord, StartupError> {
        let mut next_state = self.state;
        let record = next_state.create_first_admin(identity, username, scope, credential)?;
        store.store(&next_state).map_err(StartupError::Storage)?;
        self.sessions
            .authd_mut()
            .database_mut()
            .insert(record)
            .map_err(StartupError::Authentication)?;
        self.state = next_state;
        self.provisioning_required = false;
        Ok(record)
    }

    pub fn begin_login(
        &mut self,
        username: &str,
        node: NodeId,
        credential: CredentialId,
        kind: CredentialKind,
        now_us: u64,
    ) -> Result<AuthenticationChallenge, StartupError> {
        if self.provisioning_required {
            return Err(StartupError::NotProvisioned)
        }
        self.sessions.begin_login(username, node, credential, kind, now_us)
    }

    pub fn complete_login<V: CredentialVerifier>(
        &mut self,
        challenge: AuthenticationChallenge,
        response: &[u8],
        verifier: &mut V,
        login_address_space: AddressSpaceId,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        let view = self.sessions.complete_login(
            challenge,
            response,
            verifier,
            login_address_space,
            now_us,
        )?;
        self.state.database = *self.sessions.authd().database();
        Ok(view)
    }

    pub fn authorize(
        &mut self,
        handle: SessionHandle,
        right: RightIdentifier,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        match self.sessions.authorize(handle, right, now_us) {
            Ok(view) => Ok(view),
            Err(StartupError::Authentication(AuthError::VerificationFailed))
                if self.state.groups.allows(
                    self.sessions
                        .touch(handle, now_us)
                        .map_err(|_| StartupError::SessionRevoked)?
                        .identity,
                    right,
                ) => self.sessions.touch(handle, now_us),
            Err(error) => Err(error),
        }
    }

    pub fn logout(&mut self, handle: SessionHandle) -> Result<(), StartupError> {
        self.sessions.logout(handle)
    }

    pub fn revoke_identity(&mut self, identity: IdentityId) -> usize {
        self.sessions.revoke_identity(identity)
    }

    pub fn manage_account<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        request: AccountManagementRequest,
        now_us: u64,
    ) -> Result<AccountManagementResult, StartupError> {
        self.authorize(handle, RightIdentifier::SYSTEM_ADMIN, now_us)?;
        let mut next_state = self.state;
        let (result, revoked_identity) = match request {
            AccountManagementRequest::Create(record) => {
                Self::validate_managed_record(record)?;
                next_state
                    .database
                    .insert(record)
                    .map_err(StartupError::Authentication)?;
                (AccountManagementResult::Created(record), None)
            }
            AccountManagementRequest::Update(record) => {
                Self::validate_managed_record(record)?;
                let previous = next_state
                    .database
                    .record(record.identity)
                    .map_err(StartupError::Authentication)?;
                if Self::is_last_enabled_administrator(&next_state.database, previous)
                    && (!record.is_login_usable() || !Self::is_administrator(&record))
                {
                    return Err(StartupError::LastAdministrator)
                }
                next_state
                    .database
                    .replace(record)
                    .map_err(StartupError::Authentication)?;
                (AccountManagementResult::Updated(record), Some(record.identity))
            }
            AccountManagementRequest::Rename { identity, username } => {
                let mut record = next_state
                    .database
                    .record(identity)
                    .map_err(StartupError::Authentication)?;
                record.username = username;
                next_state
                    .database
                    .replace(record)
                    .map_err(StartupError::Authentication)?;
                (AccountManagementResult::Renamed(record), None)
            }
            AccountManagementRequest::Disable(identity) => {
                let mut record = next_state
                    .database
                    .record(identity)
                    .map_err(StartupError::Authentication)?;
                if Self::is_last_enabled_administrator(&next_state.database, record) {
                    return Err(StartupError::LastAdministrator)
                }
                record.set_state(AccountState::Disabled);
                next_state
                    .database
                    .replace(record)
                    .map_err(StartupError::Authentication)?;
                (AccountManagementResult::Disabled(record), Some(identity))
            }
            AccountManagementRequest::Delete(identity) => {
                let record = next_state
                    .database
                    .record(identity)
                    .map_err(StartupError::Authentication)?;
                if Self::is_last_enabled_administrator(&next_state.database, record) {
                    return Err(StartupError::LastAdministrator)
                }
                next_state
                    .database
                    .remove(identity)
                    .map_err(StartupError::Authentication)?;
                (AccountManagementResult::Deleted(record), Some(identity))
            }
        };
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        next_state.validate()?;
        store.store(&next_state).map_err(StartupError::Storage)?;
        *self.sessions.authd_mut().database_mut() = next_state.database;
        self.state = next_state;
        if let Some(identity) = revoked_identity {
            self.sessions.revoke_identity(identity);
        }
        Ok(result)
    }

    pub fn create_account<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        record: UserRecord,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        match self.manage_account(
            store,
            handle,
            AccountManagementRequest::Create(record),
            now_us,
        )? {
            AccountManagementResult::Created(record) => Ok(record),
            _ => Err(StartupError::InvalidRecord),
        }
    }

    pub fn update_account<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        record: UserRecord,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        match self.manage_account(
            store,
            handle,
            AccountManagementRequest::Update(record),
            now_us,
        )? {
            AccountManagementResult::Updated(record) => Ok(record),
            _ => Err(StartupError::InvalidRecord),
        }
    }

    pub fn rename_account<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        identity: IdentityId,
        username: Username,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        match self.manage_account(
            store,
            handle,
            AccountManagementRequest::Rename { identity, username },
            now_us,
        )? {
            AccountManagementResult::Renamed(record) => Ok(record),
            _ => Err(StartupError::InvalidRecord),
        }
    }

    pub fn disable_account<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        identity: IdentityId,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        match self.manage_account(
            store,
            handle,
            AccountManagementRequest::Disable(identity),
            now_us,
        )? {
            AccountManagementResult::Disabled(record) => Ok(record),
            _ => Err(StartupError::InvalidRecord),
        }
    }

    pub fn delete_account<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        identity: IdentityId,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        match self.manage_account(
            store,
            handle,
            AccountManagementRequest::Delete(identity),
            now_us,
        )? {
            AccountManagementResult::Deleted(record) => Ok(record),
            _ => Err(StartupError::InvalidRecord),
        }
    }

    pub fn add_credential<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        identity: IdentityId,
        credential: Credential,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        self.authorize_credential_change(handle, identity, now_us)?;
        let mut next_state = self.state;
        let mut record = next_state
            .database
            .record(identity)
            .map_err(StartupError::Authentication)?;
        record
            .add_credential_at(credential, now_us)
            .map_err(StartupError::Authentication)?;
        if record.account_state() == AccountState::PendingSetup {
            record.set_state(AccountState::Active);
        }
        next_state
            .database
            .replace(record)
            .map_err(StartupError::Authentication)?;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        next_state.validate()?;
        store.store(&next_state).map_err(StartupError::Storage)?;
        *self.sessions.authd_mut().database_mut() = next_state.database;
        self.state = next_state;
        self.sessions.revoke_identity(identity);
        Ok(record)
    }

    pub fn remove_credential<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        identity: IdentityId,
        credential: CredentialId,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        self.authorize_credential_change(handle, identity, now_us)?;
        let mut next_state = self.state;
        let mut record = next_state
            .database
            .record(identity)
            .map_err(StartupError::Authentication)?;
        record
            .remove_credential(credential)
            .map_err(StartupError::Authentication)?;
        next_state
            .database
            .replace(record)
            .map_err(StartupError::Authentication)?;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        next_state.validate()?;
        store.store(&next_state).map_err(StartupError::Storage)?;
        *self.sessions.authd_mut().database_mut() = next_state.database;
        self.state = next_state;
        self.sessions.revoke_identity(identity);
        Ok(record)
    }

    fn authorize_credential_change(
        &mut self,
        handle: SessionHandle,
        identity: IdentityId,
        now_us: u64,
    ) -> Result<(), StartupError> {
        let caller = self.sessions.touch(handle, now_us)?.identity;
        if caller == identity && self.state.policy.allow_self_credential_changes {
            return Ok(())
        }
        self.authorize(handle, RightIdentifier::SYSTEM_ADMIN, now_us)?;
        Ok(())
    }

    fn validate_managed_record(record: UserRecord) -> Result<(), StartupError> {
        if record.identity == IdentityId::ANONYMOUS
            || (record.account_state() != AccountState::PendingSetup
                && record.credentials().count() == 0)
        {
            return Err(StartupError::InvalidRecord)
        }
        Ok(())
    }

    fn is_administrator(record: &UserRecord) -> bool {
        record.rights().any(|right| right == RightIdentifier::SYSTEM_ADMIN)
    }

    fn is_last_enabled_administrator(
        database: &AuthorizationDatabase<USERS>,
        target: UserRecord,
    ) -> bool {
        Self::is_administrator(&target)
            && target.is_login_usable()
            && database
                .records()
                .filter(|record| record.is_login_usable() && Self::is_administrator(record))
                .count()
                <= 1
    }

    pub fn expire(&mut self, now_us: u64) -> usize {
        self.sessions.expire(now_us)
    }

    pub const fn state(&self) -> &SecurityState<USERS, GROUPS> {
        &self.state
    }

    pub const fn sessions(&self) -> &SessionManager<USERS, CHALLENGES, SESSIONS> {
        &self.sessions
    }
}
