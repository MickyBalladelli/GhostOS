use synos_fabric::NodeId;
use synos_kernel::{AddressSpaceId, IdentityId, Rights};

use crate::{
    AuthDaemon, AuthError, AuthenticationChallenge, CapabilityCaveat, CapabilityKey,
    CredentialId, CredentialKind, CredentialVerifier, CryptographicCapability, Session,
    TokenError, TransportRights,
};
use crate::identity::MAX_CREDENTIAL_BYTES;

pub const MAX_WEBAUTHN_AUTHENTICATOR_DATA_BYTES: usize = 256;
pub const MAX_WEBAUTHN_CLIENT_DATA_BYTES: usize = 1_024;
pub const MAX_WEBAUTHN_SIGNATURE_BYTES: usize = 128;
pub const MAX_SSH_PUBLIC_KEY_BYTES: usize = MAX_CREDENTIAL_BYTES;
pub const MAX_SSH_EXCHANGE_HASH_BYTES: usize = 64;
pub const MAX_SSH_SIGNATURE_BYTES: usize = 512;
pub const MAX_REMOTE_CHALLENGE_LIFETIME_US: u64 = 120_000_000;
pub const MAX_REMOTE_SESSION_LIFETIME_US: u64 = 900_000_000;
pub const MAX_REMOTE_TOKEN_LIFETIME_US: u64 = 300_000_000;
pub const DEFAULT_REMOTE_SCOPE_CAPACITY: usize = 16;

const AUTHENTICATOR_DATA_HEADER_BYTES: usize = 37;
const FLAG_USER_PRESENT: u8 = 1 << 0;
const FLAG_USER_VERIFIED: u8 = 1 << 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebAuthnPolicy {
    rp_id_hash: [u8; 32],
    origin_hash: [u8; 32],
    challenge_lifetime_us: u64,
    session_lifetime_us: u64,
}

impl WebAuthnPolicy {
    pub const fn new(
        rp_id_hash: [u8; 32],
        origin_hash: [u8; 32],
        challenge_lifetime_us: u64,
        session_lifetime_us: u64,
    ) -> Result<Self, RemoteAuthError> {
        if all_zero(&rp_id_hash)
            || all_zero(&origin_hash)
            || challenge_lifetime_us == 0
            || challenge_lifetime_us > MAX_REMOTE_CHALLENGE_LIFETIME_US
            || session_lifetime_us == 0
            || session_lifetime_us > MAX_REMOTE_SESSION_LIFETIME_US
        {
            return Err(RemoteAuthError::InvalidPolicy)
        }
        Ok(Self {
            rp_id_hash,
            origin_hash,
            challenge_lifetime_us,
            session_lifetime_us,
        })
    }

    pub const fn rp_id_hash(self) -> [u8; 32] {
        self.rp_id_hash
    }

    pub const fn origin_hash(self) -> [u8; 32] {
        self.origin_hash
    }

    pub const fn challenge_lifetime_us(self) -> u64 {
        self.challenge_lifetime_us
    }

    pub const fn session_lifetime_us(self) -> u64 {
        self.session_lifetime_us
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct RemoteAuthenticationChallenge {
    pub authentication: AuthenticationChallenge,
    pub device: NodeId,
    pub rp_id_hash: [u8; 32],
    pub origin_hash: [u8; 32],
    pub ceremony_nonce: [u8; 32],
}

impl core::fmt::Debug for RemoteAuthenticationChallenge {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RemoteAuthenticationChallenge")
            .field("authentication", &self.authentication)
            .field("device", &self.device)
            .field("rp_id_hash", &self.rp_id_hash)
            .field("origin_hash", &self.origin_hash)
            .finish()
    }
}

impl RemoteAuthenticationChallenge {
    pub const WIRE_BYTES: usize = 136;

