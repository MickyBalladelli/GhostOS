//! HPET adapter. C owns register state, comparator scheduling, and IRQ routing.

use crate::devices::{ApicTrigger, Device, DeviceError, LocalApic};
use std::cell::RefCell;
use std::rc::Rc;
use std::ffi::c_void;

pub const HPET_BASE_DEFAULT: u64 = 0xFED0_0000;
pub const HPET_SIZE: u64 = 0x1000;
pub const HPET_TIMER_COUNT: usize = 32;

#[cfg(test)]
const CAP_REVISION: u32 = 0x0001_0000;
#[cfg(test)]
const CAP_NUM_TIMERS_MASK: u32 = 0x1F00;
#[cfg(test)]
const CAP_COUNT_SIZE: u32 = 0x0020;
#[cfg(test)]
const CAP_LEGACY_ROUTE: u32 = 0x8000;
#[cfg(test)]
const REG_CONFIG: u32 = 0x010;

#[repr(C)]
struct HpetTimer {
    config: u32,
    comparator: u64,
    periodic_reload: u64,
    running: bool,
}

#[repr(C)]
struct HpetState {
    cap: u32,
    config: u32,
    counter: u64,
    timers: [HpetTimer; HPET_TIMER_COUNT],
    legacy_vector: u8,
    last_ns: u64,
    seeded: bool,
}

pub struct Hpet {
    state: HpetState,
    apic: Option<Rc<RefCell<LocalApic>>>,
}

unsafe extern "C" {
    fn ghostos_vm_hpet_size() -> usize;
    fn ghostos_vm_hpet_init(hpet: *mut HpetState);
    fn ghostos_vm_hpet_reset(hpet: *mut HpetState);
    fn ghostos_vm_hpet_advance(hpet: *mut HpetState, now_ns: u64,
        irq: Option<unsafe extern "C" fn(*mut c_void, u8, bool)>, context: *mut c_void);
    fn ghostos_vm_hpet_read(hpet: *const HpetState, address: u64) -> u32;
    fn ghostos_vm_hpet_write(hpet: *mut HpetState, address: u64, value: u32);
}

unsafe extern "C" fn signal_apic(context: *mut c_void, vector: u8, level: bool) {
    unsafe { &mut *context.cast::<LocalApic>() }.signal(
        vector, if level { ApicTrigger::Level } else { ApicTrigger::Edge },
    );
}

impl Hpet {
    pub fn new() -> Self {
        assert_eq!(std::mem::size_of::<HpetState>(), unsafe { ghostos_vm_hpet_size() });
        let mut state = std::mem::MaybeUninit::<HpetState>::uninit();
        unsafe { ghostos_vm_hpet_init(state.as_mut_ptr()) };
        Self { state: unsafe { state.assume_init() }, apic: None }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic)
    }

    pub fn set_legacy_vector(&mut self, vector: u8) {
        self.state.legacy_vector = vector
    }

    pub fn advance(&mut self, now_ns: u64) {
        // Borrow before crossing C so RefCell panics cannot unwind through C.
        let mut apic = self.apic.as_ref().map(|apic| apic.borrow_mut());
        let context = apic.as_mut().map_or(std::ptr::null_mut(), |apic| {
            (&mut **apic as *mut LocalApic).cast::<c_void>()
        });
        let irq = if context.is_null() { None } else {
            Some(signal_apic as unsafe extern "C" fn(*mut c_void, u8, bool))
        };
        unsafe { ghostos_vm_hpet_advance(&mut self.state, now_ns, irq, context) }
    }

    pub fn reset(&mut self) {
        unsafe { ghostos_vm_hpet_reset(&mut self.state) }
    }
}

impl Default for Hpet {
    fn default() -> Self { Self::new() }
}

impl Device for Hpet {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        if size != 4 { return Err(DeviceError::UnsupportedSize) }
        Ok(u64::from(unsafe { ghostos_vm_hpet_read(&self.state, addr) }))
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 4 { return Err(DeviceError::UnsupportedSize) }
        unsafe { ghostos_vm_hpet_write(&mut self.state, addr, value as u32) };
        Ok(())
    }

    fn reset(&mut self) { Hpet::reset(self) }
}

