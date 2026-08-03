use synos_auth::{
    AuthenticationChallenge, CapabilityCaveat, CapabilityKey, Credential, CredentialId,
    CredentialKind, CredentialVerifier, CryptographicCapability, DatabaseScope, DiscoveryAnnouncement,
    FederationError, PeerDirectory, TokenError, TransportRights, UserRecord, Username,
    AuthDaemon, InitialCapability, ClusterId,
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
