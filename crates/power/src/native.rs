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
