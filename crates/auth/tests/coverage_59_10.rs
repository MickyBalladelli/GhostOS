use synos_auth::{
    AccountState, AuthenticationChallenge, BootLoginService, CapabilityCaveat, CapabilityKey,
    Credential, CredentialId, CredentialKind, CredentialVerifier, CryptographicCapability,
    DatabaseScope, DiscoveryAnnouncement, FederationError, InitialCapability, PeerDirectory,
    SecurityPolicy, SecurityState, SecurityStore, SecurityStoreError, SessionManager, StartupError,
    TokenError, TransportRights, UserRecord, Username, AuthDaemon, ClusterId,
};
use synos_fabric::NodeId;
use synos_kernel::{AddressSpaceId, CapabilityObject, CapabilitySpace, IdentityId, Rights};

fn record() -> UserRecord {
    let identity = IdentityId::new(7).unwrap();
    let mut record = UserRecord::new(identity, Username::new("Alice").unwrap(), DatabaseScope::Local);
    record
        .add_credential(Credential::new_passkey(CredentialId::new(1).unwrap(), b"public-key", 0).unwrap())
        .unwrap();
    record.assign_right(synos_kernel::RightIdentifier::NETWORK_INBOUND).unwrap();
    record
        .grant_initial_capability(InitialCapability {
            object: CapabilityObject::SystemControl,
            rights: Rights::CONTROL,
        })
        .unwrap();
    record
}

struct Verifier {
    accepts: bool,
}

#[derive(Default)]
struct MemorySecurityStore<const USERS: usize, const GROUPS: usize> {
    state: Option<SecurityState<USERS, GROUPS>>,
}

