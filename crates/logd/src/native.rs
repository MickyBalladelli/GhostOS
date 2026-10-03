#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Subscription { pub terminal: u32, pub minimum_level: u8, pub occupied: bool }
pub(crate) fn subscribe(slots: &[Subscription], terminal: u32, minimum_level: u8) -> Result<(bool, usize), ()> {
    let mut index = 0;
    match unsafe { ghostos_log_subscribe(slots.as_ptr(), slots.len(), terminal, minimum_level, &mut index) } {
        0 => Ok((true, index)),
        1 => Ok((false, index)),
        _ => Err(()),
    }
}
pub(crate) fn unsubscribe(slots: &[Subscription], terminal: u32) -> Result<usize, ()> {
    let mut index = 0;
    if unsafe { ghostos_log_unsubscribe(slots.as_ptr(), slots.len(), terminal, &mut index) } == 0 {
        Ok(index)
    } else { Err(()) }
}
pub(crate) fn deliver(level: u8, minimum_level: u8) -> bool {
    unsafe { ghostos_log_deliver(level, minimum_level) }
}
pub(crate) fn operator(level: u8, kind: u8) -> bool {
    unsafe { ghostos_log_operator(level, kind) }
}
pub(crate) fn dropped_delta(current: u64, previous: u64) -> u64 {
    unsafe { ghostos_log_dropped_delta(current, previous) }
}
pub(crate) fn remaining(budget: usize, used: usize) -> usize {
    unsafe { ghostos_log_remaining(budget, used) }
}
const _: () = {
    assert!(core::mem::size_of::<Subscription>() == 8);
    assert!(core::mem::offset_of!(Subscription, occupied) == 5);
};
unsafe extern "C" {
    fn ghostos_log_subscribe(slots: *const Subscription, count: usize, terminal: u32,
        minimum_level: u8, index: *mut usize) -> i32;
    fn ghostos_log_unsubscribe(slots: *const Subscription, count: usize, terminal: u32,
        index: *mut usize) -> i32;
    fn ghostos_log_deliver(level: u8, minimum_level: u8) -> bool;
    fn ghostos_log_operator(level: u8, kind: u8) -> bool;
    fn ghostos_log_dropped_delta(current: u64, previous: u64) -> u64;
    fn ghostos_log_remaining(budget: usize, used: usize) -> usize;
}