    /// WebAuthn challenge bytes. The assertion binds the login to this device,
    /// relying party, origin, credential, expiry, and one-shot daemon nonce.
    pub fn bytes(self) -> [u8; Self::WIRE_BYTES] {
        let mut bytes = [0; Self::WIRE_BYTES];
        bytes[0..4].copy_from_slice(b"SYWA");
        bytes[4] = 1;
        bytes[8..36].copy_from_slice(&self.authentication.bytes());
        bytes[36..40].copy_from_slice(&self.device.raw().to_be_bytes());
        bytes[40..72].copy_from_slice(&self.rp_id_hash);
        bytes[72..104].copy_from_slice(&self.origin_hash);
        bytes[104..136].copy_from_slice(&self.ceremony_nonce);
        bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebAuthnAssertion<'a> {
    pub authenticator_data: &'a [u8],
    pub client_data_json: &'a [u8],
    pub signature: &'a [u8],
}

impl<'a> WebAuthnAssertion<'a> {
    pub fn new(
        authenticator_data: &'a [u8],
        client_data_json: &'a [u8],
        signature: &'a [u8],
    ) -> Result<Self, RemoteAuthError> {
        if !(AUTHENTICATOR_DATA_HEADER_BYTES..=MAX_WEBAUTHN_AUTHENTICATOR_DATA_BYTES)
            .contains(&authenticator_data.len())
            || client_data_json.is_empty()
            || client_data_json.len() > MAX_WEBAUTHN_CLIENT_DATA_BYTES
            || signature.is_empty()
            || signature.len() > MAX_WEBAUTHN_SIGNATURE_BYTES
        {
            return Err(RemoteAuthError::InvalidAssertion)
        }
        Ok(Self {
            authenticator_data,
            client_data_json,
            signature,
        })
    }
}

pub struct WebAuthnVerificationRequest<'a> {
    pub cose_public_key: &'a [u8],
    pub expected_challenge: &'a [u8],
    pub expected_origin_hash: [u8; 32],
    pub authenticator_data: &'a [u8],
    pub client_data_json: &'a [u8],
    pub signature: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebAuthnVerification {
    pub signature_valid: bool,
    pub challenge_bound: bool,
    pub ceremony_is_get: bool,
    pub origin_hash: [u8; 32],
}

/// Platform boundary for COSE signature and clientDataJSON verification.
///
/// The implementation must verify the assertion signature over
/// authenticatorData || SHA-256(clientDataJSON), parse the JSON without
/// duplicate keys, and return the normalized origin hash.
pub trait WebAuthnVerifier {
    fn verify(
        &mut self,
        request: WebAuthnVerificationRequest<'_>,
    ) -> Result<WebAuthnVerification, RemoteAuthError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SshLoginPolicy {
    challenge_lifetime_us: u64,
    session_lifetime_us: u64,
}

impl SshLoginPolicy {
    pub const fn new(
        challenge_lifetime_us: u64,
        session_lifetime_us: u64,
    ) -> Result<Self, RemoteAuthError> {
        if challenge_lifetime_us == 0
            || challenge_lifetime_us > MAX_REMOTE_CHALLENGE_LIFETIME_US
            || session_lifetime_us == 0
            || session_lifetime_us > MAX_REMOTE_SESSION_LIFETIME_US
        {
            return Err(RemoteAuthError::InvalidPolicy)
        }
        Ok(Self {
            challenge_lifetime_us,
            session_lifetime_us,
        })
    }

    pub const fn challenge_lifetime_us(self) -> u64 {
        self.challenge_lifetime_us
    }

