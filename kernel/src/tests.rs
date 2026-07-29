use crate::{
    AddressSpaceId, CapabilityError, CapabilityObject, CapabilitySpace, Rights,
};

fn address_space(raw: u32) -> AddressSpaceId {
    AddressSpaceId::new(raw).expect("non-zero address space")
}

#[test]
fn delegation_attenuates_rights() {
    let owner = address_space(1);
    let borrower = address_space(2);
    let object = CapabilityObject::AddressSpace(owner);
    let mut capabilities = CapabilitySpace::<4>::new();
    let authority = capabilities
        .mint_root(
            owner,
            object,
            Rights::READ.union(Rights::DELEGATE),
        )
        .expect("root capability");

    let delegated = capabilities
        .delegate(owner, authority, borrower, Rights::READ)
        .expect("delegated capability");

    assert!(capabilities
        .authorize(borrower, delegated, object, Rights::READ)
        .is_ok());
    assert_eq!(
        capabilities.delegate(owner, authority, borrower, Rights::WRITE),
        Err(CapabilityError::RightsEscalation)
    );
}

#[test]
fn revocation_invalidates_all_descendants() {
    let owner = address_space(1);
    let child_owner = address_space(2);
    let grandchild_owner = address_space(3);
    let object = CapabilityObject::AddressSpace(owner);
    let rights = Rights::READ
        .union(Rights::DELEGATE)
        .union(Rights::REVOKE);
    let mut capabilities = CapabilitySpace::<4>::new();
    let authority = capabilities
        .mint_root(owner, object, rights)
        .expect("root capability");
    let child = capabilities
        .delegate(owner, authority, child_owner, rights)
        .expect("child capability");
    let grandchild = capabilities
        .delegate(child_owner, child, grandchild_owner, Rights::READ)
        .expect("grandchild capability");

    assert_eq!(capabilities.revoke(owner, authority), Ok(2));
    assert_eq!(capabilities.used(), 1);
    assert_eq!(
        capabilities.inspect(child_owner, child),
        Err(CapabilityError::InvalidHandle)
    );
    assert_eq!(
        capabilities.inspect(grandchild_owner, grandchild),
        Err(CapabilityError::InvalidHandle)
    );
}

#[test]
fn deleted_slots_get_new_generation_handles() {
    let owner = address_space(1);
    let object = CapabilityObject::AddressSpace(owner);
    let mut capabilities = CapabilitySpace::<1>::new();
    let first = capabilities
        .mint_root(owner, object, Rights::READ)
        .expect("first capability");

    assert_eq!(capabilities.delete(owner, first), Ok(1));

    let second = capabilities
        .mint_root(owner, object, Rights::READ)
        .expect("second capability");
    assert_ne!(first, second);
    assert_eq!(
        capabilities.inspect(owner, first),
        Err(CapabilityError::InvalidHandle)
    );
}
