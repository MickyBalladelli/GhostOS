#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Slot {
    pub service: u64,
    pub process: u64,
    pub occupied: bool,
    pub has_snapshot: bool,
}
pub(crate) fn service(service: u64) -> Result<(), crate::HealthError> {
    match unsafe { ghostos_health_service(service) } {
        0 => Ok(()),
        2 => Err(crate::HealthError::InvalidService),
        _ => unreachable!("native health service"),
    }
}
pub(crate) fn duplicate(services: &[u64], service: u64) -> bool {
    unsafe { ghostos_health_duplicate(services.as_ptr(), services.len(), service) == 1 }
}
pub(crate) fn registration(previous: u64) -> u64 {
    unsafe { ghostos_health_registration(previous) }
}
pub(crate) fn mark_time(previous: u64, progress: u64, was_seen: bool) -> bool {
    unsafe { ghostos_health_mark_time(previous, progress, was_seen) }
}
pub(crate) fn fault(memory_corrupt: bool, memory_checked: bool, expected_first: u64,
    checksum: u64, expected_second: u64, compared: bool, heartbeat_seen: bool, now_us: u64,
    heartbeat_at_us: u64, heartbeat_timeout_us: u64, have_heartbeat_timeout: bool,
    driver_seen: bool, driver_at_us: u64, driver_timeout_us: u64, have_driver: bool,
    progress_seen: bool, progress_at_us: u64, progress_timeout_us: u64, have_progress: bool,
) -> u8 {
    unsafe { ghostos_health_fault(memory_corrupt, memory_checked, expected_first, checksum,
        expected_second, compared, heartbeat_seen, now_us, heartbeat_at_us, heartbeat_timeout_us,
        have_heartbeat_timeout, driver_seen, driver_at_us, driver_timeout_us, have_driver,
        progress_seen, progress_at_us, progress_timeout_us, have_progress) as u8 }
}
fn recovery<T>(code: i32) -> Result<T, super::RecoveryError> {
    Err(match code {
        1 => super::RecoveryError::Capacity,
        2 => super::RecoveryError::InvalidService,
        3 => super::RecoveryError::AlreadyRegistered,
        4 => super::RecoveryError::NotFound,
        5 => super::RecoveryError::StaleProcess,
        6 => super::RecoveryError::NoSnapshot,
        7 => super::RecoveryError::InvalidProcess,
        _ => unreachable!("native recovery result"),
    })
}
pub(crate) fn recover_index(code: i32, index: usize) -> Result<usize, super::RecoveryError> {
    if code == 0 { Ok(index) } else { recovery(code) }
}
pub(crate) fn register(slots: &[Slot], service: u64, image: u128, process: u64) -> Result<usize, super::RecoveryError> {
    let mut index = 0;
    recover_index(unsafe { ghostos_heal_register(slots.as_ptr(), slots.len(), service,
        (image >> 64) as u64, image as u64, process, &mut index) }, index)
}
pub(crate) fn find(slots: &[Slot], service: u64) -> Result<usize, super::RecoveryError> {
    let mut index = 0;
    recover_index(unsafe { ghostos_heal_find(slots.as_ptr(), slots.len(), service, &mut index) }, index)
}
pub(crate) fn set_process(slots: &[Slot], service: u64, process: u64) -> Result<usize, super::RecoveryError> {
    let mut index = 0;
    recover_index(unsafe { ghostos_heal_set_process(slots.as_ptr(), slots.len(), service, process, &mut index) }, index)
}
pub(crate) fn prepare(slots: &[Slot], service: u64, crashed: u64) -> Result<usize, super::RecoveryError> {
    let mut index = 0;
    recover_index(unsafe { ghostos_heal_prepare(slots.as_ptr(), slots.len(), service, crashed, &mut index) }, index)
}
pub(crate) fn generation(generation: u32) -> u32 {
    unsafe { ghostos_heal_generation(generation) }
}
pub(crate) fn accept_process(process: u64, crashed: u64) -> Result<(), super::RecoveryError> {
    let code = unsafe { ghostos_heal_accept_process(process, crashed) };
    if code == 0 { Ok(()) } else { recovery(code) }
}
const _: () = {
    assert!(core::mem::size_of::<Slot>() == 24);
    assert!(core::mem::offset_of!(Slot, occupied) == 16);
};
unsafe extern "C" {
    fn ghostos_health_service(service: u64) -> i32;
    fn ghostos_health_duplicate(services: *const u64, count: usize, service: u64) -> i32;
    fn ghostos_health_registration(previous: u64) -> u64;
    fn ghostos_health_mark_time(previous: u64, progress: u64, was_seen: bool) -> bool;
    fn ghostos_health_fault(memory_corrupt: bool, memory_checked: bool, expected_first: u64,
        checksum: u64, expected_second: u64, compared: bool, heartbeat_seen: bool, now_us: u64,
        heartbeat_at_us: u64, heartbeat_timeout_us: u64, have_heartbeat_timeout: bool,
        driver_seen: bool, driver_at_us: u64, driver_timeout_us: u64, have_driver: bool,
        progress_seen: bool, progress_at_us: u64, progress_timeout_us: u64, have_progress: bool) -> i32;
    fn ghostos_heal_register(slots: *const Slot, count: usize, service: u64, image_high: u64,
        image_low: u64, process: u64, index: *mut usize) -> i32;
    fn ghostos_heal_find(slots: *const Slot, count: usize, service: u64, index: *mut usize) -> i32;
    fn ghostos_heal_set_process(slots: *const Slot, count: usize, service: u64, process: u64,
        index: *mut usize) -> i32;
    fn ghostos_heal_prepare(slots: *const Slot, count: usize, service: u64, crashed: u64,
        index: *mut usize) -> i32;
    fn ghostos_heal_generation(generation: u32) -> u32;
    fn ghostos_heal_accept_process(process: u64, crashed: u64) -> i32;
}
