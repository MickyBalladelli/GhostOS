use ghostos_fabric::{AddressRange, NodeId, PAGE_SIZE};
use ghostos_fabric::dsm::{DsmHeader, DsmPacket, MessageKind};
use ghostos_shield::Error;
use ghostos_shield::fabric::{FrameKey, FrameVerifier, SignedFrame};

fn frame(key: FrameKey, sequence: u32) -> SignedFrame {
    let packet = DsmPacket::new(
        DsmHeader {
            kind: MessageKind::PageRequest,
            source: NodeId::LOCAL,
            destination: NodeId::new(2).unwrap(),
            sequence,
            page_address: PAGE_SIZE,
            lease_epoch: 1,
            fragment: 0,
            fragment_count: 1,
        },
        b"membership",
    )
    .unwrap();
    SignedFrame::new(packet, key).unwrap()
}

#[test]
fn cluster_key_rotation_revocation_replay_rollback_and_root_recovery_are_fenced() {
    let old_key = FrameKey::new([71; 32]);
    let new_key = FrameKey::new([72; 32]);
    let destination = NodeId::new(2).unwrap();
    let allowed = AddressRange::new(PAGE_SIZE, PAGE_SIZE * 2).unwrap();
    let mut verifier = FrameVerifier::<2>::new();
    verifier.trust_peer(NodeId::LOCAL, old_key).unwrap();
    verifier
        .verify(&frame(old_key, 10), destination, allowed)
        .unwrap();

    verifier.trust_peer(NodeId::LOCAL, new_key).unwrap();
    assert_eq!(
        verifier.verify(&frame(old_key, 11), destination, allowed),
        Err(Error::SignatureMismatch)
    );
    verifier
        .verify(&frame(new_key, 1), destination, allowed)
        .unwrap();
    assert_eq!(
        verifier.verify(&frame(new_key, 1), destination, allowed),
        Err(Error::Replay)
    );
    assert_eq!(
        verifier.verify(&frame(new_key, 0), destination, allowed),
        Err(Error::Replay)
    );

    verifier.revoke_peer(NodeId::LOCAL).unwrap();
    assert_eq!(
        verifier.verify(&frame(new_key, 2), destination, allowed),
        Err(Error::Unauthorized)
    );
    verifier.trust_peer(NodeId::LOCAL, new_key).unwrap();
    verifier
        .verify(&frame(new_key, 1), destination, allowed)
        .unwrap();
}
