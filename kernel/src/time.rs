//! Kernel timekeeper shared by interrupt handling and user services.

unsafe extern "C" {
    fn ghostos_time_initialize();
    fn ghostos_time_advance_monotonic(elapsed_us: u64);
    fn ghostos_time_timer_tick() -> u64;
    fn ghostos_time_monotonic_now_us() -> u64;
    fn ghostos_time_realtime_ready() -> bool;
    fn ghostos_time_realtime_now_ns(now_ns: *mut u64) -> bool;
}

pub fn initialize() {
    unsafe { ghostos_time_initialize() }
}

pub fn advance_monotonic(elapsed_us: u64) {
    unsafe { ghostos_time_advance_monotonic(elapsed_us) }
}

pub fn timer_tick() -> u64 {
    unsafe { ghostos_time_timer_tick() }
}

pub fn monotonic_now_us() -> u64 {
    unsafe { ghostos_time_monotonic_now_us() }
}

pub fn realtime_ready() -> bool {
    unsafe { ghostos_time_realtime_ready() }
}

pub fn realtime_now_ns() -> Option<u64> {
    let mut now_ns = 0;
    unsafe { ghostos_time_realtime_now_ns(&mut now_ns).then_some(now_ns) }
}