    pub const fn session_lifetime_us(self) -> u64 {
        self.session_lifetime_us
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SshAuthenticationChallenge {
    pub authentication: AuthenticationChallenge,
    pub device: NodeId,
    public_key: [u8; MAX_SSH_PUBLIC_KEY_BYTES],
    public_key_length: u8,
    exchange_hash: [u8; MAX_SSH_EXCHANGE_HASH_BYTES],
    exchange_hash_length: u8,
    challenge_lifetime_us: u64,
    session_lifetime_us: u64,
}

impl SshAuthenticationChallenge {
    pub const WIRE_BYTES: usize = 220;

    fn new(
        authentication: AuthenticationChallenge,
        device: NodeId,
        public_key: &[u8],
        exchange_hash: &[u8],
        policy: SshLoginPolicy,
    ) -> Result<Self, RemoteAuthError> {
        if public_key.is_empty()
            || public_key.len() > MAX_SSH_PUBLIC_KEY_BYTES
            || exchange_hash.is_empty()
            || exchange_hash.len() > MAX_SSH_EXCHANGE_HASH_BYTES
        {
            return Err(RemoteAuthError::InvalidSshProof)
        }
        let mut key = [0; MAX_SSH_PUBLIC_KEY_BYTES];
        key[..public_key.len()].copy_from_slice(public_key);
        let mut hash = [0; MAX_SSH_EXCHANGE_HASH_BYTES];
        hash[..exchange_hash.len()].copy_from_slice(exchange_hash);
        Ok(Self {
            authentication,
            device,
            public_key: key,
            public_key_length: public_key.len() as u8,
            exchange_hash: hash,
            exchange_hash_length: exchange_hash.len() as u8,
            challenge_lifetime_us: policy.challenge_lifetime_us,
            session_lifetime_us: policy.session_lifetime_us,
        })
    }

    pub fn public_key(&self) -> &[u8] {
        &self.public_key[..self.public_key_length as usize]
    }

    pub fn exchange_hash(&self) -> &[u8] {
        &self.exchange_hash[..self.exchange_hash_length as usize]
    }

    /// SSH challenge bytes. They bind the configured credential, device,
    /// public key, exchange hash, expiry, and one-shot authentication nonce.
    pub fn bytes(self) -> [u8; Self::WIRE_BYTES] {
        let mut bytes = [0; Self::WIRE_BYTES];
        bytes[0..4].copy_from_slice(b"SYSH");
        bytes[4] = 1;
        bytes[8..36].copy_from_slice(&self.authentication.bytes());
        bytes[36..40].copy_from_slice(&self.device.raw().to_be_bytes());
        bytes[40] = self.public_key_length;
        bytes[41] = self.exchange_hash_length;
        bytes[44..140].copy_from_slice(&self.public_key);
        bytes[140..204].copy_from_slice(&self.exchange_hash);
        bytes[204..212].copy_from_slice(&self.challenge_lifetime_us.to_be_bytes());
        bytes[212..220].copy_from_slice(&self.session_lifetime_us.to_be_bytes());
        bytes
    }
}

pub struct SshSignatureVerificationRequest<'a> {
    pub public_key: &'a [u8],
    pub exchange_hash: &'a [u8],
    pub challenge: &'a [u8],
    pub signature: &'a [u8],
}

/// Platform boundary for SSH public-key signature verification.
pub trait SshSignatureVerifier {
    fn verify(&mut self, request: SshSignatureVerificationRequest<'_>) -> bool;
}

#[derive(Clone, Copy, Debug)]
pub struct RemoteAdminSession {
    session: Session,
    device: NodeId,
    credential: CredentialId,
    authenticated_at_us: u64,
}

impl RemoteAdminSession {
    pub const fn identity(&self) -> IdentityId {
        self.session.identity()
    }

    pub const fn device(&self) -> NodeId {
        self.device
    }

    pub const fn credential(&self) -> CredentialId {
        self.credential
    }

    pub const fn authenticated_at_us(&self) -> u64 {
        self.authenticated_at_us
    }

    pub const fn expires_at_us(&self) -> u64 {
        self.session.expires_at_us
    }

