#![no_std]
#![deny(unsafe_code)]

pub mod identity;
pub mod federation;
pub mod federation_control;
pub mod lending;
pub mod lease;
pub mod logical;
pub mod remote;
pub mod revocation_monitor;
pub mod startup;
pub mod token;

pub use identity::{
    AuthDaemon, AuthError, AuthenticationChallenge, AuthorizationDatabase, AuthorizationStore,
    AccountState, Credential, CredentialId, CredentialKind, CredentialVerifier, DatabaseScope,
    InitialCapability, PublicCredentialData, Session, SessionCapabilities, UserRecord, Username,
    MAX_CREDENTIAL_LABEL_BYTES, MAX_USERNAME_BYTES, RESERVED_USERNAMES,
};
pub use federation::{
    accept_offer, ClusterId, DiscoveryAnnouncement, FederatedLease, FederatedResourceKind,
    FederatedResourceOffer, FederationError, PeerDirectory, RevocationReason, RevocationSignal,
    MAX_FEDERATED_PEERS, MAX_REVOCATION_LATENCY_US,
};
pub use federation_control::{
    FederationCapability, FederationHealth, FederationInvitation, FederationLeaseStatus,
    FederationRecord, FederationRegistry, FederationScope, FederationState, LeaseOwner,
    MAX_FEDERATION_INVITATIONS, MAX_FEDERATION_RECORDS,
};
pub use lending::{
    LendingError, LendingKind, LendingRights, ResourceLender, RevocationAction,
};
pub use lease::{CapabilityLease, LeaseContext, LeaseError, LeaseReplayGuard};
pub use logical::{CapabilityLogicalNames, LogicalNamespace};
pub use remote::{
    DEFAULT_REMOTE_SCOPE_CAPACITY, MAX_REMOTE_CHALLENGE_LIFETIME_US,
    MAX_REMOTE_SESSION_LIFETIME_US, MAX_REMOTE_TOKEN_LIFETIME_US,
    MAX_WEBAUTHN_AUTHENTICATOR_DATA_BYTES, MAX_WEBAUTHN_CLIENT_DATA_BYTES,
    MAX_WEBAUTHN_SIGNATURE_BYTES, RemoteAdminSession, RemoteAuthError,
    RemoteAuthenticationChallenge, RemoteCapabilityScope, RemoteSecurityGateway,
    RemoteTokenError, RemoteTokenIssuer, WebAuthnAssertion, WebAuthnPolicy,
    WebAuthnVerification, WebAuthnVerificationRequest, WebAuthnVerifier, SshLoginPolicy,
    SshAuthenticationChallenge, SshSignatureVerificationRequest, SshSignatureVerifier,
    MAX_SSH_EXCHANGE_HASH_BYTES, MAX_SSH_PUBLIC_KEY_BYTES, MAX_SSH_SIGNATURE_BYTES,
    remote_safe_rights,
};
pub use revocation_monitor::{
    CacheReport, PropagationObservation, PropagationReport, RevocationCache, RevocationKey,
    RevocationMonitor, RevocationMonitorError, RevocationNotice,
    DEFAULT_REVOCATION_EVENT_CAPACITY, REVOCATION_CACHE_COUNT,
};
pub use startup::{
    AccountManagementRequest, AccountManagementResult, BootLoginService, GroupDirectory, GroupId,
    GroupRecord, SecurityPolicy, SecurityState, SecurityStore, SecurityStoreError, SessionHandle,
    SessionManager, SessionView, StartupError,
    MAX_ACTIVE_SESSIONS, MAX_CHALLENGE_LIFETIME_US, MAX_GROUP_MEMBERS, MAX_GROUP_RIGHTS,
    MAX_GROUPS, MAX_SESSION_LIFETIME_US,
};
pub use token::{
    CapabilityCaveat, CapabilityKey, CryptographicCapability, TokenError, TransportRights,
};

pub use synos_policy::{
    AffectedObject, AffectedPrincipal, Binding as PolicyBinding, CapabilityChange,
    ChangeKind as PolicyChangeKind, ObjectId as PolicyObjectId, ObjectKind as PolicyObjectKind,
    ObjectRecord as PolicyObjectRecord, PolicyChange, PolicySnapshot, PrincipalId as PolicyPrincipalId,
    SimulationError as PolicySimulationError, SimulationReport,
};

pub fn simulate_capability_change<const PRINCIPALS: usize, const OBJECTS: usize, const BINDINGS: usize>(
    snapshot: &PolicySnapshot<PRINCIPALS, OBJECTS, BINDINGS>,
    change: CapabilityChange,
) -> Result<SimulationReport, synos_policy::SimulationError> {
    snapshot.simulate(PolicyChange::Capability(change))
}
