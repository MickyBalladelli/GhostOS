//! Monotonic time shared by VM devices and host polling.

use std::rc::Rc;

#[repr(C)]
struct CHostClock {
    _private: [u8; 0],
}

#[repr(C)]
struct CManualClock {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn ghostos_vm_host_clock_new() -> *mut CHostClock;
    fn ghostos_vm_host_clock_free(clock: *mut CHostClock);
    fn ghostos_vm_host_clock_now(clock: *const CHostClock) -> u64;
    fn ghostos_vm_manual_clock_new(now_ns: u64) -> *mut CManualClock;
    fn ghostos_vm_manual_clock_free(clock: *mut CManualClock);
    fn ghostos_vm_manual_clock_set_state(clock: *mut CManualClock, now_ns: u64) -> bool;
    fn ghostos_vm_manual_clock_advance_state(clock: *mut CManualClock, elapsed_ns: u64) -> u64;
    fn ghostos_vm_manual_clock_now(clock: *const CManualClock) -> u64;
}

/// Source of monotonic nanoseconds for VM time.
///
/// Time must never move backwards. A VM receives one shared clock, so its
/// timers, pvclock updates, terminal polling, and execution loop agree about
/// when an event happened.
pub trait MonotonicClock {
    fn now_ns(&self) -> u64;
}

pub type SharedMonotonicClock = Rc<dyn MonotonicClock>;

/// Host-backed clock used by normal VM runs.
pub struct HostMonotonicClock {
    state: *mut CHostClock,
}

impl HostMonotonicClock {
    pub fn new() -> Self {
        let state = unsafe { ghostos_vm_host_clock_new() };
        assert!(!state.is_null(), "could not allocate host monotonic clock");
        Self { state }
    }
}

impl Default for HostMonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonotonicClock for HostMonotonicClock {
    fn now_ns(&self) -> u64 {
        unsafe { ghostos_vm_host_clock_now(self.state) }
    }
}

impl Drop for HostMonotonicClock {
    fn drop(&mut self) {
        unsafe { ghostos_vm_host_clock_free(self.state) }
    }
}

unsafe impl Send for HostMonotonicClock {}
unsafe impl Sync for HostMonotonicClock {}

/// Manually advanced clock for deterministic embedding and tests.
pub struct ManualMonotonicClock {
    state: *mut CManualClock,
}

impl ManualMonotonicClock {
    pub fn new(now_ns: u64) -> Self {
        let state = unsafe { ghostos_vm_manual_clock_new(now_ns) };
        assert!(!state.is_null(), "could not allocate manual monotonic clock");
        Self { state }
    }

    /// Move time forward. Panics if a caller tries to move it backwards.
    pub fn set_now_ns(&self, now_ns: u64) {
        assert!(
            unsafe { ghostos_vm_manual_clock_set_state(self.state, now_ns) },
            "monotonic clock cannot move backwards"
        )
    }

    pub fn advance_ns(&self, elapsed_ns: u64) {
        unsafe { ghostos_vm_manual_clock_advance_state(self.state, elapsed_ns) };
    }
}

impl MonotonicClock for ManualMonotonicClock {
    fn now_ns(&self) -> u64 {
        unsafe { ghostos_vm_manual_clock_now(self.state) }
    }
}

impl Default for ManualMonotonicClock {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Drop for ManualMonotonicClock {
    fn drop(&mut self) {
        unsafe { ghostos_vm_manual_clock_free(self.state) }
    }
}

unsafe impl Send for ManualMonotonicClock {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_only_moves_forward() {
        let clock = ManualMonotonicClock::new(10);
        clock.advance_ns(5);
        assert_eq!(clock.now_ns(), 15);
    }
}
