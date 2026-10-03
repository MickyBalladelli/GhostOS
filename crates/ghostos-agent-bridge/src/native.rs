#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Grant { pub expires_at_us: u64, pub occupied: bool }
fn rejected(code: i32) -> bool { code != 0 }
pub(crate) fn scope(resource: u64, rights: u16, transports: u8, lifetime_us: u64) -> Result<(), ()> {
    if rejected(unsafe { ghostos_agent_scope(resource, rights, transports, lifetime_us) }) { Err(()) } else { Ok(()) }
}
pub(crate) fn parent(parent_issuer: u32, issuer: u32, parent_resource: u64, resource: u64,
    parent_epoch: u64, epoch: u64) -> Result<(), ()> {
    if rejected(unsafe { ghostos_agent_parent(parent_issuer, issuer, parent_resource, resource, parent_epoch, epoch) }) {
        Err(())
    } else { Ok(()) }
}
pub(crate) fn covers(have: u16, need: u16) -> Result<(), ()> {
    if rejected(unsafe { ghostos_agent_covers(have, need) }) { Err(()) } else { Ok(()) }
}
pub(crate) fn transport_lifetime(have: u8, need: u8, lifetime_us: u64, max_lifetime_us: u64) -> Result<(), ()> {
    if rejected(unsafe { ghostos_agent_transport_lifetime(have, need, lifetime_us, max_lifetime_us) }) {
        Err(())
    } else { Ok(()) }
}
pub(crate) fn add_time(now_us: u64, lifetime_us: u64) -> Result<u64, ()> {
    let mut expires = 0;
    if unsafe { ghostos_agent_add_time(now_us, lifetime_us, &mut expires) } { Ok(expires) } else { Err(()) }
}
pub(crate) fn within_expiry(expires_at_us: u64, parent_expiry_us: u64) -> bool {
    unsafe { ghostos_agent_within_expiry(expires_at_us, parent_expiry_us) }
}
pub(crate) fn slot(grants: &[Grant], now_us: u64) -> Result<usize, ()> {
    let mut index = 0;
    if unsafe { ghostos_agent_slot(grants.as_ptr(), grants.len(), now_us, &mut index) } == 0 { Ok(index) } else { Err(()) }
}
pub(crate) fn next_id(value: u64) -> u64 { unsafe { ghostos_agent_next_id(value) } }
pub(crate) fn required(required: u16) -> Result<(), ()> {
    if unsafe { ghostos_agent_required(required) } == 0 { Ok(()) } else { Err(()) }
}
pub(crate) fn grant_access(now_us: u64, expires_at_us: u64, rights: u16, required: u16,
    transports: u8, transport: u8) -> Result<(), ()> {
    if unsafe { ghostos_agent_grant_access(now_us, expires_at_us, rights, required, transports, transport) } == 0 {
        Ok(())
    } else { Err(()) }
}
pub(crate) fn run_rights(task_rights: u16, commit: bool) -> u16 {
    unsafe { ghostos_agent_run_rights(task_rights, commit) }
}
pub(crate) fn active(grants: &[Grant], now_us: u64) -> usize {
    unsafe { ghostos_agent_active(grants.as_ptr(), grants.len(), now_us) }
}
pub(crate) fn finish(commit: bool, authorized: bool, success: bool) -> Result<bool, ()> {
    match unsafe { ghostos_agent_finish(commit, authorized, success) } {
        0 => Ok(false), 1 => Ok(true), _ => Err(()),
    }
}
const _: () = {
    assert!(core::mem::size_of::<Grant>() == 16);
    assert!(core::mem::offset_of!(Grant, occupied) == 8);
};
unsafe extern "C" {
    fn ghostos_agent_scope(resource: u64, rights: u16, transports: u8, lifetime_us: u64) -> i32;
    fn ghostos_agent_parent(parent_issuer: u32, issuer: u32, parent_resource: u64, resource: u64,
        parent_epoch: u64, epoch: u64) -> i32;
    fn ghostos_agent_covers(have: u16, need: u16) -> i32;
    fn ghostos_agent_transport_lifetime(have: u8, need: u8, lifetime_us: u64, max_lifetime_us: u64) -> i32;
    fn ghostos_agent_add_time(now_us: u64, lifetime_us: u64, expires_at_us: *mut u64) -> bool;
    fn ghostos_agent_within_expiry(expires_at_us: u64, parent_expiry_us: u64) -> bool;
    fn ghostos_agent_slot(grants: *const Grant, count: usize, now_us: u64, index: *mut usize) -> i32;
    fn ghostos_agent_next_id(value: u64) -> u64;
    fn ghostos_agent_required(required: u16) -> i32;
    fn ghostos_agent_grant_access(now_us: u64, expires_at_us: u64, rights: u16, required: u16,
        transports: u8, transport: u8) -> i32;
    fn ghostos_agent_run_rights(task_rights: u16, commit: bool) -> u16;
    fn ghostos_agent_active(grants: *const Grant, count: usize, now_us: u64) -> usize;
    fn ghostos_agent_finish(commit: bool, authorized: bool, success: bool) -> i32;
}
