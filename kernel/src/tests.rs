use crate::{
    AddressSpaceId, CapabilityError, CapabilityObject, CapabilitySpace, Rights,
};

use crate::runtime::{Dispatcher, FilesystemIpc, FilesystemIdentity};
use synos_fsd::{Capability as FsdCapability, ProcessId as FsdProcessId, Response as FsdResponse};
use synos_ipc::{SharedBuffer, SharedRegionId};
use synos_runtime::{Operation, Request};
use synos_status::Status;

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

#[test]
fn mapped_editor_memory_requires_owner_and_requested_rights() {
    let owner = address_space(1);
    let other = address_space(2);
    let region = SharedRegionId::new(7).expect("valid shared region");
    let rights = Rights::READ
        .union(Rights::WRITE)
        .union(Rights::EXECUTE)
        .union(Rights::MAP);
    let mut capabilities = CapabilitySpace::<2>::new();
    let memory = capabilities
        .mint_root(owner, CapabilityObject::MemoryRegion(region), rights)
        .expect("memory capability");

    assert!(capabilities
        .authorize_mapping(owner, memory, region, true, true)
        .is_ok());
    assert_eq!(
        capabilities.authorize_mapping(other, memory, region, false, false),
        Err(CapabilityError::AccessDenied)
    );
    assert_eq!(
        capabilities.drop_rights(owner, memory, Rights::WRITE),
        Ok(Rights::READ.union(Rights::EXECUTE).union(Rights::MAP))
    );
    assert!(capabilities
        .authorize_mapping(owner, memory, region, false, true)
        .is_ok());
    assert_eq!(
        capabilities.authorize_mapping(owner, memory, region, true, false),
        Err(CapabilityError::AccessDenied)
    );
}

struct RuntimeLinkIpc {
    request: Option<synos_fsd::Request>,
    buffer: Option<SharedBuffer>,
}

impl FilesystemIpc for RuntimeLinkIpc {
    fn transact(
        &mut self,
        _caller: AddressSpaceId,
        request: synos_fsd::Request,
        buffer: Option<SharedBuffer>,
    ) -> FsdResponse {
        self.request = Some(request);
        self.buffer = buffer;
        FsdResponse::success()
    }
}

#[test]
fn runtime_link_dispatch_preserves_operation_and_buffer_direction() {
    let caller = address_space(9);
    let process = FsdProcessId::new(9).expect("valid process");
    let authority = FsdCapability::from_raw(1_u64 << 32).expect("valid authority");
    let mut dispatcher = Dispatcher::<RuntimeLinkIpc, 2>::new(RuntimeLinkIpc {
        request: None,
        buffer: None,
    });
    dispatcher
        .register_filesystem_process(caller, FilesystemIdentity { process, authority })
        .expect("register filesystem process");

    let capability = synos_runtime::Capability::from_raw((2_u64 << 32) | 1)
        .expect("valid file capability");
    let buffer = SharedBuffer {
        region: SharedRegionId::new(4).expect("valid region"),
        offset: 16,
        length: 64,
        writable: false,
    };
    let response = dispatcher.dispatch(
        caller,
        Request::new(Operation::SynFsLink)
            .with_capability(capability)
            .with_buffer(buffer),
    );

    assert_eq!(Status::from_raw(response.status), Some(Status::NORMAL));
    assert_eq!(
        dispatcher.filesystem().request.unwrap().operation,
        synos_fsd::Operation::Link
    );
    assert_eq!(
        dispatcher.filesystem().request.unwrap().capability,
        Some(FsdCapability::from_raw(capability.raw()).unwrap())
    );
    assert_eq!(dispatcher.filesystem().buffer, Some(buffer));

    let links_buffer = SharedBuffer { writable: true, ..buffer };
    let links_response = dispatcher.dispatch(
        caller,
        Request::new(Operation::SynFsLinks).with_buffer(links_buffer),
    );
    assert_eq!(Status::from_raw(links_response.status), Some(Status::NORMAL));
    assert_eq!(
        dispatcher.filesystem().request.unwrap().operation,
        synos_fsd::Operation::Links
    );
    assert_eq!(
        dispatcher.filesystem().request.unwrap().capability,
        Some(authority)
    );
    assert_eq!(dispatcher.filesystem().buffer, Some(links_buffer));
}

#[test]
fn property_delegation_never_escalates_rights() {
    use synos_test_support::property::{run_assert, Config};

    run_assert("kernel.capability-attenuation", Config::new(0x59_3, 128), |_, _, entropy| {
        let owner = address_space(1);
        let borrower = address_space(2);
        let object = CapabilityObject::AddressSpace(owner);
        let mut capabilities = CapabilitySpace::<2>::new();
        let Ok(authority) = capabilities.mint_root(owner, object, Rights::ALL) else { return false };
        let Some(requested) = Rights::from_bits(((entropy.next_u64() as u16) & Rights::ALL.bits()) | Rights::READ.bits()) else { return false };
        let Ok(delegated) = capabilities.delegate(owner, authority, borrower, requested) else { return false };
        if capabilities
            .authorize(borrower, delegated, object, requested)
            .is_err()
        {
            return false;
        }
        let Some(extra) = Rights::from_bits((!requested.bits()) & Rights::ALL.bits()) else { return false };
        if !extra.is_empty()
            && capabilities
                .authorize(borrower, delegated, object, extra)
                .is_ok()
        {
            return false;
        }
        true
    })
    .expect("generated capability delegations remain attenuated");
}
