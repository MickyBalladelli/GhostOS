use crate::{ThermalAction, ThermalTripPoints};
#[repr(C)]
pub(crate) struct Log { pub next: usize, pub len: usize, pub dropped: u64 }
impl Log {
    pub const EMPTY: Self = Self { next: 0, len: 0, dropped: 0 };
    pub fn push(&mut self, capacity: usize) -> Option<usize> {
        let mut slot = 0;
        unsafe { ghostos_thermal_push(self, capacity, &mut slot) }.then_some(slot)
    }
    pub fn drain(&mut self, capacity: usize, length: usize) -> (usize, usize) {
        let mut first = 0;
        let count = unsafe { ghostos_thermal_drain(self, capacity, length, &mut first) };
        (first, count)
    }
}
#[repr(C)]
struct Trips { passive: u32, hot: u32, critical: u32, present: u8 }
pub(crate) fn decide(trips: ThermalTripPoints, hysteresis: u32,
    temperature: u32, previous: ThermalAction) -> (ThermalAction, u8) {
    let native = Trips {
        passive: trips.passive_deci_kelvin.unwrap_or(0),
        hot: trips.hot_deci_kelvin.unwrap_or(0),
        critical: trips.critical_deci_kelvin.unwrap_or(0),
        present: u8::from(trips.passive_deci_kelvin.is_some()) |
            (u8::from(trips.hot_deci_kelvin.is_some()) << 1) |
            (u8::from(trips.critical_deci_kelvin.is_some()) << 2),
    };
    let mut action = 0;
    let mut percent = 0;
    let event = unsafe { ghostos_thermal_decide(&native, hysteresis, temperature,
        previous.code(), previous.throttle_percent(), &mut action, &mut percent) };
    let action = match action {
        0 => ThermalAction::Normal,
        1 => ThermalAction::Throttle { percent },
        2 => ThermalAction::EmergencyShutdown,
        _ => unreachable!("native thermal action"),
    };
    (action, event)
}
const _: () = {
    assert!(core::mem::size_of::<Trips>() == 16);
    assert!(core::mem::offset_of!(Trips, present) == 12);
    assert!(core::mem::offset_of!(Log, dropped) == 2 * core::mem::size_of::<usize>());
};
unsafe extern "C" {
    fn ghostos_thermal_decide(trips: *const Trips, hysteresis: u32,
        temperature: u32, previous: u8, previous_percent: u8,
        action: *mut u8, percent: *mut u8) -> u8;
    fn ghostos_thermal_push(log: *mut Log, capacity: usize, slot: *mut usize) -> bool;
    fn ghostos_thermal_drain(log: *mut Log, capacity: usize,
        destination_length: usize, first: *mut usize) -> usize;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Candidate {
    pub cpus: [u64; 2], pub idle_power_mw: u32,
    pub id: u8, pub load: u8, pub throttle: u8, pub valid: bool,
}
pub(crate) fn frequency(minimum: u32, maximum: u32, load: u8, throttle: u8) -> u32 {
    unsafe { ghostos_power_frequency(minimum, maximum, load, throttle) }
}
pub(crate) fn idle(now: u64, wake: u64, budget: u64) -> crate::CpuIdleState {
    match unsafe { ghostos_power_idle(now, wake, budget) } {
        0 => crate::CpuIdleState::C0,
        1 => crate::CpuIdleState::C1,
        2 => crate::CpuIdleState::C2,
        3 => crate::CpuIdleState::C3,
        _ => unreachable!("native idle state"),
    }
}
pub(crate) fn device(now: u64, active: u64, idle: u64, suspend: u64) -> crate::DevicePowerState {
    match unsafe { ghostos_power_device(now, active, idle, suspend) } {
        0 => crate::DevicePowerState::Active,
        1 => crate::DevicePowerState::RuntimeIdle,
        2 => crate::DevicePowerState::Suspended,
        _ => unreachable!("native device state"),
    }
}
pub(crate) fn place(clusters: &[Candidate], request: crate::WorkloadRequest) -> Result<usize, crate::PowerPolicyError> {
    let affinity = request.affinity.raw_words();
    let mut selected = 0;
    match unsafe { ghostos_power_place(clusters.as_ptr(), clusters.len(), affinity.as_ptr(),
        request.class as u8, request.preferred_cluster.is_some(), request.preferred_cluster.unwrap_or(0),
        cfg!(debug_assertions), &mut selected) } {
        0 => Ok(selected),
        1 => Err(crate::PowerPolicyError::NoCluster),
        2 => panic!("attempt to add with overflow"),
        _ => unreachable!("native power placement"),
    }
}
const _: () = {
    assert!(core::mem::size_of::<Candidate>() == 24);
    assert!(core::mem::offset_of!(Candidate, valid) == 23);
};
unsafe extern "C" {
    fn ghostos_power_frequency(minimum: u32, maximum: u32, load: u8, throttle: u8) -> u32;
    fn ghostos_power_idle(now: u64, wake: u64, budget: u64) -> u8;
    fn ghostos_power_device(now: u64, active: u64, idle: u64, suspend: u64) -> u8;
    fn ghostos_power_place(clusters: *const Candidate, count: usize, affinity: *const u64,
        class_id: u8, preferred: bool, preferred_id: u8, checked: bool, selected: *mut usize) -> i32;
}
