//! Central identity and VM-fleet authorization.
//!
//! This module is the transport-independent directory authority. A network
//! service can own a `Directory` and expose these operations over HTTP, CXL,
//! or another authenticated fabric protocol. The directory stores public
//! credential material only; private passkey keys remain in the authenticator.

use core::fmt;

use ghostos_fabric::NodeId;
use ghostos_kernel::{
    AddressSpaceId, ExecutionPersona, IdentityId, RightIdentifier, MAX_PERSONA_RIGHTS,
};

use crate::{
    AccountRole, AccountState, AuthDaemon, AuthError, AuthenticationChallenge,
    AuthorizationDatabase, CapabilityKey, Credential, CredentialId, CredentialVerifier,
    DatabaseScope, GroupDirectory, GroupRecord, TokenError, UserRecord,
};
use crate::identity::{DEFAULT_CHALLENGE_CAPACITY, DEFAULT_USER_CAPACITY};
use crate::startup::MAX_GROUPS;

pub const DEFAULT_DIRECTORY_RP_ID: &str = "login.ghostos.local";
pub const DEFAULT_DIRECTORY_ORIGIN: &str = "https://login.ghostos.local";
pub const MAX_DIRECTORY_NODES: usize = 64;
pub const DEFAULT_DIRECTORY_NONCE_CAPACITY: usize = 64;
pub const DEFAULT_DIRECTORY_CACHE_CAPACITY: usize = 32;
pub const DIRECTORY_JOIN_WINDOW_US: u64 = 120_000_000;
pub const DIRECTORY_TOKEN_MAX_LIFETIME_US: u64 = 300_000_000;
pub const DEFAULT_DIRECTORY_OFFLINE_LIFETIME_US: u64 = 900_000_000;

const JOIN_PAYLOAD_BYTES: usize = 32;
const DIRECTORY_TOKEN_BASE_BYTES: usize = 232;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryError {
    AlreadyExists,
    Authentication(AuthError),
    CacheCapacity,
    CacheExpired,
    CacheMiss,
    Capacity,
    Expired,
    InvalidJoin,
    InvalidPolicy,
    InvalidToken,
    LastAdministrator,
    NodeAlreadyJoined,
    NodeNotFound,
    NodeRevoked,
    Replay,
    StalePolicy,
    Token(TokenError),
    UserNotFound,
    WrongNode,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NodeJoinRequest {
    pub node: NodeId,
    pub issued_at_us: u64,
    pub nonce: u64,
    authenticator: [u8; 32],
}

impl fmt::Debug for NodeJoinRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeJoinRequest")
            .field("node", &self.node)
            .field("issued_at_us", &self.issued_at_us)
            .field("nonce", &"[redacted]")
            .finish()
    }
}

impl NodeJoinRequest {
    pub const WIRE_BYTES: usize = 64;

    pub fn new(
        node: NodeId,
        issued_at_us: u64,
        nonce: u64,
        bootstrap_key: CapabilityKey,
    ) -> Result<Self, DirectoryError> {
        if node.raw() == 0 || nonce == 0 {
            return Err(DirectoryError::InvalidJoin)
        }
        let mut request = Self {
            node,
            issued_at_us,
            nonce,
            authenticator: [0; 32],
        };
        request.authenticator = bootstrap_key
            .authenticate(&request.payload())
            .map_err(DirectoryError::Token)?;
        Ok(request)
    }

    pub fn verify(self, bootstrap_key: CapabilityKey) -> Result<(), DirectoryError> {
        if self.node.raw() == 0 || self.nonce == 0 {
            return Err(DirectoryError::InvalidJoin)
        }
        bootstrap_key
            .verify_authenticator(&self.payload(), &self.authenticator)
            .map_err(|error| match error {
                TokenError::InvalidSignature => DirectoryError::InvalidJoin,
                other => DirectoryError::Token(other),
            })
    }

    pub fn encode(self) -> [u8; Self::WIRE_BYTES] {
        let mut wire = [0; Self::WIRE_BYTES];
        wire[..JOIN_PAYLOAD_BYTES].copy_from_slice(&self.payload());
        wire[JOIN_PAYLOAD_BYTES..].copy_from_slice(&self.authenticator);
        wire
    }

    pub fn decode(wire: [u8; Self::WIRE_BYTES]) -> Result<Self, DirectoryError> {
        if &wire[0..4] != b"SYJN"
            || wire[4] != 1
            || wire[5..8].iter().any(|byte| *byte != 0)
            || wire[28..32].iter().any(|byte| *byte != 0)
        {
            return Err(DirectoryError::InvalidJoin)
        }
        let node = NodeId::new(read_u32(&wire, 8)).ok_or(DirectoryError::InvalidJoin)?;
        let nonce = read_u64(&wire, 20);
        if nonce == 0 {
            return Err(DirectoryError::InvalidJoin)
        }
        let mut authenticator = [0; 32];
        authenticator.copy_from_slice(&wire[JOIN_PAYLOAD_BYTES..]);
        Ok(Self {
            node,
            issued_at_us: read_u64(&wire, 12),
            nonce,
            authenticator,
        })
    }

