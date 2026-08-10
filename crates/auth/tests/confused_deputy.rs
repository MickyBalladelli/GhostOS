use synos_auth::{
    CapabilityLogicalNames, CapabilityKey, FederationError, FederationRegistry, FederationScope,
    LendingKind, LendingRights, ResourceLender, TokenError, TransportRights,
};
use synos_fabric::{Access, AddressRange, NodeId, PageFault, PAGE_SIZE};
use synos_fabric::dsm::{DlmLeaseMode, SoftwareDlmLease};
use synos_kernel::{AddressSpaceId, CapabilityObject, CapabilitySpace, Rights};
use synos_system_model::logical::{LogicalScope, LogicalTarget, LogicalTargetKind, Principal};

#[test]
fn logical_name_service_rejects_wrong_namespace_generation_and_operation() {
    let caller_space = AddressSpaceId::new(7).unwrap();
    let caller = Principal::new(8).unwrap();
    let namespace = LogicalScope::Process(7);
    let other_namespace = LogicalScope::Process(8);
    let target = LogicalTarget::new(LogicalTargetKind::File, "SYS$DISK:TARGET").unwrap();
    let mut capabilities = CapabilitySpace::<8>::new();
    let root = capabilities
        .mint_root(
            caller_space,
            CapabilityObject::LogicalNamespace { scope: 1, id: 7 },
            Rights::READ
                .union(Rights::WRITE)
                .union(Rights::DELEGATE)
                .union(Rights::REVOKE),
        )
        .unwrap();
    let other_root = capabilities
        .mint_root(
            caller_space,
            CapabilityObject::LogicalNamespace { scope: 1, id: 8 },
            Rights::WRITE,
        )
        .unwrap();
    let read_write = capabilities
        .delegate(
            caller_space,
            root,
            caller_space,
            Rights::READ.union(Rights::WRITE),
        )
        .unwrap();
    let read_only = capabilities
        .delegate(caller_space, root, caller_space, Rights::READ)
        .unwrap();
    let mut service = CapabilityLogicalNames::<8, 2, 8>::new(caller, capabilities);

    service
        .define(caller_space, other_root, caller, other_namespace, "OTHER", target)
        .unwrap();
    service
        .define(caller_space, root, caller, namespace, "TARGET", target)
        .unwrap();

    assert_eq!(
        service.resolve(
            caller_space,
            read_write,
            caller,
            8,
            None,
            None,
            "OTHER",
        ),
        Err(synos_system_model::logical::LogicalError::AccessDenied)
    );
    assert_eq!(
        service.define(
            caller_space,
            read_only,
            caller,
            namespace,
            "TARGET",
            target,
        ),
        Err(synos_system_model::logical::LogicalError::AccessDenied)
    );
    assert!(service
        .resolve(
            caller_space,
            read_write,
            caller,
            7,
            None,
            None,
            "TARGET",
        )
        .is_ok());

    service.capabilities_mut().revoke(caller_space, root).unwrap();
    assert_eq!(
        service.resolve(
            caller_space,
            read_write,
            caller,
            7,
            None,
            None,
            "TARGET",
        ),
        Err(synos_system_model::logical::LogicalError::AccessDenied)
    );
}

#[test]
fn resource_service_rejects_valid_capability_for_wrong_object_or_tenant() {
    let key = CapabilityKey::new([6; 32]);
    let borrower = NodeId::new(2).unwrap();
    let other_borrower = NodeId::new(3).unwrap();
    let mut lender = ResourceLender::<2>::new(NodeId::LOCAL, key, 0);
    let range = AddressRange::new(PAGE_SIZE * 4, PAGE_SIZE).unwrap();
    let capability = lender
        .lend_memory(
            borrower,
            41,
            LendingKind::Ram,
            range,
            LendingRights::READ_WRITE,
            TransportRights::LAYER2,
            0,
            100,
        )
        .unwrap();
    let lease = SoftwareDlmLease {
        owner: borrower,
        mode: DlmLeaseMode::Exclusive,
        epoch: 1,
        expires_at_us: 100,
    };
    let fault = PageFault {
        virtual_address: PAGE_SIZE * 8,
        access: Access::Read,
        user: true,
        present: false,
        reserved_bit: false,
        instruction_fetch: false,
    };

    assert_eq!(
        lender.authorize_remote_fault(
            &capability,
            borrower,
            fault,
            TransportRights::LAYER2,
            lease,
            10,
        ),
        Err(synos_auth::LendingError::AccessDenied)
    );
    assert_eq!(
        lender.authorize_remote_fault(
            &capability,
            other_borrower,
            PageFault {
                virtual_address: PAGE_SIZE * 4,
                ..fault
            },
            TransportRights::LAYER2,
            lease,
            10,
        ),
        Err(synos_auth::LendingError::AccessDenied)
    );
}

#[test]
fn federation_service_rejects_valid_capability_for_wrong_generation() {
    let key = CapabilityKey::new([7; 32]);
    let local = synos_auth::ClusterId::new(1).unwrap();
    let peer = synos_auth::ClusterId::new(2).unwrap();
    let mut registry = FederationRegistry::<2, 2>::new(local, key);
    let invitation = registry
        .invite(
            peer,
            FederationScope::CPU,
            TransportRights::LAYER2,
            0,
            100,
            1,
        )
        .unwrap();
    registry.accept(invitation, 1).unwrap();
    assert!(registry
        .authorize_offer(
            peer,
            synos_auth::FederatedResourceKind::Cpu,
            TransportRights::LAYER2,
            1,
            2,
        )
        .is_ok());

    registry.fence(peer, 3).unwrap();
    assert_eq!(
        registry.authorize_offer(
            peer,
            synos_auth::FederatedResourceKind::Cpu,
            TransportRights::LAYER2,
            1,
            4,
        ),
        Err(FederationError::StaleEpoch)
    );
}

#[test]
fn capability_service_rejects_valid_capability_for_wrong_operation() {
    let key = CapabilityKey::new([8; 32]);
    let token = synos_auth::CryptographicCapability::issue(
        key,
        NodeId::LOCAL,
        NodeId::new(2).unwrap(),
        99,
        Rights::READ,
        TransportRights::LAYER2,
        0,
        100,
        1,
        1,
    )
    .unwrap();

    assert_eq!(
        token.verify(
            key,
            NodeId::new(2).unwrap(),
            Rights::WRITE,
            TransportRights::LAYER2,
            10,
            1,
        ),
        Err(TokenError::AccessDenied)
    );
}
