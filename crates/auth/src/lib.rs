#![no_std]
#![deny(unsafe_code)]

pub mod identity;
pub mod federation;
pub mod lending;
pub mod logical;
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
pub use token::{
    CapabilityCaveat, CapabilityKey, CryptographicCapability, TokenError, TransportRights,
};