    fn payload(self) -> [u8; JOIN_PAYLOAD_BYTES] {
        let mut payload = [0; JOIN_PAYLOAD_BYTES];
        payload[0..4].copy_from_slice(b"SYJN");
        payload[4] = 1;
        payload[8..12].copy_from_slice(&self.node.raw().to_be_bytes());
        payload[12..20].copy_from_slice(&self.issued_at_us.to_be_bytes());
        payload[20..28].copy_from_slice(&self.nonce.to_be_bytes());
        payload
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryNode {
    pub node: NodeId,
    pub joined_at_us: u64,
    pub join_nonce: u64,
    pub revoked: bool,
}

impl DirectoryNode {
    pub const fn is_active(self) -> bool {
        !self.revoked
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct DirectoryTrustAnchor {
    node: NodeId,
    issuer: NodeId,
    signing_key: CapabilityKey,
    revocation_epoch: u64,
    policy_generation: u64,
}

impl fmt::Debug for DirectoryTrustAnchor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectoryTrustAnchor")
            .field("node", &self.node)
            .field("issuer", &self.issuer)
            .field("signing_key", &"[redacted]")
            .field("revocation_epoch", &self.revocation_epoch)
            .field("policy_generation", &self.policy_generation)
            .finish()
    }
}

impl DirectoryTrustAnchor {
    pub const fn node(self) -> NodeId {
        self.node
    }

    pub const fn issuer(self) -> NodeId {
        self.issuer
    }

    pub const fn revocation_epoch(self) -> u64 {
        self.revocation_epoch
    }

    pub const fn policy_generation(self) -> u64 {
        self.policy_generation
    }

    pub fn verify_token(
        self,
        token: &DirectoryToken,
        challenge: &[u8; 32],
        now_us: u64,
    ) -> Result<VerifiedDirectoryToken, DirectoryError> {
        token.verify(
            self.signing_key,
            self.issuer,
            self.node,
            challenge,
            now_us,
            self.revocation_epoch,
            self.policy_generation,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryClaims {
    pub identity: IdentityId,
    pub credential: CredentialId,
    pub node: NodeId,
    rights: [Option<RightIdentifier>; MAX_PERSONA_RIGHTS],
    rights_length: u8,
}

impl DirectoryClaims {
    pub fn new(
        identity: IdentityId,
        credential: CredentialId,
        node: NodeId,
    ) -> Result<Self, DirectoryError> {
        if identity == IdentityId::ANONYMOUS || node.raw() == 0 {
            return Err(DirectoryError::InvalidToken)
        }
        Ok(Self {
            identity,
            credential,
            node,
            rights: [None; MAX_PERSONA_RIGHTS],
            rights_length: 0,
        })
    }

    pub fn rights(&self) -> impl Iterator<Item = RightIdentifier> + '_ {
        self.rights[..self.rights_length as usize]
            .iter()
            .flatten()
            .copied()
    }

    pub fn has(&self, right: RightIdentifier) -> bool {
        self.rights().any(|existing| existing == right)
    }

    pub fn persona(&self) -> Result<ExecutionPersona, DirectoryError> {
        let mut rights = [RightIdentifier::BATCH_JOB; MAX_PERSONA_RIGHTS];
        let mut length = 0;
        for right in self.rights() {
            rights[length] = right;
            length += 1;
        }
        ExecutionPersona::new(self.identity, &rights[..length])
            .map_err(|_| DirectoryError::Capacity)
    }

    fn push_right(&mut self, right: RightIdentifier) -> Result<(), DirectoryError> {
        if self.has(right) {
            return Ok(())
        }
        if right.raw() == 0 {
            return Err(DirectoryError::InvalidToken)
        }
        let slot = self
            .rights
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(DirectoryError::Capacity)?;
        *slot = Some(right);
        self.rights_length += 1;
        Ok(())
    }

}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectorySession {
    identity: IdentityId,
    credential: CredentialId,
    node: NodeId,
    challenge_binding: [u8; 32],
    pub authenticated_at_us: u64,
    pub expires_at_us: u64,
}

impl DirectorySession {
    pub const fn identity(self) -> IdentityId {
        self.identity
    }

    pub const fn credential(self) -> CredentialId {
        self.credential
    }

    pub const fn node(self) -> NodeId {
        self.node
    }

    pub const fn challenge_binding(self) -> [u8; 32] {
        self.challenge_binding
    }

    pub const fn is_usable_at(self, now_us: u64) -> bool {
        now_us >= self.authenticated_at_us && now_us < self.expires_at_us
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct DirectoryToken {
    pub issuer: NodeId,
    pub target: NodeId,
    pub identity: IdentityId,
    pub credential: CredentialId,
    pub not_before_us: u64,
    pub expires_at_us: u64,
    pub revocation_epoch: u64,
    pub policy_generation: u64,
    nonce: u64,
    challenge: [u8; 32],
    rights: [Option<RightIdentifier>; MAX_PERSONA_RIGHTS],
    rights_length: u8,
    authenticator: [u8; 32],
}

impl fmt::Debug for DirectoryToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectoryToken")
            .field("value", &"[redacted]")
            .finish()
    }
}

impl DirectoryToken {
    pub const WIRE_BYTES: usize = 264;

    fn issue(
        key: CapabilityKey,
        issuer: NodeId,
        claims: DirectoryClaims,
        not_before_us: u64,
        expires_at_us: u64,
        revocation_epoch: u64,
        policy_generation: u64,
        nonce: u64,
        challenge: [u8; 32],
    ) -> Result<Self, DirectoryError> {
        if issuer.raw() == 0
            || expires_at_us <= not_before_us
            || expires_at_us - not_before_us > DIRECTORY_TOKEN_MAX_LIFETIME_US
            || revocation_epoch == 0
            || policy_generation == 0
            || nonce == 0
            || challenge.iter().all(|byte| *byte == 0)
        {
            return Err(DirectoryError::InvalidToken)
        }
        let mut token = Self {
            issuer,
            target: claims.node,
            identity: claims.identity,
            credential: claims.credential,
            not_before_us,
            expires_at_us,
            revocation_epoch,
            policy_generation,
            nonce,
            challenge,
            rights: claims.rights,
            rights_length: claims.rights_length,
            authenticator: [0; 32],
        };
        token.authenticator = key
            .authenticate(&token.base_bytes())
            .map_err(DirectoryError::Token)?;
        Ok(token)
    }

    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    pub fn challenge(&self) -> [u8; 32] {
        self.challenge
    }

    pub fn rights(&self) -> impl Iterator<Item = RightIdentifier> + '_ {
        self.rights[..self.rights_length as usize]
            .iter()
            .flatten()
            .copied()
    }

    pub fn encode(self) -> [u8; Self::WIRE_BYTES] {
        let mut wire = [0; Self::WIRE_BYTES];
        wire[..DIRECTORY_TOKEN_BASE_BYTES].copy_from_slice(&self.base_bytes());
        wire[DIRECTORY_TOKEN_BASE_BYTES..].copy_from_slice(&self.authenticator);
        wire
    }

    pub fn decode(wire: [u8; Self::WIRE_BYTES]) -> Result<Self, DirectoryError> {
        if &wire[0..4] != b"SYDT"
            || wire[4] != 1
            || wire[5..8].iter().any(|byte| *byte != 0)
            || wire[29..32].iter().any(|byte| *byte != 0)
        {
            return Err(DirectoryError::InvalidToken)
        }
        let issuer = NodeId::new(read_u32(&wire, 8)).ok_or(DirectoryError::InvalidToken)?;
        let target = NodeId::new(read_u32(&wire, 12)).ok_or(DirectoryError::InvalidToken)?;
        let identity = IdentityId::new(read_u64(&wire, 16)).ok_or(DirectoryError::InvalidToken)?;
        let credential = CredentialId::new(read_u32(&wire, 24)).ok_or(DirectoryError::InvalidToken)?;
        let rights_length = wire[28] as usize;
        if rights_length > MAX_PERSONA_RIGHTS {
            return Err(DirectoryError::InvalidToken)
        }
        let mut rights = [None; MAX_PERSONA_RIGHTS];
        for index in 0..rights_length {
            rights[index] = Some(
                RightIdentifier::new(read_u64(&wire, 104 + index * 8))
                    .ok_or(DirectoryError::InvalidToken)?,
            );
        }
        let mut challenge = [0; 32];
        challenge.copy_from_slice(&wire[72..104]);
        let mut authenticator = [0; 32];
        authenticator.copy_from_slice(&wire[DIRECTORY_TOKEN_BASE_BYTES..]);
        let token = Self {
            issuer,
            target,
            identity,
            credential,
            not_before_us: read_u64(&wire, 32),
            expires_at_us: read_u64(&wire, 40),
            revocation_epoch: read_u64(&wire, 48),
            policy_generation: read_u64(&wire, 56),
            nonce: read_u64(&wire, 64),
            challenge,
            rights,
            rights_length: wire[28],
            authenticator,
        };
        token.validate_shape()?;
        Ok(token)
    }

    fn verify(
        &self,
        key: CapabilityKey,
        issuer: NodeId,
        target: NodeId,
        challenge: &[u8; 32],
        now_us: u64,
        revocation_epoch: u64,
        policy_generation: u64,
    ) -> Result<VerifiedDirectoryToken, DirectoryError> {
        self.validate_shape()?;
        if self.issuer != issuer
            || self.target != target
            || self.challenge != *challenge
            || self.revocation_epoch != revocation_epoch
            || self.policy_generation != policy_generation
        {
            return Err(DirectoryError::InvalidToken)
        }
        if now_us < self.not_before_us || now_us >= self.expires_at_us {
            return Err(DirectoryError::Expired)
        }
        key.verify_authenticator(&self.base_bytes(), &self.authenticator)
            .map_err(|error| match error {
                TokenError::InvalidSignature => DirectoryError::InvalidToken,
                other => DirectoryError::Token(other),
            })?;
        Ok(VerifiedDirectoryToken {
            token: *self,
            claims: DirectoryClaims {
                identity: self.identity,
                credential: self.credential,
                node: self.target,
                rights: self.rights,
                rights_length: self.rights_length,
            },
        })
    }

    fn validate_shape(&self) -> Result<(), DirectoryError> {
        if self.issuer.raw() == 0
            || self.target.raw() == 0
            || self.identity == IdentityId::ANONYMOUS
            || self.credential.raw() == 0
            || self.expires_at_us <= self.not_before_us
            || self.expires_at_us - self.not_before_us > DIRECTORY_TOKEN_MAX_LIFETIME_US
            || self.revocation_epoch == 0
            || self.policy_generation == 0
            || self.nonce == 0
            || self.challenge.iter().all(|byte| *byte == 0)
            || self.rights_length as usize > MAX_PERSONA_RIGHTS
        {
            return Err(DirectoryError::InvalidToken)
        }
        for index in 0..self.rights_length as usize {
            let right = self.rights[index].ok_or(DirectoryError::InvalidToken)?;
            if self.rights[..index].contains(&Some(right)) {
                return Err(DirectoryError::InvalidToken)
            }
        }
        if self.rights[self.rights_length as usize..]
            .iter()
            .any(Option::is_some)
        {
            return Err(DirectoryError::InvalidToken)
        }
        Ok(())
    }

    fn base_bytes(&self) -> [u8; DIRECTORY_TOKEN_BASE_BYTES] {
        let mut bytes = [0; DIRECTORY_TOKEN_BASE_BYTES];
        bytes[0..4].copy_from_slice(b"SYDT");
        bytes[4] = 1;
        bytes[8..12].copy_from_slice(&self.issuer.raw().to_be_bytes());
        bytes[12..16].copy_from_slice(&self.target.raw().to_be_bytes());
        bytes[16..24].copy_from_slice(&self.identity.raw().to_be_bytes());
        bytes[24..28].copy_from_slice(&self.credential.raw().to_be_bytes());
        bytes[28] = self.rights_length;
        bytes[32..40].copy_from_slice(&self.not_before_us.to_be_bytes());
        bytes[40..48].copy_from_slice(&self.expires_at_us.to_be_bytes());
        bytes[48..56].copy_from_slice(&self.revocation_epoch.to_be_bytes());
        bytes[56..64].copy_from_slice(&self.policy_generation.to_be_bytes());
        bytes[64..72].copy_from_slice(&self.nonce.to_be_bytes());
        bytes[72..104].copy_from_slice(&self.challenge);
        for (index, right) in self.rights.iter().enumerate() {
            if let Some(right) = right {
                let start = 104 + index * 8;
                bytes[start..start + 8].copy_from_slice(&right.raw().to_be_bytes());
            }
        }
        bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedDirectoryToken {
    token: DirectoryToken,
    claims: DirectoryClaims,
}

impl VerifiedDirectoryToken {
    pub const fn token(&self) -> &DirectoryToken {
        &self.token
    }

    pub const fn claims(&self) -> &DirectoryClaims {
        &self.claims
    }
}

#[derive(Clone, Copy)]
struct UsedNonce {
    nonce: u64,
    expires_at_us: u64,
}

pub struct DirectoryTokenVerifier<const CAPACITY: usize = DEFAULT_DIRECTORY_NONCE_CAPACITY> {
    anchor: DirectoryTrustAnchor,
    used_nonces: [Option<UsedNonce>; CAPACITY],
}

impl<const CAPACITY: usize> DirectoryTokenVerifier<CAPACITY> {
    pub const fn new(anchor: DirectoryTrustAnchor) -> Self {
        Self {
            anchor,
            used_nonces: [None; CAPACITY],
        }
    }

    pub const fn anchor(&self) -> DirectoryTrustAnchor {
        self.anchor
    }

    pub fn update_anchor(&mut self, anchor: DirectoryTrustAnchor) -> Result<(), DirectoryError> {
        if anchor.node() != self.anchor.node() || anchor.issuer() != self.anchor.issuer() {
            return Err(DirectoryError::WrongNode)
        }
        self.anchor = anchor;
        self.used_nonces = [None; CAPACITY];
        Ok(())
    }

    pub fn accept(
        &mut self,
        token: &DirectoryToken,
        challenge: &[u8; 32],
        now_us: u64,
    ) -> Result<VerifiedDirectoryToken, DirectoryError> {
        for entry in &mut self.used_nonces {
            if entry.is_some_and(|used| used.expires_at_us <= now_us) {
                *entry = None;
            }
        }
        let verified = self.anchor.verify_token(token, challenge, now_us)?;
        if self
            .used_nonces
            .iter()
            .flatten()
            .any(|used| used.nonce == token.nonce)
        {
            return Err(DirectoryError::Replay)
        }
        let slot = self
            .used_nonces
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(DirectoryError::Capacity)?;
        *slot = Some(UsedNonce {
            nonce: token.nonce,
            expires_at_us: token.expires_at_us,
        });
        Ok(verified)
    }
}

#[derive(Clone, Copy)]
struct OfflineEntry {
    claims: DirectoryClaims,
    valid_until_us: u64,
    revocation_epoch: u64,
    policy_generation: u64,
}

pub struct DirectoryOfflineCache<const CAPACITY: usize = DEFAULT_DIRECTORY_CACHE_CAPACITY> {
    entries: [Option<OfflineEntry>; CAPACITY],
}

impl<const CAPACITY: usize> DirectoryOfflineCache<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            entries: [None; CAPACITY],
        }
    }

    pub fn cache(
        &mut self,
        verified: &VerifiedDirectoryToken,
        now_us: u64,
        max_offline_us: u64,
    ) -> Result<(), DirectoryError> {
        if max_offline_us == 0 || now_us >= verified.token.expires_at_us {
            return Err(DirectoryError::InvalidPolicy)
        }
        let valid_until_us = core::cmp::min(
            verified.token.expires_at_us,
            now_us
                .checked_add(max_offline_us)
                .ok_or(DirectoryError::InvalidPolicy)?,
        );
        for entry in &mut self.entries {
            if entry.is_some_and(|cached| cached.valid_until_us <= now_us) {
                *entry = None;
            }
        }
        if let Some(entry) = self.entries.iter_mut().flatten().find(|entry| {
            entry.claims.identity == verified.claims.identity
                && entry.claims.node == verified.claims.node
        }) {
            *entry = OfflineEntry {
                claims: verified.claims,
                valid_until_us,
                revocation_epoch: verified.token.revocation_epoch,
                policy_generation: verified.token.policy_generation,
            };
            return Ok(())
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(DirectoryError::CacheCapacity)?;
        *slot = Some(OfflineEntry {
            claims: verified.claims,
            valid_until_us,
            revocation_epoch: verified.token.revocation_epoch,
            policy_generation: verified.token.policy_generation,
        });
        Ok(())
    }

    pub fn authorize(
        &mut self,
        identity: IdentityId,
        node: NodeId,
        now_us: u64,
        revocation_epoch: u64,
        policy_generation: u64,
    ) -> Result<DirectoryClaims, DirectoryError> {
        let entry = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.claims.identity == identity && entry.claims.node == node)
            .ok_or(DirectoryError::CacheMiss)?;
        if now_us >= entry.valid_until_us {
            return Err(DirectoryError::CacheExpired)
        }
        if entry.revocation_epoch != revocation_epoch
            || entry.policy_generation != policy_generation
        {
            return Err(DirectoryError::StalePolicy)
        }
        Ok(entry.claims)
    }

    pub fn invalidate_identity(&mut self, identity: IdentityId) {
        for entry in &mut self.entries {
            if entry.is_some_and(|cached| cached.claims.identity == identity) {
                *entry = None;
            }
        }
    }

    pub const fn len(&self) -> usize {
        let mut count = 0;
        let mut index = 0;
        while index < CAPACITY {
            if self.entries[index].is_some() {
                count += 1;
            }
            index += 1;
        }
        count
    }
}

impl<const CAPACITY: usize> Default for DirectoryOfflineCache<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryBootstrapPolicy {
    pub require_directory: bool,
    pub allow_local_break_glass: bool,
    pub max_offline_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryBootstrapMode {
    CentralEnrollment,
    CentralLogin,
    OfflineLogin,
    LocalBreakGlass,
    DirectoryUnavailable,
    FirstBoot,
    RecoveryRequired,
}

impl DirectoryBootstrapPolicy {
    pub const fn new(
        require_directory: bool,
        allow_local_break_glass: bool,
        max_offline_us: u64,
    ) -> Result<Self, DirectoryError> {
        if max_offline_us == 0 {
            return Err(DirectoryError::InvalidPolicy)
        }
        Ok(Self {
            require_directory,
            allow_local_break_glass,
            max_offline_us,
        })
    }

    pub const fn select_mode(
        self,
        local_database_present: bool,
        directory_available: bool,
        offline_cache_available: bool,
        local_break_glass_requested: bool,
    ) -> DirectoryBootstrapMode {
        if local_break_glass_requested {
            return if self.allow_local_break_glass && local_database_present {
                DirectoryBootstrapMode::LocalBreakGlass
            } else {
                DirectoryBootstrapMode::RecoveryRequired
            }
        }
        if directory_available {
            return if local_database_present {
                DirectoryBootstrapMode::CentralLogin
            } else {
                DirectoryBootstrapMode::CentralEnrollment
            }
        }
        if !local_database_present {
            return DirectoryBootstrapMode::FirstBoot
        }
        if offline_cache_available && self.max_offline_us > 0 {
            return DirectoryBootstrapMode::OfflineLogin
        }
        if !self.require_directory && self.allow_local_break_glass {
            DirectoryBootstrapMode::LocalBreakGlass
        } else {
            DirectoryBootstrapMode::DirectoryUnavailable
        }
    }
}

pub struct Directory<
    const USERS: usize = DEFAULT_USER_CAPACITY,
    const CHALLENGES: usize = DEFAULT_CHALLENGE_CAPACITY,
    const NODES: usize = MAX_DIRECTORY_NODES,
    const GROUPS: usize = MAX_GROUPS,
> {
    issuer: NodeId,
    signing_key: CapabilityKey,
    bootstrap_key: CapabilityKey,
    authd: AuthDaemon<USERS, CHALLENGES>,
    groups: GroupDirectory<GROUPS>,
    nodes: [Option<DirectoryNode>; NODES],
    revocation_epoch: u64,
    policy_generation: u64,
    next_nonce: u64,
}

impl<
        const USERS: usize,
        const CHALLENGES: usize,
        const NODES: usize,
        const GROUPS: usize,
    > Directory<USERS, CHALLENGES, NODES, GROUPS>
{
    pub fn new(
        issuer: NodeId,
        signing_key: CapabilityKey,
        bootstrap_key: CapabilityKey,
        authd: AuthDaemon<USERS, CHALLENGES>,
    ) -> Self {
        Self {
            issuer,
            signing_key,
            bootstrap_key,
            authd,
            groups: GroupDirectory::new(),
            nodes: [None; NODES],
            revocation_epoch: 1,
            policy_generation: 1,
            next_nonce: 1,
        }
    }

    pub const fn issuer(&self) -> NodeId {
        self.issuer
    }

    pub const fn revocation_epoch(&self) -> u64 {
        self.revocation_epoch
    }

    pub const fn policy_generation(&self) -> u64 {
        self.policy_generation
    }

    pub const fn authd(&self) -> &AuthDaemon<USERS, CHALLENGES> {
        &self.authd
    }

    pub fn authd_mut(&mut self) -> &mut AuthDaemon<USERS, CHALLENGES> {
        &mut self.authd
    }

    pub const fn database(&self) -> &AuthorizationDatabase<USERS> {
        self.authd.database()
    }

    pub fn database_mut(&mut self) -> &mut AuthorizationDatabase<USERS> {
        self.authd.database_mut()
    }

    pub const fn groups(&self) -> &GroupDirectory<GROUPS> {
        &self.groups
    }

    pub fn groups_mut(&mut self) -> &mut GroupDirectory<GROUPS> {
        &mut self.groups
    }

    pub fn insert_group(&mut self, group: GroupRecord) -> Result<(), DirectoryError> {
        self.groups.insert(group).map_err(|error| match error {
            crate::StartupError::Capacity => DirectoryError::Capacity,
            crate::StartupError::AlreadyExists => DirectoryError::AlreadyExists,
            _ => DirectoryError::InvalidPolicy,
        })?;
        self.mark_policy_changed();
        Ok(())
    }

    pub fn insert_identity(&mut self, record: UserRecord) -> Result<(), DirectoryError> {
        self.database_mut()
            .insert(record)
            .map_err(DirectoryError::Authentication)?;
        self.mark_policy_changed();
        Ok(())
    }

    pub fn add_credential(
        &mut self,
        identity: IdentityId,
        credential: Credential,
        created_at_us: u64,
    ) -> Result<(), DirectoryError> {
        self.database_mut()
            .record_mut(identity)
            .map_err(DirectoryError::Authentication)?
            .add_credential_at(credential, created_at_us)
            .map_err(DirectoryError::Authentication)?;
        self.mark_policy_changed();
        Ok(())
    }

    pub fn assign_role(
        &mut self,
        identity: IdentityId,
        role: AccountRole,
    ) -> Result<(), DirectoryError> {
        self.database_mut()
            .record_mut(identity)
            .map_err(DirectoryError::Authentication)?
            .assign_role(role)
            .map_err(DirectoryError::Authentication)?;
        self.mark_policy_changed();
        Ok(())
    }

    pub fn assign_right(
        &mut self,
        identity: IdentityId,
        right: RightIdentifier,
    ) -> Result<(), DirectoryError> {
        self.database_mut()
            .record_mut(identity)
            .map_err(DirectoryError::Authentication)?
            .assign_right(right)
            .map_err(DirectoryError::Authentication)?;
        self.mark_policy_changed();
        Ok(())
    }

    pub fn set_credential_label(
        &mut self,
        identity: IdentityId,
        credential: CredentialId,
        label: &str,
    ) -> Result<(), DirectoryError> {
        self.database_mut()
            .record_mut(identity)
            .map_err(DirectoryError::Authentication)?
            .set_credential_label(credential, label)
            .map_err(DirectoryError::Authentication)?;
        self.mark_policy_changed();
        Ok(())
    }

    pub fn join_node(
        &mut self,
        request: NodeJoinRequest,
        now_us: u64,
    ) -> Result<DirectoryTrustAnchor, DirectoryError> {
        request.verify(self.bootstrap_key)?;
        if request.issued_at_us > now_us
            || now_us - request.issued_at_us > DIRECTORY_JOIN_WINDOW_US
        {
            return Err(DirectoryError::InvalidJoin)
        }
        if let Some(node) = self.nodes.iter_mut().flatten().find(|entry| entry.node == request.node) {
            if node.is_active() {
                return Err(DirectoryError::NodeAlreadyJoined)
            }
            if node.join_nonce == request.nonce {
                return Err(DirectoryError::InvalidJoin)
            }
            node.revoked = false;
            node.joined_at_us = now_us;
            node.join_nonce = request.nonce;
        } else {
            let slot = self
                .nodes
                .iter_mut()
                .find(|entry| entry.is_none())
                .ok_or(DirectoryError::Capacity)?;
            *slot = Some(DirectoryNode {
                node: request.node,
                joined_at_us: now_us,
                join_nonce: request.nonce,
                revoked: false,
            });
        }
        self.mark_policy_changed();
        self.trust_anchor_for(request.node)
    }

    pub fn revoke_node(&mut self, node: NodeId) -> Result<(), DirectoryError> {
        let record = self
            .nodes
            .iter_mut()
            .flatten()
            .find(|entry| entry.node == node)
            .ok_or(DirectoryError::NodeNotFound)?;
        record.revoked = true;
        self.bump_revocation_epoch();
        Ok(())
    }

    pub fn trust_anchor_for(&self, node: NodeId) -> Result<DirectoryTrustAnchor, DirectoryError> {
        let record = self
            .nodes
            .iter()
            .flatten()
            .find(|entry| entry.node == node)
            .ok_or(DirectoryError::NodeNotFound)?;
        if !record.is_active() {
            return Err(DirectoryError::NodeRevoked)
        }
        Ok(DirectoryTrustAnchor {
            node,
            issuer: self.issuer,
            signing_key: self.signing_key,
            revocation_epoch: self.revocation_epoch,
            policy_generation: self.policy_generation,
        })
    }

    pub fn nodes(&self) -> impl Iterator<Item = DirectoryNode> + '_ {
        self.nodes.iter().flatten().copied()
    }

    pub fn begin_login(
        &mut self,
        username: &str,
        node: NodeId,
        credential: CredentialId,
        now_us: u64,
        lifetime_us: u64,
    ) -> Result<AuthenticationChallenge, DirectoryError> {
        self.trust_anchor_for(node)?;
        self.authd
            .begin_authentication(username, node, credential, now_us, lifetime_us)
            .map_err(DirectoryError::Authentication)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_login<V: CredentialVerifier>(
        &mut self,
        challenge: AuthenticationChallenge,
        response: &[u8],
        verifier: &mut V,
        node: NodeId,
        login_address_space: AddressSpaceId,
        now_us: u64,
        session_lifetime_us: u64,
    ) -> Result<DirectorySession, DirectoryError> {
        self.trust_anchor_for(node)?;
        let session = self
            .authd
            .complete_authentication_from_node(
                challenge,
                response,
                verifier,
                node,
                login_address_space,
                now_us,
                session_lifetime_us,
            )
            .map_err(DirectoryError::Authentication)?;
        Ok(DirectorySession {
            identity: session.identity(),
            credential: challenge.credential,
            node: session.node(),
            challenge_binding: challenge_binding(challenge),
            authenticated_at_us: now_us,
            expires_at_us: session.expires_at_us,
        })
    }

    pub fn issue_token(
        &mut self,
        session: DirectorySession,
        now_us: u64,
        lifetime_us: u64,
    ) -> Result<DirectoryToken, DirectoryError> {
        self.trust_anchor_for(session.node)?;
        if !session.is_usable_at(now_us)
            || lifetime_us == 0
            || lifetime_us > DIRECTORY_TOKEN_MAX_LIFETIME_US
        {
            return Err(DirectoryError::InvalidToken)
        }
        let expires_at_us = now_us
            .checked_add(lifetime_us)
            .ok_or(DirectoryError::InvalidToken)?;
        if expires_at_us > session.expires_at_us {
            return Err(DirectoryError::InvalidToken)
        }
        let claims = self.claims_for(
            session.identity,
            session.credential,
            session.node,
            now_us,
        )?;
        self.next_nonce = self.next_nonce.wrapping_add(1).max(1);
        DirectoryToken::issue(
            self.signing_key,
            self.issuer,
            claims,
            now_us,
            expires_at_us,
            self.revocation_epoch,
            self.policy_generation,
            self.next_nonce,
            session.challenge_binding,
        )
    }

    pub fn revoke_identity(&mut self, identity: IdentityId) -> Result<(), DirectoryError> {
        let is_last_administrator = self
            .database()
            .record(identity)
            .map_err(DirectoryError::Authentication)?
            .has_role(AccountRole::Administrator)
            && self
                .database()
                .records()
                .filter(|record| {
                    record.account_state() == AccountState::Active
                        && record.has_role(AccountRole::Administrator)
                })
                .count()
                <= 1;
        if is_last_administrator {
            return Err(DirectoryError::LastAdministrator)
        }
        let record = self
            .database_mut()
            .record_mut(identity)
            .map_err(DirectoryError::Authentication)?;
        record.set_state(AccountState::Disabled);
        self.bump_revocation_epoch();
        Ok(())
    }

    pub fn revoke_credential(
        &mut self,
        identity: IdentityId,
        credential: CredentialId,
    ) -> Result<(), DirectoryError> {
        self.database_mut()
            .record_mut(identity)
            .map_err(DirectoryError::Authentication)?
            .set_credential_revoked(credential, true)
            .map_err(DirectoryError::Authentication)?;
        self.bump_revocation_epoch();
        Ok(())
    }

    pub fn revoke_all_sessions(&mut self) {
        self.bump_revocation_epoch();
    }

    pub fn mark_policy_changed(&mut self) {
        self.policy_generation = self.policy_generation.wrapping_add(1).max(1);
    }

    fn bump_revocation_epoch(&mut self) {
        self.revocation_epoch = self.revocation_epoch.wrapping_add(1).max(1);
        self.mark_policy_changed();
    }

    fn claims_for(
        &self,
        identity: IdentityId,
        credential: CredentialId,
        node: NodeId,
        now_us: u64,
    ) -> Result<DirectoryClaims, DirectoryError> {
        let record = self
            .database()
            .record(identity)
            .map_err(DirectoryError::Authentication)?;
        if !scope_allows_node(record.scope, node)
            || !record.is_login_usable_at(now_us)
            || !record
                .credential(credential)
                .is_some_and(|credential| credential.is_usable_at(now_us))
        {
            return Err(DirectoryError::UserNotFound)
        }
        let mut claims = DirectoryClaims::new(identity, credential, node)?;
        for right in record.rights() {
            claims.push_right(right)?;
        }
        for group in self.groups.groups() {
            if group.members().any(|member| member == identity) {
                for right in group.rights() {
                    claims.push_right(right)?;
                }
            }
        }
        Ok(claims)
    }
}

fn scope_allows_node(scope: DatabaseScope, node: NodeId) -> bool {
    match scope {
        DatabaseScope::Local => true,
        DatabaseScope::NodeLocal(bound_node) => bound_node == node,
    }
}

fn challenge_binding(challenge: AuthenticationChallenge) -> [u8; 32] {
    let mut binding = [0; 32];
    binding[0..4].copy_from_slice(b"SYAB");
    binding[4..32].copy_from_slice(&challenge.bytes());
    binding
}

fn read_u32(bytes: &[u8], start: usize) -> u32 {
    let mut value = [0; 4];
    value.copy_from_slice(&bytes[start..start + 4]);
    u32::from_be_bytes(value)
}

fn read_u64(bytes: &[u8], start: usize) -> u64 {
    let mut value = [0; 8];
    value.copy_from_slice(&bytes[start..start + 8]);
    u64::from_be_bytes(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AccountRole, AuthorizationDatabase, Credential, UserRecord, Username,
    };

    const BOOTSTRAP_KEY: CapabilityKey = CapabilityKey::new([0x11; 32]);
    const SIGNING_KEY: CapabilityKey = CapabilityKey::new([0x22; 32]);

    fn directory() -> Directory<4, 4, 4, 4> {
        let identity = IdentityId::new(7).unwrap();
        let mut user = UserRecord::new(
            identity,
            Username::new("alice").unwrap(),
            DatabaseScope::Local,
        );
        user.add_credential(
            Credential::new_passkey(CredentialId::new(1).unwrap(), b"public-key", 0).unwrap(),
        )
        .unwrap();
        user.assign_role(AccountRole::ReadOnly).unwrap();
        let mut database = AuthorizationDatabase::new();
        database.insert(user).unwrap();
        Directory::new(
            NodeId::LOCAL,
            SIGNING_KEY,
            BOOTSTRAP_KEY,
            AuthDaemon::new(database, 1),
        )
    }

    fn join(directory: &mut Directory<4, 4, 4, 4>, node: NodeId) -> DirectoryTrustAnchor {
        let request = NodeJoinRequest::new(node, 100, node.raw() as u64, BOOTSTRAP_KEY).unwrap();
        directory.join_node(request, 100).unwrap()
    }

    fn session(node: NodeId, challenge_binding: [u8; 32]) -> DirectorySession {
        DirectorySession {
            identity: IdentityId::new(7).unwrap(),
            credential: CredentialId::new(1).unwrap(),
            node,
            challenge_binding,
            authenticated_at_us: 100,
            expires_at_us: 10_000,
        }
    }

    struct AcceptVerifier;

    impl CredentialVerifier for AcceptVerifier {
        fn verify(
            &mut self,
            _kind: crate::CredentialKind,
            _public_material: &[u8],
            _challenge: &[u8],
            response: &[u8],
        ) -> bool {
            response == b"proof"
        }
    }

    #[test]
    fn shared_credential_authenticates_on_two_nodes() {
        let mut directory = directory();
        let node_a = NodeId::from_valid_raw(2);
        let node_b = NodeId::from_valid_raw(3);
        join(&mut directory, node_a);
        join(&mut directory, node_b);
        let credential = CredentialId::new(1).unwrap();
        let challenge_a = directory
            .begin_login("alice", node_a, credential, 200, 100)
            .unwrap();
        let challenge_b = directory
            .begin_login("alice", node_b, credential, 200, 100)
            .unwrap();
        let mut verifier = AcceptVerifier;
        let session_a = directory
            .complete_login(
                challenge_a,
                b"proof",
                &mut verifier,
                node_a,
                AddressSpaceId::new(2).unwrap(),
                201,
                1_000,
            )
            .unwrap();
        let session_b = directory
            .complete_login(
                challenge_b,
                b"proof",
                &mut verifier,
                node_b,
                AddressSpaceId::new(3).unwrap(),
                201,
                1_000,
            )
            .unwrap();

        assert_eq!(session_a.identity(), session_b.identity());
        assert_eq!(session_a.credential(), credential);
    }

    #[test]
    fn one_identity_can_issue_tokens_for_multiple_joined_nodes() {
        let mut directory = directory();
        let node_a = NodeId::from_valid_raw(2);
        let node_b = NodeId::from_valid_raw(3);
        let _initial_anchor_a = join(&mut directory, node_a);
        let anchor_b = join(&mut directory, node_b);
        let anchor_a = directory.trust_anchor_for(node_a).unwrap();
        let challenge = [0x33; 32];

        let token_a = directory.issue_token(session(node_a, challenge), 200, 100).unwrap();
        let token_b = directory.issue_token(session(node_b, challenge), 200, 100).unwrap();
        let verified_a = anchor_a.verify_token(&token_a, &challenge, 201).unwrap();
        let verified_b = anchor_b.verify_token(&token_b, &challenge, 201).unwrap();

        assert_eq!(verified_a.claims().identity, verified_b.claims().identity);
        assert!(verified_a.claims().has(RightIdentifier::SYSTEM_READ_ONLY));
        assert!(verified_a.claims().persona().unwrap().has(RightIdentifier::SYSTEM_READ_ONLY));
        assert_eq!(DirectoryToken::decode(token_a.encode()).unwrap(), token_a);
        assert_eq!(
            anchor_a.verify_token(&token_b, &challenge, 201),
            Err(DirectoryError::InvalidToken)
        );
    }

    #[test]
    fn token_verifier_rejects_replay_and_wrong_challenge() {
        let mut directory = directory();
        let node = NodeId::from_valid_raw(2);
        let anchor = join(&mut directory, node);
        let challenge = [0x44; 32];
        let token = directory.issue_token(session(node, challenge), 200, 100).unwrap();
        let mut verifier = DirectoryTokenVerifier::<2>::new(anchor);

        verifier.accept(&token, &challenge, 201).unwrap();
        assert_eq!(
            verifier.accept(&token, &challenge, 201),
            Err(DirectoryError::Replay)
        );
        assert_eq!(
            verifier.accept(&token, &[0x45; 32], 201),
            Err(DirectoryError::InvalidToken)
        );
        let mut wire = token.encode();
        wire[16] ^= 1;
        let tampered = DirectoryToken::decode(wire).unwrap();
        assert_eq!(
            anchor.verify_token(&tampered, &challenge, 201),
            Err(DirectoryError::InvalidToken)
        );
    }

    #[test]
    fn revocation_and_policy_changes_invalidate_cached_identity() {
        let mut directory = directory();
        let node = NodeId::from_valid_raw(2);
        let anchor = join(&mut directory, node);
        let challenge = [0x55; 32];
        let token = directory.issue_token(session(node, challenge), 200, 100).unwrap();
        let verified = anchor.verify_token(&token, &challenge, 201).unwrap();
        let mut cache = DirectoryOfflineCache::<2>::new();

        cache.cache(&verified, 201, 50).unwrap();
        assert!(cache
            .authorize(
                verified.claims().identity,
                node,
                249,
                directory.revocation_epoch(),
                directory.policy_generation(),
            )
            .is_ok());
        assert_eq!(
            cache.authorize(
                verified.claims().identity,
                node,
                251,
                directory.revocation_epoch(),
                directory.policy_generation(),
            ),
            Err(DirectoryError::CacheExpired)
        );

        cache.cache(&verified, 201, 50).unwrap();
        directory.mark_policy_changed();
        assert_eq!(
            cache.authorize(
                verified.claims().identity,
                node,
                202,
                directory.revocation_epoch(),
                directory.policy_generation(),
            ),
            Err(DirectoryError::StalePolicy)
        );
        directory.revoke_node(node).unwrap();
        assert_eq!(directory.trust_anchor_for(node), Err(DirectoryError::NodeRevoked));
    }

    #[test]
    fn node_join_requires_bootstrap_key_and_fresh_request() {
        let node = NodeId::from_valid_raw(2);
        let request = NodeJoinRequest::new(node, 100, 1, BOOTSTRAP_KEY).unwrap();
        assert_eq!(
            request.verify(CapabilityKey::new([0x99; 32])),
            Err(DirectoryError::InvalidJoin)
        );
        let mut directory = directory();
        assert_eq!(
            directory.join_node(request, 100 + DIRECTORY_JOIN_WINDOW_US + 1),
            Err(DirectoryError::InvalidJoin)
        );
        directory.join_node(request, 100).unwrap();
        directory.revoke_node(node).unwrap();
        assert_eq!(directory.join_node(request, 101), Err(DirectoryError::InvalidJoin));
        let rejoin = NodeJoinRequest::new(node, 101, 2, BOOTSTRAP_KEY).unwrap();
        assert!(directory.join_node(rejoin, 101).is_ok());
    }

    #[test]
    fn bootstrap_policy_keeps_a_recovery_path_and_last_admin() {
        let policy = DirectoryBootstrapPolicy::new(true, true, 100).unwrap();
        assert_eq!(
            policy.select_mode(false, true, false, false),
            DirectoryBootstrapMode::CentralEnrollment
        );
        assert_eq!(
            policy.select_mode(true, false, true, false),
            DirectoryBootstrapMode::OfflineLogin
        );
        assert_eq!(
            policy.select_mode(true, false, false, true),
            DirectoryBootstrapMode::LocalBreakGlass
        );

        let mut directory = directory();
        let identity = IdentityId::new(7).unwrap();
        directory
            .database_mut()
            .record_mut(identity)
            .unwrap()
            .assign_role(AccountRole::Administrator)
            .unwrap();
        assert_eq!(
            directory.revoke_identity(identity),
            Err(DirectoryError::LastAdministrator)
        );
    }
}
