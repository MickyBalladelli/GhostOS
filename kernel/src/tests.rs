use crate::{
    AddressSpaceId, CapabilityError, CapabilityObject, CapabilitySpace, Rights,
};

use crate::allocator::{EarlyFrameAllocator, QuotaAllocationError};
use crate::ipc::{Channel, IpcError, Message};
use crate::runtime::{Dispatcher, FilesystemIpc, FilesystemIdentity};
use crate::scheduler::SchedulerError;
use crate::task::{ExecutionMode, SchedulingPolicy, ThreadState};
use synos_boot_protocol::{BootInfo, BootMethod, MemoryKind, MemoryRegion, MAX_MEMORY_REGIONS};
use synos_fsd::{Capability as FsdCapability, ProcessId as FsdProcessId, Response as FsdResponse};
use synos_ipc::{SharedBuffer, SharedRegionId};
use synos_runtime::{Operation, Request};
use synos_status::Status;

#[test]
fn malformed_boot_info_returns_a_stable_status() {
    let mut boot_info = BootInfo::empty(BootMethod::Uefi);
    boot_info.memory_region_count = MAX_MEMORY_REGIONS + 1;

    assert_eq!(crate::validate_boot_info(&boot_info), Err(Status::INVALID_ARGUMENT));
}

fn address_space(raw: u32) -> AddressSpaceId {
    AddressSpaceId::new(raw).expect("non-zero address space")
}

