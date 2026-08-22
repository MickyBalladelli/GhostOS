use ghostos_auth::{CapabilityCaveat, CapabilityKey, CryptographicCapability, TokenError, TransportRights};
use ghostos_fabric::NodeId;
use ghostos_kernel::Rights;

#[test]
fn attacker_cannot_add_rights_while_attenuating_capability() {
    let key = CapabilityKey::new([0x31; 32]);
    let subject = NodeId::new(7).unwrap();
    let token = CryptographicCapability::issue(
        key,
        NodeId::LOCAL,
        subject,
        41,
        Rights::READ,
        TransportRights::LAYER2,
        10,
        100,
        1,
        9,
    )
    .unwrap();

    assert_eq!(
        token.attenuate(CapabilityCaveat {
            subject: None,
            rights: Rights::READ.union(Rights::WRITE),
            transports: TransportRights::LAYER2,
            expires_at_us: 90,
        }),
        Err(TokenError::RightsEscalation)
    )
}

#[test]
fn modified_wire_capability_cannot_cross_privilege_boundary() {
    let key = CapabilityKey::new([0x42; 32]);
    let subject = NodeId::new(8).unwrap();
    let token = CryptographicCapability::issue(
        key,
        NodeId::LOCAL,
        subject,
        42,
        Rights::READ,
        TransportRights::LAYER2,
        10,
        100,
        2,
        10,
    )
    .unwrap();
    let mut wire = token.encode();
    wire[25] |= Rights::WRITE.bits() as u8;
    let forged = CryptographicCapability::decode(wire).unwrap();

    assert_eq!(
        forged.verify(
            key,
            subject,
            Rights::WRITE,
            TransportRights::LAYER2,
            50,
            2,
        ),
        Err(TokenError::InvalidSignature)
    )
}
