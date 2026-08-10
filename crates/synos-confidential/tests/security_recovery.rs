use synos_confidential::{
    CapabilityRights, EncryptedDsmFrame, EnclaveManager, EnclavePlatform, Error, FabricTransport,
    MlKemKeypair, NonceReplayGuard,
};
use synos_fabric::{AddressRange, NodeId, PAGE_SIZE};
use synos_fabric::dsm::{DsmHeader, DsmPacket, MessageKind};
use synos_shield::attestation::{AttestationKey, AttestationQuote, HardwareRoot};
use synos_shield::Error as ShieldError;

fn packet(sequence: u32) -> DsmPacket {
    DsmPacket::new(
        DsmHeader {
            kind: MessageKind::PageData,
            source: NodeId::LOCAL,
            destination: NodeId::new(2).unwrap(),
            sequence,
            page_address: PAGE_SIZE,
            lease_epoch: 1,
            fragment: 0,
            fragment_count: 1,
        },
        b"evidence",
    )
    .unwrap()
}

#[test]
fn evidence_key_rotation_revocation_replay_downgrade_and_root_recovery_are_fenced() {
    let node = NodeId::new(2).unwrap();
    let range = AddressRange::new(PAGE_SIZE * 4, PAGE_SIZE).unwrap();
    let old_key = AttestationKey::new([81; 32]);
    let new_key = AttestationKey::new([82; 32]);
    let measurement = [83; 32];
    let mut manager = EnclaveManager::<2, 2, 4>::new(55).unwrap();
    manager
        .register(
            node,
            EnclavePlatform::AmdSevSnp,
            measurement,
            old_key,
            &[range],
        )
        .unwrap();
    manager.issue_challenge(node, [84; 32], 50).unwrap();
    let old_quote = AttestationQuote::new(
        node,
        HardwareRoot::AmdSevSnp,
        2,
        [84; 32],
        measurement,
        old_key,
    )
    .unwrap();
    manager.admit(old_quote, 3).unwrap();
    let capability = manager
        .provision_dsm(node, 10, range, CapabilityRights::READ, 100, 3)
        .unwrap();

    manager.rotate_attestation_key(node, new_key).unwrap();
    assert_eq!(
        manager.validate(capability, 10, CapabilityRights::READ, 4),
        Err(Error::Unauthorized)
    );
    manager.issue_challenge(node, [85; 32], 50).unwrap();
    let new_quote = AttestationQuote::new(
        node,
        HardwareRoot::AmdSevSnp,
        4,
        [85; 32],
        measurement,
        new_key,
    )
    .unwrap();
    let stale_quote = AttestationQuote::new(
        node,
        HardwareRoot::AmdSevSnp,
        4,
        [85; 32],
        measurement,
        old_key,
    )
    .unwrap();
    assert_eq!(
        manager.admit(stale_quote, 5),
        Err(Error::Attestation(ShieldError::SignatureMismatch))
    );
    manager.admit(new_quote, 5).unwrap();
    assert_eq!(
        manager.admit(new_quote, 5),
        Err(Error::Attestation(ShieldError::Unauthorized))
    );

    manager.revoke(node).unwrap();
    assert_eq!(manager.admit(new_quote, 6), Err(Error::Unauthorized));
    assert_eq!(manager.revoke(node), Err(Error::NotFound));

    let old_pair = MlKemKeypair::from_seed([86; 32]);
    let new_pair = MlKemKeypair::from_seed([87; 32]);
    let old_frame = EncryptedDsmFrame::seal(
        &packet(9),
        FabricTransport::Ethernet,
        old_pair.public_key(),
        [88; 32],
        [89; 16],
    )
    .unwrap();
    assert!(matches!(
        old_frame.open(new_pair.secret_key()),
        Err(Error::AuthenticationFailed)
    ));
    assert_eq!(old_frame.open(old_pair.secret_key()).unwrap().header.sequence, 9);
    let mut replay = NonceReplayGuard::<2>::new();
    replay.accept(&old_frame).unwrap();
    assert_eq!(replay.accept(&old_frame), Err(Error::Replay));

    let new_frame = EncryptedDsmFrame::seal(
        &packet(1),
        FabricTransport::Ethernet,
        new_pair.public_key(),
        [90; 32],
        [91; 16],
    )
    .unwrap();
    let mut encoded = vec![0; synos_confidential::fabric::ENCRYPTED_FRAME_BYTES];
    let length = new_frame.encode(&mut encoded).unwrap();
    encoded[4] = 0;
    assert!(matches!(
        EncryptedDsmFrame::decode(&encoded[..length]),
        Err(Error::InvalidInput)
    ));
    assert_eq!(new_frame.open(new_pair.secret_key()).unwrap().header.sequence, 1);
}