impl<const USERS: usize, const GROUPS: usize> SecurityStore<USERS, GROUPS>
    for MemorySecurityStore<USERS, GROUPS>
{
    fn load(
        &mut self,
        state: &mut SecurityState<USERS, GROUPS>,
    ) -> Result<bool, SecurityStoreError> {
        if let Some(saved) = self.state {
            *state = saved;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn store(
        &mut self,
        state: &SecurityState<USERS, GROUPS>,
    ) -> Result<(), SecurityStoreError> {
        self.state = Some(*state);
        Ok(())
    }
}

fn passkey(id: u32) -> Credential {
    Credential::new_passkey(
        CredentialId::new(id).unwrap(),
        &[id as u8, 0x42, 0x99],
        0,
    )
    .unwrap()
}

fn management_service() -> (
    BootLoginService<4, 4, 4, 4>,
    MemorySecurityStore<4, 4>,
    synos_auth::SessionHandle,
) {
    let mut store = MemorySecurityStore::default();
    let mut service = BootLoginService::start(&mut store, SecurityPolicy::default(), 100).unwrap();
    let admin_identity = IdentityId::new(1).unwrap();
    service
        .create_first_admin_at(
            &mut store,
            admin_identity,
            "admin",
            DatabaseScope::Local,
            passkey(1),
            10,
        )
        .unwrap();
    let challenge = service
        .begin_login(
            "admin",
            NodeId::LOCAL,
            CredentialId::new(1).unwrap(),
            CredentialKind::Passkey,
            10,
        )
        .unwrap();
    let session = service
        .complete_login(
            challenge,
            b"proof",
            &mut Verifier { accepts: true },
            AddressSpaceId::new(4).unwrap(),
            10,
        )
        .unwrap();
    (service, store, session.handle)
}

impl CredentialVerifier for Verifier {
    fn verify(
        &mut self,
        _kind: CredentialKind,
        _public_material: &[u8],
        _challenge: &[u8],
        _response: &[u8],
    ) -> bool {
        self.accepts
    }
}

#[test]
fn authentication_challenges_are_one_shot_and_sessions_expire() {
    let mut database = synos_auth::AuthorizationDatabase::<2>::new();
    database.insert(record()).unwrap();
    let mut daemon = AuthDaemon::<2, 2>::new(database, 100);
    let login_space = AddressSpaceId::new(4).unwrap();
    let challenge = daemon
        .begin_authentication("alice", NodeId::LOCAL, CredentialId::new(1).unwrap(), 10, 20)
        .unwrap();
    assert_eq!(challenge.bytes().len(), 28);
    let session = daemon
        .complete_authentication(challenge, b"proof", &mut Verifier { accepts: true }, login_space, 15, 10)
        .unwrap();
    assert_eq!(session.identity(), IdentityId::new(7).unwrap());
    assert!(matches!(daemon.complete_authentication(challenge, b"proof", &mut Verifier { accepts: true }, login_space, 15, 10), Err(synos_auth::AuthError::InvalidChallenge)));

    let mut capabilities = CapabilitySpace::<2>::new();
    assert_eq!(session.instantiate_capabilities(&mut capabilities, 24).unwrap().handles().count(), 1);
    assert!(matches!(session.instantiate_capabilities(&mut capabilities, 25), Err(synos_auth::AuthError::SessionExpired)));

    let expired = daemon
        .begin_authentication("alice", NodeId::LOCAL, CredentialId::new(1).unwrap(), 30, 1)
        .unwrap();
    assert!(matches!(daemon.complete_authentication(expired, b"proof", &mut Verifier { accepts: true }, login_space, 31, 10), Err(synos_auth::AuthError::InvalidChallenge)));
}

#[test]
fn wrong_credentials_are_rejected_and_challenges_are_consumed() {
    let mut database = synos_auth::AuthorizationDatabase::<2>::new();
    database.insert(record()).unwrap();
    let mut daemon = AuthDaemon::<2, 2>::new(database, 100);
    let login_space = AddressSpaceId::new(4).unwrap();
    let challenge = daemon
        .begin_authentication("alice", NodeId::LOCAL, CredentialId::new(1).unwrap(), 10, 20)
        .unwrap();

    assert!(matches!(
        daemon.complete_authentication(
            challenge,
            b"wrong-proof",
            &mut Verifier { accepts: false },
            login_space,
            15,
            10,
        ),
        Err(synos_auth::AuthError::VerificationFailed)
    ));
    assert!(matches!(
        daemon.complete_authentication(
            challenge,
            b"proof",
            &mut Verifier { accepts: true },
            login_space,
            15,
            10,
        ),
        Err(synos_auth::AuthError::InvalidChallenge)
    ));
}

#[test]
fn account_management_creates_renames_disables_and_deletes_accounts() {
    let (mut service, mut store, administrator) = management_service();
    let identity = IdentityId::new(2).unwrap();
    let mut account = UserRecord::new(
        identity,
        Username::new("Alice").unwrap(),
        DatabaseScope::Local,
    );
    account.add_credential(passkey(2)).unwrap();

    let created = service
        .create_account(&mut store, administrator, account, 20)
        .unwrap();
    assert_eq!(created.identity, identity);
    assert_eq!(created.username.as_str(), "alice");
    let stored = service.state().database.record(identity).unwrap();
    assert_eq!(stored.identity, created.identity);
    assert_eq!(stored.username, created.username);

    let renamed = service
        .rename_account(
            &mut store,
            administrator,
            identity,
            Username::new("AliceRenamed").unwrap(),
            30,
        )
        .unwrap();
    assert_eq!(renamed.identity, identity);
    assert_eq!(renamed.username.as_str(), "alicerenamed");

    let disabled = service
        .disable_account(&mut store, administrator, identity, 40)
        .unwrap();
    assert_eq!(disabled.account_state(), AccountState::Disabled);
    assert!(!disabled.is_login_usable());

    let deleted = service
        .delete_account(&mut store, administrator, identity, 50)
        .unwrap();
    assert_eq!(deleted.identity, identity);
    assert!(matches!(
        service.state().database.record(identity),
        Err(synos_auth::AuthError::UserNotFound)
    ));
    assert!(store.state.is_some());
}

#[test]
fn last_administrator_cannot_be_disabled_or_deleted() {
    let (mut service, mut store, administrator) = management_service();
    let identity = IdentityId::new(1).unwrap();

    assert!(matches!(
        service.disable_account(&mut store, administrator, identity, 20),
        Err(StartupError::LastAdministrator)
    ));
    assert!(matches!(
        service.delete_account(&mut store, administrator, identity, 30),
        Err(StartupError::LastAdministrator)
    ));

    let record = service.state().database.record(identity).unwrap();
    assert_eq!(record.account_state(), AccountState::Active);
    assert!(record.has_role(synos_auth::AccountRole::Administrator));
}

#[test]
fn session_lifetime_idle_timeout_and_identity_revocation_fence_access() {
    let policy = SecurityPolicy::new(50, 20, 10).unwrap();
    let login_space = AddressSpaceId::new(4).unwrap();

    let mut lifetime_manager = session_manager(policy);
    let lifetime_session = complete_session(&mut lifetime_manager, login_space, 10);
    assert_eq!(lifetime_session.expires_at_us, 30);
    lifetime_manager
        .authorize(
            lifetime_session.handle,
            synos_kernel::RightIdentifier::NETWORK_INBOUND,
            19,
        )
        .unwrap();
    assert_eq!(
        lifetime_manager.authorize(
            lifetime_session.handle,
            synos_kernel::RightIdentifier::NETWORK_INBOUND,
            30,
        ),
        Err(StartupError::SessionExpired)
    );

    let mut idle_manager = session_manager(policy);
    let idle_session = complete_session(&mut idle_manager, login_space, 10);
    assert_eq!(
        idle_manager.authorize(
            idle_session.handle,
            synos_kernel::RightIdentifier::NETWORK_INBOUND,
            20,
        ),
        Err(StartupError::SessionExpired)
    );

    let mut revocation_manager = session_manager(policy);
    let revoked_session = complete_session(&mut revocation_manager, login_space, 10);
    assert_eq!(revocation_manager.revoke_identity(revoked_session.identity), 1);
    assert_eq!(
        revocation_manager.authorize(
            revoked_session.handle,
            synos_kernel::RightIdentifier::NETWORK_INBOUND,
            11,
        ),
        Err(StartupError::SessionNotFound)
    );
}

fn session_manager(policy: SecurityPolicy) -> SessionManager<2, 2, 2> {
    let mut database = synos_auth::AuthorizationDatabase::<2>::new();
    database.insert(record()).unwrap();
    SessionManager::new(database, policy, 100)
}

fn complete_session(
    manager: &mut SessionManager<2, 2, 2>,
    login_space: AddressSpaceId,
    now_us: u64,
) -> synos_auth::SessionView {
    let challenge = manager
        .begin_login(
            "alice",
            NodeId::LOCAL,
            CredentialId::new(1).unwrap(),
            CredentialKind::Passkey,
            now_us,
        )
        .unwrap();
    manager
        .complete_login(
            challenge,
            b"proof",
            &mut Verifier { accepts: true },
            login_space,
            now_us,
        )
        .unwrap()
}

#[test]
fn token_attenuation_encoding_expiry_and_revocation_are_enforced() {
    let key = CapabilityKey::new([3; 32]);
    let rights = Rights::READ.union(Rights::WRITE);
    let token = CryptographicCapability::issue(
        key,
        NodeId::LOCAL,
        NodeId::new(2).unwrap(),
        9,
        rights,
        TransportRights::ALL,
        10,
        100,
        4,
        7,
    )
    .unwrap()
    .attenuate(CapabilityCaveat {
        subject: Some(NodeId::new(2).unwrap()),
        rights: Rights::READ,
        transports: TransportRights::LAYER2,
        expires_at_us: 80,
    })
    .unwrap();
    token.verify(key, NodeId::new(2).unwrap(), Rights::READ, TransportRights::LAYER2, 20, 4).unwrap();
    assert_eq!(token.verify(key, NodeId::new(2).unwrap(), Rights::WRITE, TransportRights::LAYER2, 20, 4), Err(TokenError::AccessDenied));
    assert_eq!(token.verify(key, NodeId::new(3).unwrap(), Rights::READ, TransportRights::LAYER2, 20, 4), Err(TokenError::AccessDenied));
    assert_eq!(token.verify(key, NodeId::new(2).unwrap(), Rights::READ, TransportRights::LAYER2, 80, 4), Err(TokenError::AccessDenied));
    assert_eq!(token.verify(key, NodeId::new(2).unwrap(), Rights::READ, TransportRights::LAYER2, 20, 5), Err(TokenError::AccessDenied));

    let mut wire = token.encode();
    wire[191] ^= 1;
    let tampered = CryptographicCapability::decode(wire).unwrap();
    assert_eq!(tampered.verify(key, NodeId::new(2).unwrap(), Rights::READ, TransportRights::LAYER2, 20, 4), Err(TokenError::InvalidSignature));
    assert_eq!(token.attenuate(CapabilityCaveat { subject: Some(NodeId::new(3).unwrap()), rights: Rights::READ, transports: TransportRights::LAYER2, expires_at_us: 70 }), Err(TokenError::RightsEscalation));
}

#[test]
fn federation_announcements_reject_replay_and_bad_keys() {
    let key = CapabilityKey::new([8; 32]);
    let cluster = ClusterId::new(44).unwrap();
    let announcement = DiscoveryAnnouncement::issue(key, cluster, NodeId::new(2).unwrap(), 3, 10, 100, 9, TransportRights::LAYER2).unwrap();
    announcement.verify(key, 20).unwrap();
    let decoded = DiscoveryAnnouncement::decode(announcement.encode()).unwrap();
    decoded.verify(key, 20).unwrap();
    assert_eq!(decoded.verify(CapabilityKey::new([9; 32]), 20), Err(FederationError::Token(TokenError::InvalidSignature)));

    let mut peers = PeerDirectory::<2>::new();
    peers.observe(decoded, key, 20).unwrap();
    assert_eq!(peers.observe(decoded, key, 20), Err(FederationError::Replay));
    assert_eq!(peers.peer(cluster, 100), Err(FederationError::Expired));
    assert_eq!(peers.observe(DiscoveryAnnouncement::issue(key, cluster, NodeId::new(2).unwrap(), 4, 10, 100, 10, TransportRights::LAYER2).unwrap(), key, 20), Ok(()));
}

#[allow(dead_code)]
fn _challenge_is_wire_stable(challenge: AuthenticationChallenge) -> [u8; 28] {
    challenge.bytes()
}
