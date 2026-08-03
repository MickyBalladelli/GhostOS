//! High Precision Event Timer (HPET) emulation.
//!
//! Implements the Intel/AMD-compatible HPET with:
//!   * A 64-bit main counter driven by a nominal 10 MHz clock;
//!   * 32 timers, each with a 64-bit comparator, config register, and
//!     optional periodic mode;
//!   * Interrupt routing to the shared local APIC through the configured
//!     legacy-replacement I/O APIC vector;
//!   * The standard capability/configuration register block in the 1 MiB
//!     MMIO aperture.

use crate::devices::{ApicTrigger, Device, DeviceError, LocalApic};
use std::cell::RefCell;
use std::rc::Rc;

/// Default HPET MMIO base address (the classic ACPI HPET base).
pub const HPET_BASE_DEFAULT: u64 = 0xFED0_0000;

/// HPET MMIO aperture size (1 MiB per the HPET spec).
pub const HPET_SIZE: u64 = 0x1000;

/// Number of HPET timers implemented.
pub const HPET_TIMER_COUNT: usize = 32;

const HPET_CLK_PERIOD_NS: u64 = 100; // 10 MHz => 100 ns per tick

// CAP register layout.
const CAP_REVISION: u32 = 0x0001_0000;
const CAP_NUM_TIMERS_MASK: u32 = 0x1F00;
const CAP_COUNT_SIZE: u32 = 0x0020; // 64-bit counter
const CAP_LEGACY_ROUTE: u32 = 0x8000;

// CONFIG register.
const CONF_ENABLE: u32 = 0x1;
const CONF_LEGACY_ROUTE: u32 = 0x2;

const REG_CAP: u32 = 0x000;
const REG_CONFIG: u32 = 0x010;
const REG_IRQ_STATUS: u32 = 0x030;

// Timer config bits.
const TIMER_TYPE_PERIODIC: u32 = 0x4;
const TIMER_INT_ENABLE: u32 = 0x2;
const TIMER_INT_TYPE_LEVEL: u32 = 0x8;

struct HpetTimer {
    config: u32,
    comparator: u64,
    periodic_reload: u64,
    /// True when the timer is currently armed/periodic.
    running: bool,
}

impl HpetTimer {
    fn new() -> Self {
        Self {
            // Enabled, edge-triggered, individual interrupt routing, 32-bit
            // mode per the reset default on most hardware.
            config: 0,
            comparator: 0,
            periodic_reload: 0,
            running: false,
        }
    }
}

/// HPET device attached to the MMU at [`HPET_BASE_DEFAULT`].
pub struct Hpet {
    cap: u32,
    config: u32,
    counter: u64,
    timers: [HpetTimer; HPET_TIMER_COUNT],
    apic: Option<Rc<RefCell<LocalApic>>>,
    /// Legacy-replacement IRQ vector (I/O APIC IRQ 0) for timers 0 and 1.
    legacy_vector: u8,
    last_ns: Option<u64>,
}

impl Hpet {
    pub fn new() -> Self {
        Self {
            cap: CAP_REVISION
                | (((HPET_TIMER_COUNT as u32 - 1) << 8) & CAP_NUM_TIMERS_MASK)
                | CAP_COUNT_SIZE
                | CAP_LEGACY_ROUTE,
            config: 0,
            counter: 0,
            timers: std::array::from_fn(|_| HpetTimer::new()),
            apic: None,
            legacy_vector: 0,
            last_ns: None,
        }
    }