    pub const fn local_session(&self) -> &Session {
        &self.session
    }
}

impl<const USERS: usize, const CHALLENGES: usize> AuthDaemon<USERS, CHALLENGES> {
    pub(crate) fn begin_remote_administration(
        &mut self,
        username: &str,
        login_node: NodeId,
        device: NodeId,
        credential: CredentialId,
        ceremony_nonce: [u8; 32],
        policy: WebAuthnPolicy,
        now_us: u64,
    ) -> Result<RemoteAuthenticationChallenge, RemoteAuthError> {
        if all_zero(&ceremony_nonce) {
            return Err(RemoteAuthError::InvalidChallengeEntropy)
        }
        let authentication = self
            .begin_authentication_for_kind(
                username,
                login_node,
                credential,
                Some(CredentialKind::Passkey),
                now_us,
                policy.challenge_lifetime_us,
            )
            .map_err(RemoteAuthError::Authentication)?;
        Ok(RemoteAuthenticationChallenge {
            authentication,
            device,
            rp_id_hash: policy.rp_id_hash,
            origin_hash: policy.origin_hash,
            ceremony_nonce,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn complete_remote_administration<V: WebAuthnVerifier>(
        &mut self,
        challenge: RemoteAuthenticationChallenge,
        assertion: WebAuthnAssertion<'_>,
        verifier: &mut V,
        policy: WebAuthnPolicy,
        login_address_space: AddressSpaceId,
        now_us: u64,
    ) -> Result<RemoteAdminSession, RemoteAuthError> {
        if challenge.rp_id_hash != policy.rp_id_hash
            || challenge.origin_hash != policy.origin_hash
        {
            return Err(RemoteAuthError::PolicyMismatch)
        }
        let assertion = WebAuthnAssertion::new(
            assertion.authenticator_data,
            assertion.client_data_json,
            assertion.signature,
        )?;
        let stored_sign_count = self
            .database_mut()
            .record_mut(challenge.authentication.identity)
            .and_then(|record| {
                record.passkey_sign_count(challenge.authentication.credential)
            })
            .map_err(RemoteAuthError::Authentication)?;
        let mut adapter = WebAuthnAdapter {
            challenge,
            assertion,
            verifier,
            policy,
            stored_sign_count,
            sign_count: None,
        };
        let session = self
            .complete_authentication(
                challenge.authentication,
                assertion.signature,
                &mut adapter,
                login_address_space,
                now_us,
                policy.session_lifetime_us,
            )
            .map_err(RemoteAuthError::Authentication)?;
        let sign_count = adapter
            .sign_count
            .ok_or(RemoteAuthError::InvalidAssertion)?;
        self.database_mut()
            .record_mut(session.identity())
            .and_then(|record| {
                record.record_passkey_use(challenge.authentication.credential, sign_count)
            })
            .map_err(RemoteAuthError::Authentication)?;
        Ok(RemoteAdminSession {
            session,
            device: challenge.device,
            credential: challenge.authentication.credential,
            authenticated_at_us: now_us,
        })
    }

    pub(crate) fn begin_ssh_login(
        &mut self,
        username: &str,
        login_node: NodeId,
        device: NodeId,
        public_key: &[u8],
        exchange_hash: &[u8],
        policy: SshLoginPolicy,
        now_us: u64,
    ) -> Result<SshAuthenticationChallenge, RemoteAuthError> {
        if public_key.is_empty()
            || public_key.len() > MAX_SSH_PUBLIC_KEY_BYTES
            || exchange_hash.is_empty()
            || exchange_hash.len() > MAX_SSH_EXCHANGE_HASH_BYTES
        {
            return Err(RemoteAuthError::InvalidSshProof)
        }
        let record = self
            .database()
            .login_record_for_ssh(username, login_node, public_key)
            .ok_or(RemoteAuthError::Authentication(AuthError::CredentialNotFound))?;
        let credential = record
            .ssh_key_credential(public_key)
            .ok_or(RemoteAuthError::Authentication(AuthError::CredentialNotFound))?;
        let authentication = self
            .begin_authentication_for_kind(
                username,
                login_node,
                credential,
                Some(CredentialKind::SshKey),
                now_us,
                policy.challenge_lifetime_us,
            )
            .map_err(RemoteAuthError::Authentication)?;
        SshAuthenticationChallenge::new(
            authentication,
            device,
            public_key,
            exchange_hash,
            policy,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn complete_ssh_login<V: SshSignatureVerifier>(
        &mut self,
        challenge: SshAuthenticationChallenge,
        signature: &[u8],
        verifier: &mut V,
        policy: SshLoginPolicy,
        login_address_space: AddressSpaceId,
        now_us: u64,
    ) -> Result<RemoteAdminSession, RemoteAuthError> {
        if challenge.challenge_lifetime_us != policy.challenge_lifetime_us
            || challenge.session_lifetime_us != policy.session_lifetime_us
        {
            return Err(RemoteAuthError::PolicyMismatch)
        }
        if signature.is_empty() || signature.len() > MAX_SSH_SIGNATURE_BYTES {
            return Err(RemoteAuthError::InvalidSshProof)
        }
        let mut adapter = SshAdapter {
            challenge,
            signature,
            verifier,
        };
        let session = self
            .complete_authentication(
                challenge.authentication,
                signature,
                &mut adapter,
                login_address_space,
                now_us,
                policy.session_lifetime_us,
            )
            .map_err(RemoteAuthError::Authentication)?;
        Ok(RemoteAdminSession {
            session,
            device: challenge.device,
            credential: challenge.authentication.credential,
            authenticated_at_us: now_us,
        })
    }
}

struct SshAdapter<'a, V> {
    challenge: SshAuthenticationChallenge,
    signature: &'a [u8],
    verifier: &'a mut V,
}

impl<V: SshSignatureVerifier> CredentialVerifier for SshAdapter<'_, V> {
    fn verify(
        &mut self,
        kind: CredentialKind,
        public_material: &[u8],
        challenge: &[u8],
        response: &[u8],
    ) -> bool {
        if kind != CredentialKind::SshKey
            || public_material != self.challenge.public_key()
            || challenge != self.challenge.authentication.bytes()
            || response != self.signature
        {
            return false
        }
        self.verifier.verify(SshSignatureVerificationRequest {
            public_key: self.challenge.public_key(),
            exchange_hash: self.challenge.exchange_hash(),
            challenge: &self.challenge.bytes(),
            signature: self.signature,
        })
    }
}

struct WebAuthnAdapter<'a, V> {
    challenge: RemoteAuthenticationChallenge,
    assertion: WebAuthnAssertion<'a>,
    verifier: &'a mut V,
    policy: WebAuthnPolicy,
    stored_sign_count: u32,
    sign_count: Option<u32>,
}

impl<V: WebAuthnVerifier> CredentialVerifier for WebAuthnAdapter<'_, V> {
    fn verify(
        &mut self,
        kind: CredentialKind,
        public_material: &[u8],
        challenge: &[u8],
        response: &[u8],
    ) -> bool {
        if kind != CredentialKind::Passkey
            || challenge != self.challenge.authentication.bytes()
            || response != self.assertion.signature
        {
            return false
        }
        let authenticator = self.assertion.authenticator_data;
        if authenticator.len() < AUTHENTICATOR_DATA_HEADER_BYTES
            || authenticator[0..32] != self.policy.rp_id_hash
        {
            return false
        }
        let flags = authenticator[32];
        if flags & FLAG_USER_PRESENT == 0 || flags & FLAG_USER_VERIFIED == 0 {
            return false
        }
        let verification = match self.verifier.verify(WebAuthnVerificationRequest {
            cose_public_key: public_material,
            expected_challenge: &self.challenge.bytes(),
            expected_origin_hash: self.policy.origin_hash,
            authenticator_data: authenticator,
            client_data_json: self.assertion.client_data_json,
            signature: self.assertion.signature,
        }) {
            Ok(verification) => verification,
            Err(_) => return false,
        };
        if !verification.signature_valid
            || !verification.challenge_bound
            || !verification.ceremony_is_get
            || verification.origin_hash != self.policy.origin_hash
        {
            return false
        }
        let sign_count = u32::from_be_bytes([
            authenticator[33],
            authenticator[34],
            authenticator[35],
            authenticator[36],
        ]);
        if self.stored_sign_count != 0
            && sign_count != 0
            && sign_count <= self.stored_sign_count
        {
            return false
        }
        self.sign_count = Some(sign_count);
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteCapabilityScope {
    resource: u64,
    rights: Rights,
    max_lifetime_us: u64,
}

impl RemoteCapabilityScope {
    pub const fn new(
        resource: u64,
        rights: Rights,
        max_lifetime_us: u64,
    ) -> Result<Self, RemoteTokenError> {
        if resource == 0
            || rights.is_empty()
            || !remote_safe_rights().contains(rights)
            || max_lifetime_us == 0
            || max_lifetime_us > MAX_REMOTE_TOKEN_LIFETIME_US
        {
            return Err(RemoteTokenError::InvalidScope)
        }
        Ok(Self {
            resource,
            rights,
            max_lifetime_us,
        })
    }

    pub const fn resource(self) -> u64 {
        self.resource
    }

    pub const fn rights(self) -> Rights {
        self.rights
    }

    pub const fn max_lifetime_us(self) -> u64 {
        self.max_lifetime_us
    }
}

/// Issues non-transferable, Layer-2-only tokens for explicitly allowlisted
/// frontend resources. No request can choose its subject, transport, resource
/// class, revocation epoch, or upper lifetime bound.
pub struct RemoteTokenIssuer<const SCOPES: usize = DEFAULT_REMOTE_SCOPE_CAPACITY> {
    issuer: NodeId,
    key: CapabilityKey,
    scopes: [Option<RemoteCapabilityScope>; SCOPES],
    grants: [Option<RemoteGrant>; SCOPES],
    revocation_epoch: u64,
    next_nonce: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RemoteGrant {
    nonce: u64,
    resource: u64,
    identity: IdentityId,
    device: NodeId,
    credential: CredentialId,
    authenticated_at_us: u64,
    expires_at_us: u64,
}

impl<const SCOPES: usize> RemoteTokenIssuer<SCOPES> {
    pub const fn new(issuer: NodeId, key: CapabilityKey, boot_nonce: u64) -> Self {
        Self {
            issuer,
            key,
            scopes: [None; SCOPES],
            grants: [None; SCOPES],
            revocation_epoch: 1,
            next_nonce: boot_nonce,
        }
    }

    pub fn allow_scope(&mut self, scope: RemoteCapabilityScope) -> Result<(), RemoteTokenError> {
        if self
            .scopes
            .iter()
            .flatten()
            .any(|existing| existing.resource == scope.resource)
        {
            return Err(RemoteTokenError::AlreadyExists)
        }
        let slot = self
            .scopes
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(RemoteTokenError::Capacity)?;
        *slot = Some(scope);
        Ok(())
    }

    pub fn disallow_scope(&mut self, resource: u64) -> Result<(), RemoteTokenError> {
        let slot = self
            .scopes
            .iter_mut()
            .find(|entry| entry.is_some_and(|scope| scope.resource == resource))
            .ok_or(RemoteTokenError::ScopeNotFound)?;
        *slot = None;
        self.revoke_all();
        Ok(())
    }

    pub fn issue(
        &mut self,
        session: &RemoteAdminSession,
        resource: u64,
        rights: Rights,
        lifetime_us: u64,
        now_us: u64,
    ) -> Result<CryptographicCapability, RemoteTokenError> {
        if now_us >= session.expires_at_us() {
            return Err(RemoteTokenError::SessionExpired)
        }
        let scope = self.scope(resource)?;
        if rights.is_empty()
            || !scope.rights.contains(rights)
            || lifetime_us == 0
            || lifetime_us > scope.max_lifetime_us
        {
            return Err(RemoteTokenError::ScopeViolation)
        }
        let expires_at_us = now_us
            .checked_add(lifetime_us)
            .ok_or(RemoteTokenError::ScopeViolation)?;
        if expires_at_us > session.expires_at_us() {
            return Err(RemoteTokenError::ScopeViolation)
        }
        let grant_slot = self
            .grants
            .iter()
            .position(|entry| {
                entry.is_none_or(|grant| {
                    grant.expires_at_us <= now_us || grant.resource == resource
                })
            })
            .ok_or(RemoteTokenError::Capacity)?;
        self.next_nonce = self.next_nonce.wrapping_add(1).max(1);
        let token = CryptographicCapability::issue(
            self.key,
            self.issuer,
            session.device(),
            resource,
            rights,
            TransportRights::LAYER2,
            now_us,
            expires_at_us,
            self.revocation_epoch,
            self.next_nonce,
        )
        .map_err(RemoteTokenError::Token)?;
        let token = token
            .attenuate(CapabilityCaveat {
                subject: Some(session.device()),
                rights,
                transports: TransportRights::LAYER2,
                expires_at_us,
            })
            .map_err(RemoteTokenError::Token)?;
        self.grants[grant_slot] = Some(RemoteGrant {
            nonce: token.nonce,
            resource,
            identity: session.identity(),
            device: session.device(),
            credential: session.credential(),
            authenticated_at_us: session.authenticated_at_us(),
            expires_at_us,
        });
        Ok(token)
    }

    pub fn authorize(
        &self,
        token: &CryptographicCapability,
        session: &RemoteAdminSession,
        required: Rights,
        now_us: u64,
    ) -> Result<(), RemoteTokenError> {
        if now_us >= session.expires_at_us()
            || token.issuer != self.issuer
            || token.resource == 0
            || token.effective_subject() != session.device()
            || !token.subject_is_sealed()
        {
            return Err(RemoteTokenError::AccessDenied)
        }
        let grant = self
            .grants
            .iter()
            .flatten()
            .find(|grant| grant.nonce == token.nonce && grant.resource == token.resource)
            .ok_or(RemoteTokenError::AccessDenied)?;
        if grant.identity != session.identity()
            || grant.device != session.device()
            || grant.credential != session.credential()
            || grant.authenticated_at_us != session.authenticated_at_us()
            || grant.expires_at_us != token.effective_expiry()
        {
            return Err(RemoteTokenError::AccessDenied)
        }
        let scope = self.scope(token.resource)?;
        let scope_deadline = token
            .not_before_us
            .checked_add(scope.max_lifetime_us)
            .ok_or(RemoteTokenError::AccessDenied)?;
        if !scope.rights.contains(required)
            || !scope.rights.contains(token.effective_rights())
            || !remote_safe_rights().contains(token.effective_rights())
            || token.effective_transports() != TransportRights::LAYER2
            || token.not_before_us < session.authenticated_at_us()
            || token.effective_expiry() > session.expires_at_us()
            || token.effective_expiry() > scope_deadline
        {
            return Err(RemoteTokenError::AccessDenied)
        }
        token
            .verify(
                self.key,
                session.device(),
                required,
                TransportRights::LAYER2,
                now_us,
                self.revocation_epoch,
            )
            .map_err(RemoteTokenError::Token)
    }

    pub fn revoke_all(&mut self) {
        self.revocation_epoch = self.revocation_epoch.wrapping_add(1).max(1);
        self.grants.fill(None)
    }

    pub const fn revocation_epoch(&self) -> u64 {
        self.revocation_epoch
    }

    fn scope(&self, resource: u64) -> Result<RemoteCapabilityScope, RemoteTokenError> {
        self.scopes
            .iter()
            .flatten()
            .find(|scope| scope.resource == resource)
            .copied()
            .ok_or(RemoteTokenError::ScopeNotFound)
    }
}

pub const fn remote_safe_rights() -> Rights {
    Rights::READ
        .union(Rights::WRITE)
        .union(Rights::EXECUTE)
        .union(Rights::SEND)
        .union(Rights::RECEIVE)
        .union(Rights::DEBUG)
}

const fn all_zero(bytes: &[u8; 32]) -> bool {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0 {
            return false
        }
        index += 1
    }
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteAuthError {
    Authentication(AuthError),
    InvalidAssertion,
    InvalidChallengeEntropy,
    InvalidPolicy,
    InvalidRemoteChallenge,
    InvalidSshProof,
    PolicyMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteTokenError {
    AccessDenied,
    AlreadyExists,
    Capacity,
    InvalidScope,
    ScopeNotFound,
    ScopeViolation,
    SessionExpired,
    Token(TokenError),
}

/// Trusted composition root for the configured WebAuthn policy, local auth
/// database, and remote token allowlist.
pub struct RemoteSecurityGateway<
    const USERS: usize,
    const CHALLENGES: usize,
    const SCOPES: usize = DEFAULT_REMOTE_SCOPE_CAPACITY,
> {
    authd: AuthDaemon<USERS, CHALLENGES>,
    policy: WebAuthnPolicy,
    tokens: RemoteTokenIssuer<SCOPES>,
    pending: [Option<RemoteAuthenticationChallenge>; CHALLENGES],
    ssh_pending: [Option<SshAuthenticationChallenge>; CHALLENGES],
}

impl<const USERS: usize, const CHALLENGES: usize, const SCOPES: usize>
    RemoteSecurityGateway<USERS, CHALLENGES, SCOPES>
{
    pub const fn new(
        authd: AuthDaemon<USERS, CHALLENGES>,
        policy: WebAuthnPolicy,
        tokens: RemoteTokenIssuer<SCOPES>,
    ) -> Self {
        Self {
            authd,
            policy,
            tokens,
            pending: [None; CHALLENGES],
            ssh_pending: [None; CHALLENGES],
        }
    }

    pub fn begin_administration(
        &mut self,
        username: &str,
        login_node: NodeId,
        device: NodeId,
        credential: CredentialId,
        ceremony_nonce: [u8; 32],
        now_us: u64,
    ) -> Result<RemoteAuthenticationChallenge, RemoteAuthError> {
        self.pending
            .iter_mut()
            .filter(|entry| {
                entry.is_some_and(|challenge| {
                    challenge.authentication.expires_at_us <= now_us
                })
            })
            .for_each(|entry| *entry = None);
        let slot = self
            .pending
            .iter()
            .position(Option::is_none)
            .ok_or(RemoteAuthError::Authentication(AuthError::Capacity))?;
        let challenge = self.authd.begin_remote_administration(
            username,
            login_node,
            device,
            credential,
            ceremony_nonce,
            self.policy,
            now_us,
        )?;
        self.pending[slot] = Some(challenge);
        Ok(challenge)
    }

    pub fn complete_administration<V: WebAuthnVerifier>(
        &mut self,
        challenge: RemoteAuthenticationChallenge,
        assertion: WebAuthnAssertion<'_>,
        verifier: &mut V,
        login_address_space: AddressSpaceId,
        now_us: u64,
    ) -> Result<RemoteAdminSession, RemoteAuthError> {
        let slot = self
            .pending
            .iter()
            .position(|entry| entry.is_some_and(|pending| pending == challenge))
            .ok_or(RemoteAuthError::InvalidRemoteChallenge)?;
        self.pending[slot] = None;
        self.authd.complete_remote_administration(
            challenge,
            assertion,
            verifier,
            self.policy,
            login_address_space,
            now_us,
        )
    }

    pub fn begin_ssh_login(
        &mut self,
        username: &str,
        login_node: NodeId,
        device: NodeId,
        public_key: &[u8],
        exchange_hash: &[u8],
        policy: SshLoginPolicy,
        now_us: u64,
    ) -> Result<SshAuthenticationChallenge, RemoteAuthError> {
        self.ssh_pending
            .iter_mut()
            .filter(|entry| {
                entry.is_some_and(|challenge| {
                    challenge.authentication.expires_at_us <= now_us
                })
            })
            .for_each(|entry| *entry = None);
        let slot = self
            .ssh_pending
            .iter()
            .position(Option::is_none)
            .ok_or(RemoteAuthError::Authentication(AuthError::Capacity))?;
        let challenge = self.authd.begin_ssh_login(
            username,
            login_node,
            device,
            public_key,
            exchange_hash,
            policy,
            now_us,
        )?;
        self.ssh_pending[slot] = Some(challenge);
        Ok(challenge)
    }

    pub fn complete_ssh_login<V: SshSignatureVerifier>(
        &mut self,
        challenge: SshAuthenticationChallenge,
        signature: &[u8],
        verifier: &mut V,
        policy: SshLoginPolicy,
        login_address_space: AddressSpaceId,
        now_us: u64,
    ) -> Result<RemoteAdminSession, RemoteAuthError> {
        let slot = self
            .ssh_pending
            .iter()
            .position(|entry| entry.is_some_and(|pending| pending == challenge))
            .ok_or(RemoteAuthError::InvalidRemoteChallenge)?;
        self.ssh_pending[slot] = None;
        self.authd.complete_ssh_login(
            challenge,
            signature,
            verifier,
            policy,
            login_address_space,
            now_us,
        )
    }

    pub fn issue_token(
        &mut self,
        session: &RemoteAdminSession,
        resource: u64,
        rights: Rights,
        lifetime_us: u64,
        now_us: u64,
    ) -> Result<CryptographicCapability, RemoteTokenError> {
        self.tokens
            .issue(session, resource, rights, lifetime_us, now_us)
    }

    pub fn authorize(
        &self,
        token: &CryptographicCapability,
        session: &RemoteAdminSession,
        required: Rights,
        now_us: u64,
    ) -> Result<(), RemoteTokenError> {
        self.tokens.authorize(token, session, required, now_us)
    }

    pub const fn authd(&self) -> &AuthDaemon<USERS, CHALLENGES> {
        &self.authd
    }

    pub fn authd_mut(&mut self) -> &mut AuthDaemon<USERS, CHALLENGES> {
        &mut self.authd
    }

    pub const fn token_issuer(&self) -> &RemoteTokenIssuer<SCOPES> {
        &self.tokens
    }

    pub fn token_issuer_mut(&mut self) -> &mut RemoteTokenIssuer<SCOPES> {
        &mut self.tokens
    }
}
