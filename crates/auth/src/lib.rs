#![no_std]
#![deny(unsafe_code)]

pub mod identity;
pub mod federation;
pub mod lending;
pub mod logical;
pub mod remote;
pub mod token;

pub use identity::{
    AuthDaemon, AuthError, AuthenticationChallenge, AuthorizationDatabase, AuthorizationStore,
    Credential, CredentialId, CredentialKind, CredentialVerifier, DatabaseScope,
    InitialCapability, Session, SessionCapabilities, UserRecord, Username,
};
pub use federation::{
    accept_offer, ClusterId, DiscoveryAnnouncement, FederatedLease, FederatedResourceKind,
    FederatedResourceOffer, FederationError, PeerDirectory, RevocationReason, RevocationSignal,
    MAX_FEDERATED_PEERS, MAX_REVOCATION_LATENCY_US,
};
pub use lending::{
    LendingError, LendingKind, LendingRights, ResourceLender, RevocationAction,
};
pub use logical::{CapabilityLogicalNames, LogicalNamespace};
pub use remote::{
    DEFAULT_REMOTE_SCOPE_CAPACITY, MAX_REMOTE_CHALLENGE_LIFETIME_US,
    MAX_REMOTE_SESSION_LIFETIME_US, MAX_REMOTE_TOKEN_LIFETIME_US,
    MAX_WEBAUTHN_AUTHENTICATOR_DATA_BYTES, MAX_WEBAUTHN_CLIENT_DATA_BYTES,
    MAX_WEBAUTHN_SIGNATURE_BYTES, RemoteAdminSession, RemoteAuthError,
    RemoteAuthenticationChallenge, RemoteCapabilityScope, RemoteSecurityGateway,
    RemoteTokenError, RemoteTokenIssuer, WebAuthnAssertion, WebAuthnPolicy,
    WebAuthnVerification, WebAuthnVerificationRequest, WebAuthnVerifier, remote_safe_rights,
};
pub use token::{
    CapabilityCaveat, CapabilityKey, CryptographicCapability, TokenError, TransportRights,
};
