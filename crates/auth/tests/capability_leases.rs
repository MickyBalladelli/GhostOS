use synos_auth::{CapabilityKey, CapabilityLease, LeaseContext, LeaseError, LeaseReplayGuard};
use synos_fabric::NodeId;
use synos_kernel::Rights;

const PURPOSE_IO: u64 = 7;

fn lease(audience: NodeId, object: u64, tenant: u64, generation: u64) -> CapabilityLease {
    CapabilityLease::issue(
        CapabilityKey::new([9; 32]),
        NodeId::LOCAL,
        NodeId::new(2).unwrap(),
        audience,
        object,
        tenant,
        generation,
        PURPOSE_IO,
        Rights::READ,
        10,
        100,
        4,
        object ^ tenant ^ generation,
    )
    .unwrap()
}

fn context(audience: NodeId, object: u64, tenant: u64, generation: u64) -> LeaseContext {
    LeaseContext {
        subject: NodeId::new(2).unwrap(),
        audience,
        object,
        tenant,
        generation,
        purpose: PURPOSE_IO,
        required: Rights::READ,
        now_us: 20,
    }
}

#[test]
fn privileged_daemons_bind_audience_object_tenant_generation_and_purpose() {
    let daemons = [
        NodeId::new(11).unwrap(),
        NodeId::new(12).unwrap(),
        NodeId::new(13).unwrap(),
        NodeId::new(14).unwrap(),
        NodeId::new(15).unwrap(),
        NodeId::new(16).unwrap(),
    ];
    for audience in daemons {
        let token = lease(audience, 41, 5, 9);
        assert_eq!(
            token.authorize(CapabilityKey::new([9; 32]), context(audience, 41, 5, 9), 4),
            Ok(())
        );
        assert_eq!(
            token.authorize(
                CapabilityKey::new([9; 32]),
                context(NodeId::LOCAL, 41, 5, 9),
                4,
            ),
            Err(LeaseError::AudienceMismatch)
        );
    }
}

#[test]
fn copied_tokens_fail_for_other_object_tenant_generation_or_purpose() {
    let token = lease(NodeId::LOCAL, 41, 5, 9);
    for (object, tenant, generation, purpose, expected) in [
        (42, 5, 9, PURPOSE_IO, LeaseError::ObjectMismatch),
        (41, 6, 9, PURPOSE_IO, LeaseError::TenantMismatch),
        (41, 5, 10, PURPOSE_IO, LeaseError::GenerationMismatch),
        (41, 5, 9, PURPOSE_IO + 1, LeaseError::PurposeMismatch),
    ] {
        let mut request = context(NodeId::LOCAL, object, tenant, generation);
        request.purpose = purpose;
        assert_eq!(
            token.authorize(CapabilityKey::new([9; 32]), request, 4),
            Err(expected)
        );
    }
}

#[test]
fn expiry_revocation_and_replay_are_rejected() {
    let token = lease(NodeId::LOCAL, 41, 5, 9);
    assert_eq!(
        token.authorize(
            CapabilityKey::new([9; 32]),
            LeaseContext {
                now_us: 100,
                ..context(NodeId::LOCAL, 41, 5, 9)
            },
            4,
        ),
        Err(LeaseError::Expired)
    );
    assert_eq!(
        token.authorize(CapabilityKey::new([9; 32]), context(NodeId::LOCAL, 41, 5, 9), 5),
        Err(LeaseError::Revoked)
    );

    let mut guard = LeaseReplayGuard::<2>::new();
    assert_eq!(
        guard.authorize_once(
            &token,
            CapabilityKey::new([9; 32]),
            context(NodeId::LOCAL, 41, 5, 9),
            4,
        ),
        Ok(())
    );
    assert_eq!(
        guard.authorize_once(
            &token,
            CapabilityKey::new([9; 32]),
            context(NodeId::LOCAL, 41, 5, 9),
            4,
        ),
        Err(LeaseError::Replay)
    );
}

#[test]
fn encoded_lease_keeps_signature_bound_to_all_fields() {
    let token = lease(NodeId::LOCAL, 41, 5, 9);
    let mut encoded = token.encode();
    encoded[96] ^= 1;
    let copied = CapabilityLease::decode(encoded).unwrap();
    assert_eq!(
        copied.authorize(CapabilityKey::new([9; 32]), context(NodeId::LOCAL, 41, 5, 9), 4),
        Err(LeaseError::InvalidSignature)
    );
}
