// Inventory: coverage_59_10.rs (legacy roadmap section 59).
use ghostos_confidential::{
    CapabilityRights, EncryptedDsmFrame, EnclaveManager, EnclavePlatform, Error, FabricTransport,
    MlKemKeypair, NonceReplayGuard,
};
use ghostos_fabric::{AddressRange, NodeId, PAGE_SIZE};
use ghostos_fabric::dsm::{DsmHeader, DsmPacket, MessageKind};
use ghostos_ipc::InheritableDescriptor;
use ghostos_shield::attestation::{AttestationKey, AttestationQuote, HardwareRoot};

fn packet() -> DsmPacket {
    DsmPacket::new(DsmHeader {
        kind: MessageKind::PageData,
        source: NodeId::LOCAL,
        destination: NodeId::new(2).unwrap(),
        sequence: 4,
        page_address: PAGE_SIZE,
        lease_epoch: 1,
        fragment: 0,
        fragment_count: 1,
    }, b"secret page").unwrap()
}

#[test]
fn enclave_admission_gates_capability_provisioning_and_revoke_all() {
    let node = NodeId::new(2).unwrap();
    let measurement = [7; 32];
    let key = AttestationKey::new([9; 32]);
    let range = AddressRange::new(PAGE_SIZE * 4, PAGE_SIZE).unwrap();
    let mut manager = EnclaveManager::<2, 2, 2>::new(55).unwrap();
    manager.register(node, EnclavePlatform::AmdSevSnp, measurement, key, &[range]).unwrap();
    assert_eq!(manager.provision_dsm(node, 10, range, CapabilityRights::READ, 100, 1), Err(Error::NotAdmitted));

    let nonce = [3; 32];
    manager.issue_challenge(node, nonce, 50).unwrap();
    let quote = AttestationQuote::new(node, HardwareRoot::AmdSevSnp, 2, nonce, measurement, key).unwrap();
    manager.admit(quote, 3).unwrap();
    assert!(manager.is_admitted(node));
    let capability = manager.provision_dsm(node, 10, range, CapabilityRights::READ, 100, 3).unwrap();
    manager.validate(capability, 10, CapabilityRights::READ, 4).unwrap();
    assert_eq!(manager.validate(capability, 11, CapabilityRights::READ, 4), Err(Error::Unauthorized));
    assert_eq!(manager.provision_dsm(node, 10, AddressRange::new(PAGE_SIZE * 8, PAGE_SIZE).unwrap(), CapabilityRights::READ, 100, 3), Err(Error::NotProtected));
    manager.revoke_all();
    assert_eq!(manager.validate(capability, 10, CapabilityRights::READ, 4), Err(Error::Unauthorized));

    let ipc = manager.provision_ipc(node, 10, InheritableDescriptor::channel(2).unwrap(), CapabilityRights::SEND, 100, 3).unwrap();
    assert_eq!(ipc.rights(), CapabilityRights::SEND);
}

#[test]
fn confidential_frames_round_trip_authenticate_and_block_replay() {
    let keypair = MlKemKeypair::from_seed([5; 32]);
    let nonce = [4; 16];
    let frame = EncryptedDsmFrame::seal(&packet(), FabricTransport::Ethernet, keypair.public_key(), [8; 32], nonce).unwrap();
    let opened = frame.open(keypair.secret_key()).unwrap();
    assert_eq!(opened.header, packet().header);
    assert_eq!(opened.payload(), b"secret page");

    let mut encoded = vec![0; ghostos_confidential::fabric::ENCRYPTED_FRAME_BYTES];
    let length = frame.encode(&mut encoded).unwrap();
    let decoded = EncryptedDsmFrame::decode(&encoded[..length]).unwrap();
    assert_eq!(decoded.open(keypair.secret_key()).unwrap().payload(), b"secret page");
    encoded[length - 1] ^= 1;
    assert!(matches!(EncryptedDsmFrame::decode(&encoded[..length]).unwrap().open(keypair.secret_key()), Err(Error::AuthenticationFailed)));

    let mut guard = NonceReplayGuard::<2>::new();
    guard.accept(&frame).unwrap();
    assert_eq!(guard.accept(&frame), Err(Error::Replay));
}
