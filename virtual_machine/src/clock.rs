//! Monotonic time shared by VM devices and host polling.

use std::cell::Cell;
use std::rc::Rc;

unsafe extern "C" {
    fn ghostos_vm_clock_now_ns() -> u64;
    fn ghostos_vm_clock_elapsed_ns(start_ns: u64) -> u64;
    fn ghostos_vm_manual_clock_set(clock: *mut u64, now_ns: u64) -> bool;
    fn ghostos_vm_manual_clock_advance(clock: *mut u64, elapsed_ns: u64) -> u64;
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
    started_ns: u64,
}

impl HostMonotonicClock {
    pub fn new() -> Self {
        Self {
            started_ns: unsafe { ghostos_vm_clock_now_ns() },
        }
    }
}

impl Default for HostMonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonotonicClock for HostMonotonicClock {
    fn now_ns(&self) -> u64 {
        unsafe { ghostos_vm_clock_elapsed_ns(self.started_ns) }
    }
}

/// Manually advanced clock for deterministic embedding and tests.
#[derive(Default)]
pub struct ManualMonotonicClock {
    now_ns: Cell<u64>,
}

impl ManualMonotonicClock {
    pub fn new(now_ns: u64) -> Self {
        Self {
            now_ns: Cell::new(now_ns),
        }
    }

    /// Move time forward. Panics if a caller tries to move it backwards.
    pub fn set_now_ns(&self, now_ns: u64) {
        assert!(
            unsafe { ghostos_vm_manual_clock_set(self.now_ns.as_ptr(), now_ns) },
            "monotonic clock cannot move backwards"
        )
    }

    pub fn advance_ns(&self, elapsed_ns: u64) {
        unsafe { ghostos_vm_manual_clock_advance(self.now_ns.as_ptr(), elapsed_ns) };
    }
}

impl MonotonicClock for ManualMonotonicClock {
    fn now_ns(&self) -> u64 {
        self.now_ns.get()
    }
}

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