    /// Attach the shared local APIC that receives timer interrupts.
    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic);
    }

    /// Set the legacy-replacement interrupt vector used for timers 0 and 1.
    pub fn set_legacy_vector(&mut self, vector: u8) {
        self.legacy_vector = vector;
    }

    /// Advance the main counter to host time `now_ns` and fire due timers.
    pub fn advance(&mut self, now_ns: u64) {
        if self.config & CONF_ENABLE == 0 {
            self.last_ns = Some(now_ns);
            return;
        }

        let Some(last) = self.last_ns else {
            self.last_ns = Some(now_ns);
            return;
        };
        if now_ns <= last {
            return;
        }

        let elapsed = now_ns - last;
        self.last_ns = Some(now_ns);
        let ticks = elapsed / HPET_CLK_PERIOD_NS;
        if ticks == 0 {
            return;
        }

        self.counter = self.counter.wrapping_add(ticks);

        for (i, t) in self.timers.iter_mut().enumerate() {
            if !t.running || t.config & TIMER_INT_ENABLE == 0 {
                continue;
            }
            let periodic = t.config & TIMER_TYPE_PERIODIC != 0;
            let cmp = t.comparator;
            let mut interrupt = false;

            if self.counter >= cmp {
                interrupt = true;
                if periodic {
                    // Advance the comparator by whole periods until it's in
                    // the future again, then refill the reload count.
                    let period = t.periodic_reload.max(1);
                    let next = cmp + ((self.counter - cmp) / period + 1) * period;
                    t.comparator = next;
                    t.periodic_reload = period;
                } else {
                    t.running = false;
                }
            }

            // Deliver to the APIC (legacy-replacement if configured).
            if interrupt {
                let vector = if self.config & CONF_LEGACY_ROUTE != 0 && i < 2 {
                    self.legacy_vector
                } else {
                    // Simple fixed mapping: each timer i uses IRQ (i % 24)
                    // routed to vector base + IRQ.
                    let irq = (i as u8) % 24;
                    0x30u8.wrapping_add(irq)
                };
                if vector != 0 {
                    if let Some(apic) = &self.apic {
                        let edge = t.config & TIMER_INT_TYPE_LEVEL == 0;
                        apic.borrow_mut().signal(
                            vector,
                            if edge { ApicTrigger::Edge } else { ApicTrigger::Level },
                        );
                    }
                }
            }
        }
    }

    /// Reset to power-on state (keeps APIC attach and vector).
    pub fn reset(&mut self) {
        let apic = self.apic.take();
        let legacy_vector = self.legacy_vector;
        *self = Self::new();
        self.apic = apic;
        self.legacy_vector = legacy_vector;
    }

    fn read_u32(&self, off: u32) -> u32 {
        match off {
            REG_CAP => self.cap,
            REG_CONFIG => self.config,
            REG_IRQ_STATUS => {
                // Aggregate: report any timer whose comparator was passed and
                // is still running (periodic). Simple sticky bit simulation.
                let mut status = 0u32;
                for (i, t) in self.timers.iter().enumerate() {
                    if t.running && self.counter >= t.comparator {
                        status |= 1 << (i % 32);
                    }
                }
                status
            }
            0x100..=0x400 => {
                let idx = ((off - 0x100) / 0x20) as usize;
                if idx >= HPET_TIMER_COUNT {
                    return 0;
                }
                let sub = (off - 0x100) % 0x20;
                match sub {
                    0 => self.timers[idx].config,
                    4 => {
                        if self.timers[idx].config & TIMER_INT_TYPE_LEVEL != 0 {
                            // 32-bit mode: return low dword.
                            (self.timers[idx].comparator & 0xFFFF_FFFF) as u32
                        } else {
                            (self.timers[idx].comparator & 0xFFFF_FFFF) as u32
                        }
                    }
                    8 => {
                        if self.timers[idx].config & TIMER_INT_TYPE_LEVEL != 0 {
                            0
                        } else {
                            (self.timers[idx].comparator >> 32) as u32
                        }
                    }
                    0xC => 0,
                    0x10 => {
                        if self.timers[idx].config & TIMER_INT_TYPE_LEVEL != 0 {
                            (self.timers[idx].periodic_reload & 0xFFFF_FFFF) as u32
                        } else {
                            0
                        }
                    }
                    0x14 => {
                        if self.timers[idx].config & TIMER_INT_TYPE_LEVEL != 0 {
                            (self.timers[idx].periodic_reload >> 32) as u32
                        } else {
                            0
                        }
                    }
                    _ => 0,
                }
            }
            _ => 0,
        }
    }

    fn write_u32(&mut self, off: u32, value: u32) {
        match off {
            REG_CONFIG => {
                self.config = value & 0x3;
                if self.config & CONF_ENABLE != 0 {
                    self.last_ns = None;
                }
            }
            0x100..=0x400 => {
                let idx = ((off - 0x100) / 0x20) as usize;
                if idx >= HPET_TIMER_COUNT {
                    return;
                }
                let sub = (off - 0x100) % 0x20;
                match sub {
                    0 => {
                        let t = &mut self.timers[idx];
                        // Writable bits: 32/64-bit modes (0x20/0x40), level
                        // trigger (0x8), periodic (0x4), enable (0x2).
                        t.config = value & 0x7E;
                        if t.config & TIMER_TYPE_PERIODIC != 0 && t.config & TIMER_INT_ENABLE != 0 {
                            t.running = true;
                        } else {
                            t.running = t.config & TIMER_INT_ENABLE != 0;
                        }
                    }
                    4 => {
                        let t = &mut self.timers[idx];
                        // 32-bit mode: high dword ignored for comparator.
                        if t.config & TIMER_INT_TYPE_LEVEL != 0 {
                            t.comparator = (t.comparator & 0xFFFF_FFFF_0000_0000) | value as u64;
                        } else {
                            t.comparator = (t.comparator & 0xFFFF_FFFF_0000_0000) | value as u64;
                        }
                        t.running = t.config & TIMER_INT_ENABLE != 0;
                    }
                    8 => {
                        let t = &mut self.timers[idx];
                        if t.config & TIMER_INT_TYPE_LEVEL == 0 {
                            t.comparator =
                                (t.comparator & 0xFFFF_FFFF) | ((value as u64) << 32);
                        } else {
                            // 32-bit mode: high dword write is ignored.
                        }
                    }
                    0xC => {
                        // 64-bit comparator low write in 32-bit mode aliases
                        // to the low dword only.
                        let t = &mut self.timers[idx];
                        t.comparator = (t.comparator & 0xFFFF_FFFF_0000_0000) | value as u64;
                    }
                    0x10 | 0x14 => {
                        let t = &mut self.timers[idx];
                        let is_high = sub == 0x14;
                        if t.config & TIMER_TYPE_PERIODIC != 0 {
                            t.periodic_reload = if is_high {
                                (t.periodic_reload & 0xFFFF_FFFF) | ((value as u64) << 32)
                            } else {
                                (t.periodic_reload & 0xFFFF_FFFF_0000_0000) | value as u64
                            };
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

impl Default for Hpet {
    fn default() -> Self {
        Self::new()
    }
}

impl Device for Hpet {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        if size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = (addr as u32) & 0xFFF;
        Ok(self.read_u32(off) as u64)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = (addr as u32) & 0xFFF;
        self.write_u32(off, value as u32);
        Ok(())
    }

    fn reset(&mut self) {
        Hpet::reset(self);
    }
}

impl Device for Rc<RefCell<Hpet>> {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        Device::read(&*self.borrow(), addr, size)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        Device::write(&mut *self.borrow_mut(), addr, value, size)
    }

    fn reset(&mut self) {
        self.borrow_mut().reset();
    }
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
        assert_eq!(h.cap & CAP_REVISION, 0x0001_0000);
        assert_eq!((h.cap & CAP_NUM_TIMERS_MASK) >> 8, 31);
        assert_ne!(h.cap & CAP_COUNT_SIZE, 0);
        assert_ne!(h.cap & CAP_LEGACY_ROUTE, 0);
    }

    #[test]
    fn counter_runs_only_when_enabled() {
        let mut h = hpet();
        h.advance(0);
        h.advance(100_000); // 1000 ticks
        assert_eq!(h.counter, 0, "counter must not run while disabled");

        wr(&mut h, REG_CONFIG, 1); // enable
        h.advance(0);
        h.advance(100_000);
        assert_eq!(h.counter, 1000);
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