impl Device for Rc<RefCell<Hpet>> {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        Device::read(&*self.borrow(), addr, size)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        Device::write(&mut *self.borrow_mut(), addr, value, size)
    }

    fn reset(&mut self) { self.borrow_mut().reset() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::LocalApic;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn hpet() -> Hpet {
        Hpet::new()
    }

    /// Write to a 32-bit register at a given byte offset.
    fn wr(h: &mut Hpet, off: u32, value: u32) {
        Device::write(h, HPET_BASE_DEFAULT + off as u64, value as u64, 4).unwrap();
    }

    #[test]
    fn capability_and_revision() {
        let h = hpet();
        // Revision 0x0001_0000, 32 timers (31 << 8), 64-bit counter, legacy route.
        assert_eq!(h.state.cap & CAP_REVISION, 0x0001_0000);
        assert_eq!((h.state.cap & CAP_NUM_TIMERS_MASK) >> 8, 31);
        assert_ne!(h.state.cap & CAP_COUNT_SIZE, 0);
        assert_ne!(h.state.cap & CAP_LEGACY_ROUTE, 0);
    }

    #[test]
    fn counter_runs_only_when_enabled() {
        let mut h = hpet();
        h.advance(0);
        h.advance(100_000); // 1000 ticks
        assert_eq!(h.state.counter, 0, "counter must not run while disabled");

        wr(&mut h, REG_CONFIG, 1); // enable
        h.advance(0);
        h.advance(100_000);
        assert_eq!(h.state.counter, 1000);
    }

    #[test]
    fn timer_fires_apic_on_comparator() {
        let mut h = hpet();
        let apic = Rc::new(RefCell::new(LocalApic::new(0)));
        h.attach_apic(apic.clone());
        h.set_legacy_vector(0x50);
        wr(&mut h, REG_CONFIG, 0x3); // enable + legacy route 0/1 -> IRQ0/IRQ1

        // Timer 0: periodic, 32-bit, enable, edge to legacy IRQ 0.
        wr(&mut h, 0x100 + 0, 0x4 | 0x2); // periodic + enable
        wr(&mut h, 0x100 + 0x10, 200); // periodic reload = 200 ticks
        wr(&mut h, 0x100 + 4, 200); // comparator = 200

        h.advance(0);
        h.advance(20_000); // 200 ticks
        assert_eq!(apic.borrow_mut().pending_vector(), Some(0x50));
    }

    #[test]
    fn timer_one_shot_fires_once() {
        let mut h = hpet();
        let apic = Rc::new(RefCell::new(LocalApic::new(0)));
        h.attach_apic(apic.clone());
        h.set_legacy_vector(0x51);
        wr(&mut h, REG_CONFIG, 0x3);

        // One-shot: enable only (no periodic bit).
        wr(&mut h, 0x100 + 0, 0x2);
        wr(&mut h, 0x100 + 4, 100);

        h.advance(0);
        h.advance(10_000);
        assert_eq!(apic.borrow_mut().pending_vector(), Some(0x51));

        apic.borrow_mut().accept_pending(0x51);
        wr(&mut h, 0x100 + 4, 200);
        h.advance(30_000);
        assert_eq!(
            apic.borrow_mut().pending_vector(),
            None,
            "one-shot timer should not re-fire"
        );
    }

    #[test]
    fn periodic_timer_repeats() {
        let mut h = hpet();
        let apic = Rc::new(RefCell::new(LocalApic::new(0)));
        h.attach_apic(apic.clone());
        h.set_legacy_vector(0x52);
        wr(&mut h, REG_CONFIG, 0x3);

        wr(&mut h, 0x100 + 0, 0x4 | 0x2);
        wr(&mut h, 0x100 + 0x10, 100);
        wr(&mut h, 0x100 + 4, 100);

        h.advance(0);
        h.advance(10_000);
        assert!(apic.borrow_mut().pending_vector().is_some());
        apic.borrow_mut().accept_pending(0x52);
        apic.borrow_mut().eoi();
        // 99 more ticks: counter reaches 199, just under the next
        // comparator at 200.
        h.advance(19_900);
        assert_eq!(apic.borrow_mut().pending_vector(), None);
        // Another 200 ns = 2 ticks crosses the second period.
        h.advance(20_100);
        assert!(apic.borrow_mut().pending_vector().is_some());
    }

    #[test]
    fn disabled_timer_does_not_fire() {
        let mut h = hpet();
        let apic = Rc::new(RefCell::new(LocalApic::new(0)));
        h.attach_apic(apic.clone());
        h.set_legacy_vector(0x53);
        wr(&mut h, REG_CONFIG, 0x3);
        h.advance(0);
        h.advance(10_000_000); // 100k ticks
        assert_eq!(apic.borrow_mut().pending_vector(), None);
    }

    #[test]
    fn dword_access_enforced() {
        let h = hpet();
        assert_eq!(
            Device::read(&h, HPET_BASE_DEFAULT + 4, 1),
            Err(DeviceError::UnsupportedSize)
        );
    }
}
