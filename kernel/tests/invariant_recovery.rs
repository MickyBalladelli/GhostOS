use ghostos_kernel::{
    AddressSpaceId, CapabilityObject, CapabilitySpace, Rights, Scheduler,
};
use ghostos_kernel::ipc::{Channel, ChannelId};
use ghostos_kernel::invariants::{self, InvariantId};
use ghostos_test_support::crash::{CrashBoundary, CrashDomain, CrashHarness, CrashPoint};

fn recovered_capabilities() -> (CapabilitySpace<2>, AddressSpaceId, ghostos_kernel::CapabilityHandle) {
    let owner = AddressSpaceId::new(21).expect("valid owner");
    let mut capabilities = CapabilitySpace::<2>::new();
    let endpoint = capabilities
        .mint_root(
            owner,
            CapabilityObject::IpcChannel(ChannelId::new(21).expect("valid channel")),
            Rights::SEND.union(Rights::RECEIVE),
        )
        .expect("recovered endpoint");
    (capabilities, owner, endpoint)
}

#[test]
fn invariant_recovery_rebuilds_only_valid_state() {
    let (capabilities, owner, endpoint) = recovered_capabilities();
    let channel = Channel::<2>::new(ChannelId::new(21).expect("valid channel"));
    let scheduler = Scheduler::new();
    let mut harness = CrashHarness::new(Some(CrashPoint::new(
        CrashDomain::UpdateRecovery,
        CrashBoundary::CapabilityChange,
        1,
    )));

    assert!(capabilities.check_invariants().is_ok());
    assert!(channel
        .check_invariants(&capabilities, owner, endpoint, Rights::SEND)
        .is_ok());
    assert!(scheduler.check_invariants().is_ok());
    assert!(harness
        .checkpoint(CrashDomain::UpdateRecovery, CrashBoundary::CapabilityChange)
        .is_err());

    let (recovered, recovered_owner, recovered_endpoint) = recovered_capabilities();
    assert!(recovered.check_invariants().is_ok());
    assert!(channel
        .check_invariants(
            &recovered,
            recovered_owner,
            recovered_endpoint,
            Rights::RECEIVE,
        )
        .is_ok());
    assert!(invariants::check_interrupt_delivery(
        48,
        ghostos_kernel::CpuId::new(0).expect("bootstrap CPU"),
        false,
        true,
    )
    .is_ok());
    assert!(invariants::check_page_table_transition(
        &[0x1000, 0x2000, 0x3000, 0x4000, 0x5000, 0x6000],
        0,
    )
    .is_ok());

    let failure = invariants::check_interrupt_delivery(
        256,
        ghostos_kernel::CpuId::new(0).expect("bootstrap CPU"),
        false,
        true,
    )
    .expect_err("invalid vector");
    assert_eq!(failure.invariant, InvariantId::InterruptDelivery);
    assert_eq!(failure.code, 1005);
    assert_eq!(failure.identifier(), "interrupt.delivery");
}