#[test]
fn address_spaces_prove_private_read_write_boundaries() {
    let first = address_space(41);
    let second = address_space(42);
    let mut spaces = crate::AddressSpaceTable::<2>::new();
    spaces
        .create(first, crate::PageTableRoot::new(0x1000).unwrap())
        .expect("first page table");
    spaces
        .create(second, crate::PageTableRoot::new(0x2000).unwrap())
        .expect("second page table");
    let mut capabilities = CapabilitySpace::<4>::new();
    let first_memory = capabilities
        .mint_untyped(
            first,
            crate::PhysicalRange::new(0x40_0000, 0x1000).unwrap(),
            Rights::MAP.union(Rights::READ).union(Rights::WRITE),
        )
        .expect("first private memory");
    let second_memory = capabilities
        .mint_untyped(
            second,
            crate::PhysicalRange::new(0x50_0000, 0x1000).unwrap(),
            Rights::MAP.union(Rights::READ).union(Rights::WRITE),
        )
        .expect("second private memory");
    let first_mapping = spaces
        .get_mut(first)
        .unwrap()
        .map_backing(
            first_memory,
            crate::PhysicalRange::new(0x40_0000, 0x1000).unwrap(),
            true,
        )
        .unwrap();
    let second_mapping = spaces
        .get_mut(second)
        .unwrap()
        .map_backing(
            second_memory,
            crate::PhysicalRange::new(0x50_0000, 0x1000).unwrap(),
            true,
        )
        .unwrap();

    assert_eq!(spaces.check_isolation(first, second), Ok(()));
    assert!(spaces
        .get(first)
        .unwrap()
        .can_access(first_mapping.base, 0x1000, crate::MemoryAccess::Read));
    assert!(spaces
        .get(first)
        .unwrap()
        .can_access(first_mapping.base, 0x1000, crate::MemoryAccess::Write));
    assert!(spaces
        .get(second)
        .unwrap()
        .can_access(second_mapping.base, 0x1000, crate::MemoryAccess::Read));
    assert!(spaces
        .get(second)
        .unwrap()
        .can_access(second_mapping.base, 0x1000, crate::MemoryAccess::Write));
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
fn untyped_memory_retype_rejects_overlapping_children_and_preserves_backing() {
    let owner = address_space(1);
    let borrower = address_space(2);
    let mut capabilities = CapabilitySpace::<4>::new();
    let source = capabilities
        .mint_untyped(
            owner,
            crate::PhysicalRange::new(0x20_0000, 0x4000).unwrap(),
            Rights::CREATE.union(Rights::READ).union(Rights::MAP),
        )
        .expect("untyped memory");
    let region = SharedRegionId::new(12).unwrap();
    let child = capabilities
        .retype_memory(
            owner,
            source,
            borrower,
            region,
            crate::PhysicalRange::new(0x20_0000, 0x2000).unwrap(),
            Rights::READ.union(Rights::MAP),
        )
        .expect("retype memory");
    assert_eq!(
        capabilities.inspect(borrower, child).unwrap().backing,
        Some(crate::PhysicalRange::new(0x20_0000, 0x2000).unwrap())
    );
    assert_eq!(
        capabilities.retype_memory(
            owner,
            source,
            borrower,
            SharedRegionId::new(13).unwrap(),
            crate::PhysicalRange::new(0x20_1000, 0x1000).unwrap(),
            Rights::READ,
        ),
        Err(CapabilityError::AccessDenied)
    );
    assert_eq!(
        capabilities.inspect(owner, child),
        Err(CapabilityError::AccessDenied)
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

struct RuntimeDeleteIpc {
    request: Option<synos_fsd::Request>,
    buffer: Option<SharedBuffer>,
}

impl FilesystemIpc for RuntimeDeleteIpc {
    fn transact(
        &mut self,
        _caller: AddressSpaceId,
        request: synos_fsd::Request,
        buffer: Option<SharedBuffer>,
    ) -> FsdResponse {
        self.request = Some(request);
        self.buffer = buffer;
        FsdResponse::success()
            .with_value(0, 7)
            .with_value(1, 1)
            .with_value(2, 1)
            .with_value(3, 1)
    }
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
fn runtime_delete_dispatch_uses_authority_and_bounded_input_buffer() {
    let caller = address_space(10);
    let process = FsdProcessId::new(10).expect("valid process");
    let authority = FsdCapability::from_raw(1_u64 << 32).expect("valid authority");
    let mut dispatcher = Dispatcher::<RuntimeDeleteIpc, 2>::new(RuntimeDeleteIpc {
        request: None,
        buffer: None,
    });
    dispatcher
        .register_filesystem_process(caller, FilesystemIdentity { process, authority })
        .expect("register filesystem process");

    let buffer = SharedBuffer {
        region: SharedRegionId::new(5).expect("valid region"),
        offset: 24,
        length: 128,
        writable: false,
    };
    let response = dispatcher.dispatch(
        caller,
        Request::new(Operation::SynFsDelete).with_buffer(buffer),
    );
    assert_eq!(Status::from_raw(response.status), Some(Status::NORMAL));
    assert_eq!(dispatcher.filesystem().request.unwrap().operation, synos_fsd::Operation::Delete);
    assert_eq!(dispatcher.filesystem().request.unwrap().capability, Some(authority));
    assert_eq!(dispatcher.filesystem().buffer, Some(buffer));

    let oversized = SharedBuffer {
        length: (synos_fsd::MAX_IPC_BUFFER_BYTES + 1) as u32,
        ..buffer
    };
    let response = dispatcher.dispatch(
        caller,
        Request::new(Operation::SynFsDelete).with_buffer(oversized),
    );
    assert_eq!(Status::from_raw(response.status), Some(Status::INVALID_ARGUMENT));
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

#[test]
fn allocator_skips_reserved_memory_aligns_frames_and_reports_exhaustion() {
    let regions = [
        MemoryRegion {
            start: 0x1000,
            length: 0x1000,
            kind: MemoryKind::Reserved,
            attributes: 0,
        },
        MemoryRegion {
            start: 16 * 1024 * 1024 + 1,
            length: 0x3000,
            kind: MemoryKind::Usable,
            attributes: 0,
        },
    ];
    let mut allocator = EarlyFrameAllocator::new(&regions);
    assert_eq!(allocator.allocate(), Ok(16 * 1024 * 1024 + 0x1000));
    assert_eq!(allocator.allocate(), Ok(16 * 1024 * 1024 + 0x2000));
    assert_eq!(allocator.allocate(), Err(crate::AllocationError));
}

#[test]
fn allocator_falls_back_below_early_floor_when_needed() {
    let regions = [MemoryRegion {
        start: 0x1000,
        length: 0x2000,
        kind: MemoryKind::Usable,
        attributes: 0,
    }];
    let mut allocator = EarlyFrameAllocator::new(&regions);

    assert_eq!(allocator.allocate(), Ok(0x1000));
}

#[test]
fn allocator_refunds_quota_when_no_frame_is_available() {
    let regions = [MemoryRegion {
        start: 0x1000,
        length: 0x1000,
        kind: MemoryKind::Reserved,
        attributes: 0,
    }];
    let quota = crate::CapabilityQuota::new();
    let before = quota.usage();
    let mut allocator = EarlyFrameAllocator::new(&regions);
    assert_eq!(
        allocator.allocate_for(&quota, 0),
        Err(QuotaAllocationError::Exhausted)
    );
    assert_eq!(quota.usage(), before);
}

#[test]
fn scheduler_transitions_tasks_and_rejects_stale_or_unauthorized_control() {
    let owner = address_space(1);
    let other = address_space(2);
    let mut capabilities = CapabilitySpace::<4>::new();
    let authority = capabilities
        .mint_root(
            owner,
            CapabilityObject::AddressSpace(owner),
            Rights::CREATE.union(Rights::CONTROL),
        )
        .expect("scheduler authority");
    let mut scheduler = crate::Scheduler::new();
    let thread = scheduler
        .create(
            &capabilities,
            owner,
            authority,
            owner,
            ExecutionMode::User,
            SchedulingPolicy::Cooperative,
            0x1000,
            0x8000,
        )
        .expect("create task");
    assert_eq!(scheduler.thread(thread).unwrap().state, ThreadState::Ready);
    assert_eq!(scheduler.dispatch().unwrap().next, thread);
    assert_eq!(scheduler.block_current().unwrap(), None);
    assert_eq!(scheduler.thread(thread).unwrap().state, ThreadState::Blocked);
    scheduler.wake(thread).expect("wake task");
    assert_eq!(scheduler.thread(thread).unwrap().state, ThreadState::Ready);
    assert_eq!(
        scheduler.stop(&capabilities, other, authority, thread),
        Err(SchedulerError::AccessDenied)
    );
    scheduler
        .stop(&capabilities, owner, authority, thread)
        .expect("stop task");
    assert!(matches!(
        scheduler.thread(thread),
        Err(SchedulerError::InvalidThread)
    ));
}

#[test]
fn scheduler_prioritizes_deadlines_and_keeps_isolated_cpus_out_of_kernel_work() {
    let owner = address_space(1);
    let mut capabilities = CapabilitySpace::<6>::new();
    let create = capabilities
        .mint_root(
            owner,
            CapabilityObject::AddressSpace(owner),
            Rights::CREATE,
        )
        .expect("create authority");
    let control = capabilities
        .mint_root(owner, CapabilityObject::SystemControl, Rights::CONTROL)
        .expect("system authority");
    let mut scheduler = crate::Scheduler::new();
    scheduler
        .set_online_cores(&capabilities, owner, control, crate::CpuMask::from_raw(0b11))
        .expect("online CPUs");
    scheduler
        .isolate_cores(&capabilities, owner, control, crate::CpuMask::from_raw(0b10))
        .expect("isolate one CPU");
    assert_eq!(scheduler.partition().housekeeping(), crate::CpuMask::CPU0);
    assert_eq!(
        scheduler.dispatch_on(crate::CpuId::new(1).unwrap()),
        None
    );

    let later = scheduler
        .create(
            &capabilities,
            owner,
            create,
            owner,
            ExecutionMode::User,
            SchedulingPolicy::Realtime {
                priority: 20,
                deadline: 100,
            },
            0x1000,
            0x8000,
        )
        .expect("later deadline task");
    let earlier = scheduler
        .create(
            &capabilities,
            owner,
            create,
            owner,
            ExecutionMode::User,
            SchedulingPolicy::Realtime {
                priority: 20,
                deadline: 50,
            },
            0x2000,
            0x9000,
        )
        .expect("earlier deadline task");
    assert_ne!(later, earlier);
    assert_eq!(scheduler.dispatch().unwrap().next, earlier);
}

#[test]
fn ipc_enforces_identity_queue_bounds_and_zero_copy_buffers() {
    let owner = address_space(1);
    let other = address_space(2);
    let channel_id = crate::ipc::ChannelId::new(8).expect("valid channel");
    let region = SharedRegionId::new(3).expect("valid region");
    let mut capabilities = CapabilitySpace::<4>::new();
    let endpoint = capabilities
        .mint_root(
            owner,
            CapabilityObject::IpcChannel(channel_id),
            Rights::SEND.union(Rights::RECEIVE),
        )
        .expect("endpoint capability");
    let memory = capabilities
        .mint_root(
            owner,
            CapabilityObject::MemoryRegion(region),
            Rights::READ.union(Rights::WRITE).union(Rights::MAP),
        )
        .expect("memory capability");
    let channel = Channel::<2>::new(channel_id);
    let message = Message {
        correlation: synos_observability::CorrelationId::NONE,
        label: 42,
        buffer: Some(SharedBuffer {
            region,
            offset: 4,
            length: 8,
            writable: false,
        }),
        words: [1, 2, 3, 4],
    };

    assert_eq!(
        channel.try_send(&capabilities, other, endpoint, Some(memory), message),
        Err(IpcError::AccessDenied)
    );
    assert!(channel
        .try_send(&capabilities, owner, endpoint, Some(memory), message)
        .is_ok());
    assert!(channel
        .try_send(&capabilities, owner, endpoint, Some(memory), message)
        .is_ok());
    assert_eq!(
        channel.try_send(&capabilities, owner, endpoint, Some(memory), message),
        Err(IpcError::Full)
    );
    assert_eq!(channel.try_receive(&capabilities, owner, endpoint).unwrap().label, 42);
    assert_eq!(channel.try_receive(&capabilities, owner, endpoint).unwrap().buffer, message.buffer);
    assert_eq!(channel.try_receive(&capabilities, owner, endpoint), Err(IpcError::Empty));
}

#[test]
fn page_fault_dispatch_is_quota_limited_and_status_mapped() {
    let owner = address_space(1);
    let mut capabilities = CapabilitySpace::<1>::new();
    let authority = capabilities
        .mint_root(
            owner,
            CapabilityObject::SystemControl,
            Rights::READ.union(Rights::CONTROL),
        )
        .expect("pager authority");
    let policy = crate::QuotaPolicy::new(
        crate::BucketConfig::new(8, 8).unwrap(),
        crate::BucketConfig::new(1, 1).unwrap(),
        crate::BucketConfig::new(8, 8).unwrap(),
        4096,
    )
    .unwrap();
    capabilities
        .configure_quota(owner, authority, policy)
        .expect("configure pager quota");
    let fault = crate::PageFault {
        virtual_address: 0x4000,
        access: synos_fabric::Access::Read,
        user: true,
        present: false,
        reserved_bit: false,
        instruction_fetch: false,
    };
    assert_eq!(
        crate::page_fault::dispatch_for(&capabilities, owner, authority, 0, fault),
        Ok(false)
    );
    assert_eq!(
        capabilities.consume_quota(
            owner,
            authority,
            crate::QuotaResource::PageFaults,
            0,
            1,
        ),
        Ok(crate::QuotaDecision::Allowed)
    );
    assert!(matches!(
        crate::page_fault::dispatch_for(&capabilities, owner, authority, 0, fault),
        Err(crate::PageFaultDispatchError::RateLimited { .. })
    ));
}

#[test]
fn delegated_capabilities_consume_parent_quota_atomically() {
    let owner = address_space(1);
    let first = address_space(2);
    let second = address_space(3);
    let mut capabilities = CapabilitySpace::<4>::new();
    let root = capabilities
        .mint_root(
            owner,
            CapabilityObject::SystemControl,
            Rights::READ.union(Rights::CONTROL).union(Rights::DELEGATE),
        )
        .unwrap();
    let policy = crate::QuotaPolicy::new(
        crate::BucketConfig::new(1, 1).unwrap(),
        crate::BucketConfig::new(8, 8).unwrap(),
        crate::BucketConfig::new(8, 8).unwrap(),
        4096,
    )
    .unwrap();
    capabilities.configure_quota(owner, root, policy).unwrap();
    let first_cap = capabilities.delegate(owner, root, first, Rights::READ).unwrap();
    let second_cap = capabilities.delegate(owner, root, second, Rights::READ).unwrap();
    assert_eq!(
        capabilities.consume_quota(first, first_cap, crate::QuotaResource::IpcMessages, 0, 1),
        Ok(crate::QuotaDecision::Allowed)
    );
    assert!(matches!(
        capabilities.consume_quota(second, second_cap, crate::QuotaResource::IpcMessages, 0, 1),
        Ok(crate::QuotaDecision::Throttled { .. })
    ));
    assert_eq!(
        capabilities.quota_usage(second, second_cap).unwrap().memory_in_use,
        0
    );
}

#[test]
fn invariants_catalogue_and_model_state_are_redacted() {
    use crate::invariants::{CATALOGUE, InvariantId};

    assert_eq!(CATALOGUE.len(), 6);
    assert_eq!(CATALOGUE[0].id, InvariantId::AddressSpaceOwnership.as_str());
    assert!(CATALOGUE.iter().all(|entry| entry.redaction == "identifier-only"));

    let owner = address_space(11);
    let mut capabilities = CapabilitySpace::<2>::new();
    let endpoint = capabilities
        .mint_root(
            owner,
            CapabilityObject::IpcChannel(crate::ipc::ChannelId::new(11).unwrap()),
            Rights::SEND.union(Rights::RECEIVE),
        )
        .expect("channel capability");
    assert!(capabilities.check_invariants().is_ok());

    let channel = Channel::<2>::new(crate::ipc::ChannelId::new(11).unwrap());
    assert!(channel
        .check_invariants(&capabilities, owner, endpoint, Rights::SEND)
        .is_ok());

    let scheduler = crate::Scheduler::new();
    assert!(scheduler.check_invariants().is_ok());
    assert!(crate::invariants::check_address_space(owner).is_ok());
    assert!(crate::invariants::check_interrupt_delivery(
        32,
        crate::CpuId::new(0).unwrap(),
        false,
        true,
    )
    .is_ok());
    assert!(crate::invariants::check_page_table_transition(
        &[0x1000, 0x2000, 0x3000, 0x4000, 0x5000, 0x6000],
        0,
    )
    .is_ok());

    let failure = crate::invariants::check_page_table_transition(
        &[0x1000, 0x1000, 0x3000, 0x4000, 0x5000, 0x6000],
        0,
    )
    .expect_err("duplicate table frame");
    assert_eq!(failure.invariant, InvariantId::PageTableTransition);
    assert_eq!(failure.code, 1006);
    assert_eq!(failure.identifier(), "page_table.transition");
}

#[test]
fn runtime_rejects_unknown_operations_reserved_bits_and_bad_buffers() {
    let caller = address_space(9);
    let process = FsdProcessId::new(9).expect("valid process");
    let authority = FsdCapability::from_raw(1_u64 << 32).expect("valid authority");
    let mut dispatcher = Dispatcher::<RuntimeLinkIpc, 1>::new(RuntimeLinkIpc {
        request: None,
        buffer: None,
    });
    dispatcher
        .register_filesystem_process(caller, FilesystemIdentity { process, authority })
        .expect("register filesystem process");

    let mut unknown = Request::new(Operation::Yield);
    unknown.operation = u16::MAX;
    assert_eq!(Status::from_raw(dispatcher.dispatch(caller, unknown).status), Some(Status::INVALID_ARGUMENT));

    let mut reserved = Request::new(Operation::ClockNow);
    reserved.reserved = 1;
    assert_eq!(Status::from_raw(dispatcher.dispatch(caller, reserved).status), Some(Status::INVALID_ARGUMENT));

    let path = SharedBuffer {
        region: SharedRegionId::new(4).unwrap(),
        offset: 0,
        length: 4,
        writable: false,
    };
    assert_eq!(
        Status::from_raw(
            dispatcher
                .dispatch(caller, Request::new(Operation::SynFsRead).with_buffer(path))
                .status
        ),
        Some(Status::INVALID_ARGUMENT)
    );
}
