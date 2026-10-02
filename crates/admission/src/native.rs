//! Allocation-free adapters for the C admission controller.

use super::*;
use core::mem::MaybeUninit;

const EMPTY_LEASE: AdmissionLease = AdmissionLease {
    slot: 0, sequence: 0, class: WorkClass::ControlPlaneFanout,
    priority: AdmissionPriority::Optional, tenant: 0,
};

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Slot { lease: AdmissionLease, occupied: bool }

#[repr(C)]
#[derive(Clone, Copy)]
struct TenantPolicy {
    tenant: u64,
    parent: u64,
    active_limit: u16,
    recovery_reserve: u16,
    has_parent: bool,
}

#[repr(C)]
struct TenantState {
    policy: TenantPolicy,
    active: u16,
    recovery_active: u16,
    occupied: bool,
}

#[repr(C)]
pub(super) struct State {
    pub policy: AdmissionPolicy,
    active_by_class: [u16; WORK_CLASS_COUNT],
    stats: [AdmissionStats; WORK_CLASS_COUNT],
    active: u16,
    recovery_active: u16,
    queued: u16,
    next_sequence: u64,
    tenants: [TenantState; MAX_TENANT_POLICIES],
}

#[repr(C)]
struct Outcome {
    version: u16,
    sequence: u64,
    class: u8,
    priority: u8,
    action: u8,
    reason: u8,
    active: u16,
    queued: u16,
    lease: AdmissionLease,
    has_lease: bool,
}

unsafe extern "C" {
    fn ghostos_admission_init(state: *mut State, slots: *mut Slot,
        capacity: usize, policy: *const AdmissionPolicy) -> u32;
    fn ghostos_admission_get_report(state: *const State, report: *mut AdmissionReport);
    fn ghostos_admission_configure_tenant(state: *mut State, capacity: usize, policy: *const TenantPolicy) -> u32;
    fn ghostos_admission_admit(state: *mut State, slots: *mut Slot, capacity: usize,
        tenant: u64, class: u8, priority: u8, outcome: *mut Outcome) -> u32;
    fn ghostos_admission_record_retry(state: *mut State, class: u8, priority: u8, outcome: *mut Outcome) -> u32;
    fn ghostos_admission_finish(state: *mut State, slots: *mut Slot,
        capacity: usize, lease: *const AdmissionLease) -> u32;
}

fn result(code: u32) -> Result<(), AdmissionError> {
    match code {
        0 => Ok(()),
        1 => Err(AdmissionError::InvalidPolicy),
        2 => Err(AdmissionError::InvalidLease),
        _ => unreachable!("unknown C admission result"),
    }
}

pub(super) fn initialize<const CAPACITY: usize>(policy: AdmissionPolicy) -> Result<(State, [Slot; CAPACITY]), AdmissionError> {
    let mut state = MaybeUninit::uninit();
    let mut slots = [Slot { lease: EMPTY_LEASE, occupied: false }; CAPACITY];
    // C initializes every field on success and retains no storage pointers.
    result(unsafe { ghostos_admission_init(state.as_mut_ptr(), slots.as_mut_ptr(), CAPACITY, &policy) })?;
    Ok((unsafe { state.assume_init() }, slots))
}

pub(super) fn report(state: &State) -> AdmissionReport {
    let mut report = MaybeUninit::uninit();
    unsafe {
        ghostos_admission_get_report(state, report.as_mut_ptr());
        report.assume_init()
    }
}

pub(super) fn configure_tenant(state: &mut State, capacity: usize, policy: TenantAdmissionPolicy) -> Result<(), AdmissionError> {
    let policy = TenantPolicy { tenant: policy.tenant, parent: policy.parent.unwrap_or(0),
        active_limit: policy.active_limit, recovery_reserve: policy.recovery_reserve, has_parent: policy.parent.is_some() };
    result(unsafe { ghostos_admission_configure_tenant(state, capacity, &policy) })
}

fn outcome(value: Outcome, class: WorkClass, priority: AdmissionPriority) -> AdmissionOutcome {
    debug_assert_eq!(value.class, class as u8);
    debug_assert_eq!(value.priority, priority as u8);
    AdmissionOutcome {
        version: value.version, sequence: value.sequence, class, priority,
        action: match value.action { 1 => AdmissionAction::Admitted, 2 => AdmissionAction::Delayed,
            3 => AdmissionAction::Dropped, 4 => AdmissionAction::Retried, _ => unreachable!("C admission action") },
        reason: match value.reason { 0 => AdmissionReason::None, 1 => AdmissionReason::ActiveCapacity,
            2 => AdmissionReason::RecoveryReserve, 3 => AdmissionReason::ClassLimit,
            4 => AdmissionReason::QueueFull, 5 => AdmissionReason::TenantLimit, _ => unreachable!("C admission reason") },
        active: value.active, queued: value.queued,
        lease: value.has_lease.then_some(value.lease),
    }
}

pub(super) fn admit(state: &mut State, slots: &mut [Slot], tenant: u64,
    class: WorkClass, priority: AdmissionPriority) -> AdmissionOutcome {
    let mut output = MaybeUninit::uninit();
    // Initialized controllers preserve the free-slot invariant. All C mutations
    // use these exclusive borrows; class and priority are valid Rust enum values.
    unsafe {
        result(ghostos_admission_admit(state, slots.as_mut_ptr(), slots.len(), tenant, class as u8, priority as u8, output.as_mut_ptr()))
            .expect("admission policy capacity invariant");
        outcome(output.assume_init(), class, priority)
    }
}

pub(super) fn record_retry(state: &mut State, class: WorkClass, priority: AdmissionPriority) -> AdmissionOutcome {
    let mut output = MaybeUninit::uninit();
    unsafe {
        result(ghostos_admission_record_retry(state, class as u8, priority as u8, output.as_mut_ptr()))
            .expect("valid admission class and priority");
        outcome(output.assume_init(), class, priority)
    }
}

pub(super) fn finish(state: &mut State, slots: &mut [Slot], lease: AdmissionLease) -> Result<(), AdmissionError> {
    result(unsafe { ghostos_admission_finish(state, slots.as_mut_ptr(), slots.len(), &lease) })
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(core::mem::size_of::<AdmissionPolicy>() == 18);
    assert!(core::mem::size_of::<AdmissionLease>() == 32);
    assert!(core::mem::size_of::<Slot>() == 40);
    assert!(core::mem::size_of::<TenantPolicy>() == 24);
    assert!(core::mem::size_of::<State>() == 1192);
    assert!(core::mem::offset_of!(State, tenants) == 168);
    assert!(core::mem::size_of::<Outcome>() == 64);
    assert!(core::mem::size_of::<AdmissionReport>() == 148);
};
