//! Security startup composition: persistent policy, first-admin bootstrap,
//! and bounded local login sessions.

use core::convert::TryFrom;

use synos_fabric::NodeId;
use synos_kernel::{
    AddressSpaceId, CapabilityObject, IdentityId, RightIdentifier, Rights,
};

use crate::identity::{
    AccountRole, AccountState, AuthDaemon, AuthError, AuthenticationChallenge, AuthorizationDatabase,
    Credential,
    CredentialId, CredentialKind, CredentialVerifier, DatabaseScope, InitialCapability,
    PasswordVerifier, Session, UserRecord, Username,
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
pub struct PasswordRecoveryPolicy {
    pub trusted_recovery_identity: IdentityId,
    pub trusted_recovery_key_id: u64,
    pub recovery_window_us: u64,
}

impl PasswordRecoveryPolicy {
    pub const fn new(
        trusted_recovery_identity: IdentityId,
        trusted_recovery_key_id: u64,
        recovery_window_us: u64,
    ) -> Result<Self, StartupError> {
        if trusted_recovery_identity.raw() == IdentityId::ANONYMOUS.raw()
            || trusted_recovery_key_id == 0
            || trusted_recovery_key_id > u32::MAX as u64
            || recovery_window_us == 0
        {
            return Err(StartupError::InvalidPolicy)
        }
        Ok(Self {
            trusted_recovery_identity,
            trusted_recovery_key_id,
            recovery_window_us,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryChallenge {
    pub target_identity: IdentityId,
    pub recovery_identity: IdentityId,
    pub recovery_credential: CredentialId,
    pub nonce: u64,
    pub expires_at_us: u64,
}

impl RecoveryChallenge {
    pub fn bytes(self) -> [u8; 40] {
        let mut bytes = [0; 40];
        bytes[0..4].copy_from_slice(b"SYRC");
        bytes[4..12].copy_from_slice(&self.target_identity.raw().to_be_bytes());
        bytes[12..20].copy_from_slice(&self.recovery_identity.raw().to_be_bytes());
        bytes[20..24].copy_from_slice(&self.recovery_credential.raw().to_be_bytes());
        bytes[24..32].copy_from_slice(&self.nonce.to_be_bytes());
        bytes[32..40].copy_from_slice(&self.expires_at_us.to_be_bytes());
        bytes
    }
}

pub trait PhysicalRecovery {
    fn authorize(&mut self, challenge: RecoveryChallenge) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecurityPolicy {
    pub challenge_lifetime_us: u64,
    pub session_lifetime_us: u64,
    pub idle_timeout_us: u64,
    pub allow_self_credential_changes: bool,
    pub account_lifetime_us: Option<u64>,
    pub credential_lifetime_us: Option<u64>,
    pub password_login_enabled: bool,
    pub password_recovery_policy: Option<PasswordRecoveryPolicy>,
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
            account_lifetime_us: None,
            credential_lifetime_us: None,
            password_login_enabled: false,
            password_recovery_policy: None,
        })
    }

    pub const fn with_self_credential_changes(mut self, enabled: bool) -> Self {
        self.allow_self_credential_changes = enabled;
        self
    }

    pub const fn with_account_lifetime_us(mut self, lifetime_us: Option<u64>) -> Self {
        self.account_lifetime_us = lifetime_us;
        self
    }

    pub const fn with_credential_lifetime_us(mut self, lifetime_us: Option<u64>) -> Self {
        self.credential_lifetime_us = lifetime_us;
        self
    }

    pub const fn with_password_login(
        mut self,
        enabled: bool,
        recovery_policy: Option<PasswordRecoveryPolicy>,
    ) -> Self {
        self.password_login_enabled = enabled;
        self.password_recovery_policy = recovery_policy;
        self
    }

    fn validate(&self) -> Result<(), StartupError> {
        if self.account_lifetime_us == Some(0)
            || self.credential_lifetime_us == Some(0)
            || (self.password_login_enabled && self.password_recovery_policy.is_none())
        {
            return Err(StartupError::InvalidPolicy)
        }
        if let Some(recovery) = self.password_recovery_policy {
            PasswordRecoveryPolicy::new(
                recovery.trusted_recovery_identity,
                recovery.trusted_recovery_key_id,
                recovery.recovery_window_us,
            )?;
        }
        Ok(())
    }
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            challenge_lifetime_us: 120_000_000,
            session_lifetime_us: DEFAULT_SESSION_LIFETIME_US,
            idle_timeout_us: DEFAULT_IDLE_TIMEOUT_US,
            allow_self_credential_changes: true,
            account_lifetime_us: None,
            credential_lifetime_us: None,
            password_login_enabled: false,
            password_recovery_policy: None,
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

    pub fn revoke_right(&mut self, right: RightIdentifier) -> Result<(), StartupError> {
        let slot = self
            .rights
            .iter()
            .position(|entry| entry.is_some_and(|existing| existing == right))
            .ok_or(StartupError::GroupRightNotFound)?;
        self.rights[slot] = None;
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

    pub fn remove_member(&mut self, identity: IdentityId) -> Result<(), StartupError> {
        let slot = self
            .members
            .iter()
            .position(|entry| entry.is_some_and(|member| member == identity))
            .ok_or(StartupError::GroupMemberNotFound)?;
        self.members[slot] = None;
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

    pub fn record(&self, id: GroupId) -> Result<GroupRecord, StartupError> {
        self.groups
            .iter()
            .flatten()
            .find(|group| group.id == id)
            .copied()
            .ok_or(StartupError::GroupNotFound)
    }

    pub fn record_mut(&mut self, id: GroupId) -> Result<&mut GroupRecord, StartupError> {
        self.groups
            .iter_mut()
            .flatten()
            .find(|group| group.id == id)
            .ok_or(StartupError::GroupNotFound)
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
        self.policy.validate()?;
        SecurityPolicy::new(
            self.policy.challenge_lifetime_us,
            self.policy.session_lifetime_us,
            self.policy.idle_timeout_us,
        )?;
        for record in self.database.records() {
            if record.identity == IdentityId::ANONYMOUS
                || (record.account_state() != AccountState::PendingSetup
                    && (record.credentials().count() == 0
                        || (record.account_state() == AccountState::Active
                            && !record.is_login_usable())))
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
        self.create_first_admin_at(identity, username, scope, credential, 0)
    }

    pub fn create_first_admin_at(
        &mut self,
        identity: IdentityId,
        username: &str,
        scope: DatabaseScope,
        credential: Credential,
        now_us: u64,
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
            .add_credential_at(credential, now_us)
            .map_err(StartupError::Authentication)?;
        record.apply_expiration_policy(
            self.policy.account_lifetime_us,
            self.policy.credential_lifetime_us,
            now_us,
        );
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

    /// Replace the complete security state as one durable commit. An error
    /// must leave the previously committed state unchanged.
    fn store(
        &mut self,
        state: &SecurityState<USERS, GROUPS>,
    ) -> Result<(), SecurityStoreError>;

    fn store_atomic(
        &mut self,
        state: &SecurityState<USERS, GROUPS>,
    ) -> Result<(), SecurityStoreError> {
        self.store(state)
    }
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
    GroupMemberNotFound,
    GroupRightNotFound,
    GroupNotFound,
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
    pub terminal: AddressSpaceId,
    pub node: NodeId,
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
        if kind == CredentialKind::Password && !self.password_login_allowed() {
            return Err(StartupError::InvalidPolicy)
        }
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
        self.complete_login_from_node(
            challenge,
            response,
            verifier,
            NodeId::LOCAL,
            login_address_space,
            now_us,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_login_from_node<V: CredentialVerifier>(
        &mut self,
        challenge: AuthenticationChallenge,
        response: &[u8],
        verifier: &mut V,
        node: NodeId,
        login_address_space: AddressSpaceId,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        if self.challenge_is_password(challenge) {
            return Err(StartupError::InvalidPolicy)
        }
        let session = self
            .authd
            .complete_authentication_from_node(
                challenge,
                response,
                verifier,
                node,
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

    pub fn complete_password_login<V: PasswordVerifier>(
        &mut self,
        challenge: AuthenticationChallenge,
        password: &[u8],
        verifier: &mut V,
        login_address_space: AddressSpaceId,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        self.complete_password_login_from_node(
            challenge,
            password,
            verifier,
            NodeId::LOCAL,
            login_address_space,
            now_us,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_password_login_from_node<V: PasswordVerifier>(
        &mut self,
        challenge: AuthenticationChallenge,
        password: &[u8],
        verifier: &mut V,
        node: NodeId,
        login_address_space: AddressSpaceId,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        if !self.password_login_allowed() {
            return Err(StartupError::InvalidPolicy)
        }
        let session = self
            .authd
            .complete_password_authentication_from_node(
                challenge,
                password,
                verifier,
                node,
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

    fn password_login_allowed(&self) -> bool {
        self.policy.password_login_enabled && self.policy.password_recovery_policy.is_some()
    }

    fn challenge_is_password(&self, challenge: AuthenticationChallenge) -> bool {
        self.authd
            .database()
            .record(challenge.identity)
            .ok()
            .and_then(|record| record.credential_kind(challenge.credential).ok())
            == Some(CredentialKind::Password)
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
        if !active.session.is_usable_at(active.credential, now_us) {
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

    pub fn authorize_from(
        &mut self,
        handle: SessionHandle,
        right: RightIdentifier,
        node: NodeId,
        terminal: AddressSpaceId,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        self.require_binding(handle, node, terminal)?;
        self.authorize(handle, right, now_us)
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
            || !active.session.is_usable_at(active.credential, now_us)
        {
            return Err(StartupError::SessionExpired)
        }
        active.last_activity_us = now_us;
        Ok(Self::view_of(*active))
    }

    pub fn touch_from(
        &mut self,
        handle: SessionHandle,
        node: NodeId,
        terminal: AddressSpaceId,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        self.require_binding(handle, node, terminal)?;
        self.touch(handle, now_us)
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
                    || !active.session.is_usable_at(active.credential, now_us)
            }) {
                *slot = None;
                expired += 1;
            }
        }
        expired
    }

    pub fn active_sessions(
        &self,
        now_us: u64,
    ) -> impl Iterator<Item = SessionView> + '_ {
        self.active.iter().flatten().filter_map(move |active| {
            if active.revocation_epoch == self.revocation_epoch
                && now_us < active.session.expires_at_us
                && now_us.saturating_sub(active.last_activity_us) < self.policy.idle_timeout_us
                && active.session.is_usable_at(active.credential, now_us)
            {
                Some(Self::view_of(*active))
            } else {
                None
            }
        })
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

    pub fn set_policy(&mut self, policy: SecurityPolicy) {
        self.policy = policy;
        for active in self.active.iter_mut().flatten() {
            let maximum_expiration = active
                .authenticated_at_us
                .saturating_add(policy.session_lifetime_us);
            if active.session.expires_at_us > maximum_expiration {
                active.session.expires_at_us = maximum_expiration;
            }
        }
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

    fn require_binding(
        &self,
        handle: SessionHandle,
        node: NodeId,
        terminal: AddressSpaceId,
    ) -> Result<(), StartupError> {
        let active = self
            .active
            .iter()
            .flatten()
            .find(|active| active.handle == handle)
            .ok_or(StartupError::SessionNotFound)?;
        if active.revocation_epoch != self.revocation_epoch {
            return Err(StartupError::SessionRevoked)
        }
        if active.session.node() != node || active.session.login_address_space != terminal {
            return Err(StartupError::Authentication(AuthError::VerificationFailed))
        }
        Ok(())
    }

    const fn view_of(active: ActiveSession) -> SessionView {
        SessionView {
            handle: active.handle,
            identity: active.session.identity(),
            credential: active.credential,
            terminal: active.session.login_address_space,
            node: active.session.node(),
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
    pending_recovery: [Option<RecoveryChallenge>; CHALLENGES],
    next_recovery_nonce: u64,
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
            pending_recovery: [None; CHALLENGES],
            next_recovery_nonce: boot_nonce,
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
        self.create_first_admin_at(store, identity, username, scope, credential, 0)
    }

    pub fn create_first_admin_at<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        identity: IdentityId,
        username: &str,
        scope: DatabaseScope,
        credential: Credential,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        let mut next_state = self.state;
        let record = next_state.create_first_admin_at(
            identity,
            username,
            scope,
            credential,
            now_us,
        )?;
        self.commit_state(store, next_state)?;
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
        let result = self.sessions.begin_login(username, node, credential, kind, now_us);
        self.state.database = *self.sessions.authd().database();
        result
    }

    pub fn begin_password_login(
        &mut self,
        username: &str,
        node: NodeId,
        credential: CredentialId,
        now_us: u64,
    ) -> Result<AuthenticationChallenge, StartupError> {
        self.require_password_login_policy()?;
        self.begin_login(username, node, credential, CredentialKind::Password, now_us)
    }

    pub fn begin_credential_recovery(
        &mut self,
        target_identity: IdentityId,
        now_us: u64,
    ) -> Result<RecoveryChallenge, StartupError> {
        let policy = self
            .state
            .policy
            .password_recovery_policy
            .ok_or(StartupError::InvalidPolicy)?;
        if target_identity == IdentityId::ANONYMOUS {
            return Err(StartupError::InvalidRecord)
        }
        self.state
            .database
            .record(target_identity)
            .map_err(StartupError::Authentication)?;
        let recovery_credential = CredentialId::new(
            u32::try_from(policy.trusted_recovery_key_id)
                .map_err(|_| StartupError::InvalidPolicy)?,
        )
        .ok_or(StartupError::InvalidPolicy)?;
        let recovery_record = self
            .state
            .database
            .record(policy.trusted_recovery_identity)
            .map_err(StartupError::Authentication)?;
        let credential = recovery_record
            .credential(recovery_credential)
            .ok_or(StartupError::BootstrapCredentialRequired)?;
        if !credential.is_usable_at(now_us) {
            return Err(StartupError::BootstrapCredentialRequired)
        }
        self.pending_recovery
            .iter_mut()
            .filter(|entry| entry.is_some_and(|challenge| challenge.expires_at_us <= now_us))
            .for_each(|entry| *entry = None);
        let slot = self
            .pending_recovery
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(StartupError::Capacity)?;
        self.next_recovery_nonce = self.next_recovery_nonce.wrapping_add(1).max(1);
        let challenge = RecoveryChallenge {
            target_identity,
            recovery_identity: policy.trusted_recovery_identity,
            recovery_credential,
            nonce: self.next_recovery_nonce,
            expires_at_us: now_us.saturating_add(policy.recovery_window_us),
        };
        *slot = Some(challenge);
        Ok(challenge)
    }

    pub fn complete_credential_recovery<S: SecurityStore<USERS, GROUPS>, V: CredentialVerifier>(
        &mut self,
        store: &mut S,
        challenge: RecoveryChallenge,
        proof: &[u8],
        verifier: &mut V,
        replacement: Credential,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        let challenge = self.take_recovery_challenge(challenge, now_us)?;
        let credential = self.recovery_credential(challenge, now_us)?;
        if proof.is_empty()
            || proof.len() > crate::identity::MAX_AUTH_RESPONSE_BYTES
            || !verifier.verify(
                credential.kind(),
                credential.public_material(),
                &challenge.bytes(),
                proof,
            )
        {
            return Err(StartupError::Authentication(AuthError::VerificationFailed))
        }
        self.finish_credential_recovery(store, challenge, replacement, now_us)
    }

    pub fn complete_physical_credential_recovery<
        S: SecurityStore<USERS, GROUPS>,
        P: PhysicalRecovery,
    >(
        &mut self,
        store: &mut S,
        challenge: RecoveryChallenge,
        physical: &mut P,
        replacement: Credential,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        let challenge = self.take_recovery_challenge(challenge, now_us)?;
        if !physical.authorize(challenge) {
            return Err(StartupError::Authentication(AuthError::VerificationFailed))
        }
        self.finish_credential_recovery(store, challenge, replacement, now_us)
    }

    pub fn expire_accounts<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        now_us: u64,
    ) -> Result<usize, StartupError> {
        let mut next_state = self.state;
        let expired = next_state.database.refresh_expirations(now_us);
        if expired == 0 {
            return Ok(0)
        }
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
        for record in next_state.database.records() {
            if record.account_state() == AccountState::Expired {
                self.sessions.revoke_identity(record.identity);
            }
        }
        Ok(expired)
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

    pub fn complete_password_login<V: PasswordVerifier>(
        &mut self,
        challenge: AuthenticationChallenge,
        password: &[u8],
        verifier: &mut V,
        login_address_space: AddressSpaceId,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        self.require_password_login_policy()?;
        let view = self.sessions.complete_password_login(
            challenge,
            password,
            verifier,
            login_address_space,
            now_us,
        )?;
        self.state.database = *self.sessions.authd().database();
        Ok(view)
    }

    fn require_password_login_policy(&self) -> Result<(), StartupError> {
        if !self.state.policy.password_login_enabled
            || self.state.policy.password_recovery_policy.is_none()
        {
            return Err(StartupError::InvalidPolicy)
        }
        Ok(())
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

    pub fn authorize_from(
        &mut self,
        handle: SessionHandle,
        right: RightIdentifier,
        node: NodeId,
        terminal: AddressSpaceId,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        match self
            .sessions
            .authorize_from(handle, right, node, terminal, now_us)
        {
            Ok(view) => Ok(view),
            Err(StartupError::Authentication(AuthError::VerificationFailed))
                if self.state.groups.allows(
                    self.sessions
                        .touch_from(handle, node, terminal, now_us)
                        .map_err(|_| StartupError::SessionRevoked)?
                        .identity,
                    right,
                ) => self
                    .sessions
                    .touch_from(handle, node, terminal, now_us),
            Err(error) => Err(error),
        }
    }

    pub fn authorize_role(
        &mut self,
        handle: SessionHandle,
        role: AccountRole,
        now_us: u64,
    ) -> Result<SessionView, StartupError> {
        self.authorize(handle, role.right(), now_us)
    }

    pub fn logout(&mut self, handle: SessionHandle) -> Result<(), StartupError> {
        self.sessions.logout(handle)
    }

    pub fn revoke_identity(&mut self, identity: IdentityId) -> usize {
        self.sessions.revoke_identity(identity)
    }

    pub fn list_active_sessions(
        &mut self,
        handle: SessionHandle,
        now_us: u64,
    ) -> Result<[Option<SessionView>; SESSIONS], StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        let mut sessions = [None; SESSIONS];
        for (slot, session) in sessions
            .iter_mut()
            .zip(self.sessions.active_sessions(now_us))
        {
            *slot = Some(session);
        }
        Ok(sessions)
    }

    pub fn terminate_session(
        &mut self,
        administrator: SessionHandle,
        target: SessionHandle,
        now_us: u64,
    ) -> Result<(), StartupError> {
        self.authorize_role(administrator, AccountRole::Administrator, now_us)?;
        self.sessions.logout(target)
    }

    pub fn terminate_account_sessions(
        &mut self,
        administrator: SessionHandle,
        identity: IdentityId,
        now_us: u64,
    ) -> Result<usize, StartupError> {
        self.authorize_role(administrator, AccountRole::Administrator, now_us)?;
        self.state
            .database
            .record(identity)
            .map_err(StartupError::Authentication)?;
        Ok(self.sessions.revoke_identity(identity))
    }

    fn take_recovery_challenge(
        &mut self,
        challenge: RecoveryChallenge,
        now_us: u64,
    ) -> Result<RecoveryChallenge, StartupError> {
        let slot = self
            .pending_recovery
            .iter()
            .position(|entry| entry.is_some_and(|pending| pending == challenge))
            .ok_or(StartupError::SessionNotFound)?;
        let challenge = self.pending_recovery[slot]
            .take()
            .expect("recovery challenge");
        if now_us >= challenge.expires_at_us {
            return Err(StartupError::SessionExpired)
        }
        let policy = self
            .state
            .policy
            .password_recovery_policy
            .ok_or(StartupError::InvalidPolicy)?;
        if challenge.recovery_identity != policy.trusted_recovery_identity
            || u64::from(challenge.recovery_credential.raw()) != policy.trusted_recovery_key_id
        {
            return Err(StartupError::InvalidPolicy)
        }
        Ok(challenge)
    }

    fn recovery_credential(
        &self,
        challenge: RecoveryChallenge,
        now_us: u64,
    ) -> Result<Credential, StartupError> {
        let record = self
            .state
            .database
            .record(challenge.recovery_identity)
            .map_err(StartupError::Authentication)?;
        let credential = record
            .credential(challenge.recovery_credential)
            .ok_or(StartupError::BootstrapCredentialRequired)?;
        if !credential.is_usable_at(now_us) {
            return Err(StartupError::BootstrapCredentialRequired)
        }
        Ok(credential)
    }

    fn finish_credential_recovery<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        challenge: RecoveryChallenge,
        replacement: Credential,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        let mut next_state = self.state;
        let mut record = next_state
            .database
            .record(challenge.target_identity)
            .map_err(StartupError::Authentication)?;
        if record.account_state() == AccountState::Expired
            || replacement.public_material().is_empty()
        {
            return Err(StartupError::InvalidRecord)
        }
        record
            .replace_credentials_for_recovery(replacement, now_us)
            .map_err(StartupError::Authentication)?;
        record.apply_expiration_policy(
            next_state.policy.account_lifetime_us,
            next_state.policy.credential_lifetime_us,
            now_us,
        );
        next_state
            .database
            .replace(record)
            .map_err(StartupError::Authentication)?;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
        self.sessions.revoke_identity(challenge.target_identity);
        Ok(record)
    }

    fn commit_state<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        next_state: SecurityState<USERS, GROUPS>,
    ) -> Result<(), StartupError> {
        next_state.validate()?;
        store
            .store_atomic(&next_state)
            .map_err(StartupError::Storage)?;
        self.sessions.set_policy(next_state.policy);
        *self.sessions.authd_mut().database_mut() = next_state.database;
        self.state = next_state;
        Ok(())
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
            AccountManagementRequest::Create(mut record) => {
                record.apply_expiration_policy(
                    self.state.policy.account_lifetime_us,
                    self.state.policy.credential_lifetime_us,
                    now_us,
                );
                Self::validate_managed_record(record)?;
                next_state
                    .database
                    .insert(record)
                    .map_err(StartupError::Authentication)?;
                (AccountManagementResult::Created(record), None)
            }
            AccountManagementRequest::Update(mut record) => {
                record.apply_expiration_policy(
                    self.state.policy.account_lifetime_us,
                    self.state.policy.credential_lifetime_us,
                    now_us,
                );
                Self::validate_managed_record(record)?;
                let previous = next_state
                    .database
                    .record(record.identity)
                    .map_err(StartupError::Authentication)?;
                if Self::is_last_enabled_administrator(&next_state.database, previous, now_us)
                    && (!record.is_login_usable_at(now_us) || !Self::is_administrator(&record))
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
                if Self::is_last_enabled_administrator(&next_state.database, record, now_us) {
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
                if Self::is_last_enabled_administrator(&next_state.database, record, now_us) {
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
        self.commit_state(store, next_state)?;
        if let Some(identity) = revoked_identity {
            self.sessions.revoke_identity(identity);
        }
        Ok(result)
    }

    pub fn list_groups(
        &mut self,
        handle: SessionHandle,
        now_us: u64,
    ) -> Result<GroupDirectory<GROUPS>, StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        Ok(self.state.groups)
    }

    pub fn create_group<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        id: GroupId,
        name: &str,
        now_us: u64,
    ) -> Result<GroupRecord, StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        let mut next_state = self.state;
        let group = GroupRecord::new(id, name)?;
        next_state.groups.insert(group)?;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
        Ok(group)
    }

    pub fn add_group_member<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        group: GroupId,
        identity: IdentityId,
        now_us: u64,
    ) -> Result<GroupRecord, StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        let mut next_state = self.state;
        next_state
            .database
            .record(identity)
            .map_err(StartupError::Authentication)?;
        let group_record = next_state.groups.record_mut(group)?;
        group_record.add_member(identity)?;
        let group_record = *group_record;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
        Ok(group_record)
    }

    pub fn remove_group_member<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        group: GroupId,
        identity: IdentityId,
        now_us: u64,
    ) -> Result<GroupRecord, StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        let mut next_state = self.state;
        let group_record = next_state.groups.record_mut(group)?;
        group_record.remove_member(identity)?;
        let group_record = *group_record;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
        Ok(group_record)
    }

    pub fn grant_group_right<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        group: GroupId,
        right: RightIdentifier,
        now_us: u64,
    ) -> Result<GroupRecord, StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        let mut next_state = self.state;
        let group_record = next_state.groups.record_mut(group)?;
        group_record.grant_right(right)?;
        let group_record = *group_record;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
        Ok(group_record)
    }

    pub fn revoke_group_right<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        group: GroupId,
        right: RightIdentifier,
        now_us: u64,
    ) -> Result<GroupRecord, StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        let mut next_state = self.state;
        let group_record = next_state.groups.record_mut(group)?;
        group_record.revoke_right(right)?;
        let group_record = *group_record;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
        Ok(group_record)
    }

    pub fn update_policy<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        policy: SecurityPolicy,
        now_us: u64,
    ) -> Result<SecurityPolicy, StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        let mut next_state = self.state;
        next_state.policy = policy;
        next_state.database.apply_expiration_policy(
            policy.account_lifetime_us,
            policy.credential_lifetime_us,
            now_us,
        );
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
        Ok(policy)
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

    pub fn assign_role<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        identity: IdentityId,
        role: AccountRole,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        let mut record = self
            .state
            .database
            .record(identity)
            .map_err(StartupError::Authentication)?;
        record
            .assign_role(role)
            .map_err(StartupError::Authentication)?;
        self.update_account(store, handle, record, now_us)
    }

    pub fn remove_role<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        identity: IdentityId,
        role: AccountRole,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        let mut record = self
            .state
            .database
            .record(identity)
            .map_err(StartupError::Authentication)?;
        record
            .remove_role(role)
            .map_err(StartupError::Authentication)?;
        self.update_account(store, handle, record, now_us)
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

    pub fn grant_capability<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        identity: IdentityId,
        capability: InitialCapability,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        let mut next_state = self.state;
        let mut record = next_state
            .database
            .record(identity)
            .map_err(StartupError::Authentication)?;
        record
            .grant_initial_capability(capability)
            .map_err(StartupError::Authentication)?;
        next_state
            .database
            .replace(record)
            .map_err(StartupError::Authentication)?;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
        Ok(record)
    }

    pub fn revoke_capability<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        identity: IdentityId,
        capability: InitialCapability,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        self.authorize_role(handle, AccountRole::Administrator, now_us)?;
        let mut next_state = self.state;
        let mut record = next_state
            .database
            .record(identity)
            .map_err(StartupError::Authentication)?;
        record
            .remove_initial_capability(capability)
            .map_err(StartupError::Authentication)?;
        next_state
            .database
            .replace(record)
            .map_err(StartupError::Authentication)?;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
        Ok(record)
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
        record.apply_expiration_policy(
            self.state.policy.account_lifetime_us,
            self.state.policy.credential_lifetime_us,
            now_us,
        );
        if record.account_state() == AccountState::PendingSetup {
            record.set_state(AccountState::Active);
        }
        next_state
            .database
            .replace(record)
            .map_err(StartupError::Authentication)?;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
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
        self.commit_state(store, next_state)?;
        self.sessions.revoke_identity(identity);
        Ok(record)
    }

    pub fn revoke_credential<S: SecurityStore<USERS, GROUPS>>(
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
            .set_credential_revoked(credential, true)
            .map_err(StartupError::Authentication)?;
        next_state
            .database
            .replace(record)
            .map_err(StartupError::Authentication)?;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
        self.sessions.revoke_identity(identity);
        Ok(record)
    }

    pub fn rotate_credential<S: SecurityStore<USERS, GROUPS>>(
        &mut self,
        store: &mut S,
        handle: SessionHandle,
        identity: IdentityId,
        old_credential: CredentialId,
        new_credential: Credential,
        now_us: u64,
    ) -> Result<UserRecord, StartupError> {
        self.authorize_credential_change(handle, identity, now_us)?;
        let mut next_state = self.state;
        let mut record = next_state
            .database
            .record(identity)
            .map_err(StartupError::Authentication)?;
        if record
            .credential_is_revoked(old_credential)
            .map_err(StartupError::Authentication)?
        {
            return Err(StartupError::InvalidRecord)
        }
        record
            .add_credential_at(new_credential, now_us)
            .map_err(StartupError::Authentication)?;
        record.apply_expiration_policy(
            self.state.policy.account_lifetime_us,
            self.state.policy.credential_lifetime_us,
            now_us,
        );
        record
            .set_credential_revoked(old_credential, true)
            .map_err(StartupError::Authentication)?;
        next_state
            .database
            .replace(record)
            .map_err(StartupError::Authentication)?;
        next_state.generation = next_state.generation.wrapping_add(1).max(1);
        self.commit_state(store, next_state)?;
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
                && (record.credentials().count() == 0
                    || (record.account_state() == AccountState::Active
                        && !record.is_login_usable())))
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
        now_us: u64,
    ) -> bool {
        Self::is_administrator(&target)
            && target.is_login_usable_at(now_us)
            && database
                .records()
                .filter(|record| {
                    record.is_login_usable_at(now_us) && Self::is_administrator(record)
                })
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
