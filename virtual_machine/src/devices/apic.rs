//! Local APIC (xAPIC) emulation.
//!
//! Implements the memory-mapped xAPIC register file at the conventional
//! [`APIC_BASE_DEFAULT`] address (4 KiB aperture, 16-byte-aligned dword
//! registers), the `IA32_APIC_BASE` MSR (0x1B), interrupt prioritisation
//! through IRR/ISR/TMR, TPR/PPR filtering, EOI, the interrupt command
//! register (ICR) for IPIs, the six-entry LVT table, and a count-down timer
//! with a programmable divide configuration.
//!
//! This is a single-CPU (BSP) emulation: IPIs targeting self (or any
//! "all" shorthand) are delivered to the one local APIC in the machine.

use crate::devices::{Device, DeviceError};
use std::cell::RefCell;
use std::rc::Rc;

/// Default local APIC base address (xAPIC memory-mapped register base).
pub const APIC_BASE_DEFAULT: u64 = 0xFEE0_0000;

/// Size of the xAPIC memory aperture (all registers live in 4 KiB).
pub const APIC_SIZE: u64 = 0x1000;

/// `IA32_APIC_BASE` MSR number.
pub const IA32_APIC_BASE_MSR: u32 = 0x1B;

// ---------------------------------------------------------------------------
// Register offsets (xAPIC, 16-byte aligned dwords)
// ---------------------------------------------------------------------------

#[cfg(test)]
const REG_ID: u32 = 0x020;
#[cfg(test)]
const REG_VERSION: u32 = 0x030;
#[cfg(test)]
const REG_TPR: u32 = 0x080;
#[cfg(test)]
const REG_PPR: u32 = 0x0A0;
#[cfg(test)]
const REG_EOI: u32 = 0x0B0;
#[cfg(test)]
const REG_SVR: u32 = 0x0F0;
#[cfg(test)]
const REG_ICR_LO: u32 = 0x310;
#[cfg(test)]
const REG_LVT_TIMER: u32 = 0x320;
#[cfg(test)]
const REG_LVT_THERMAL: u32 = 0x330;
#[cfg(test)]
const REG_LVT_PERFMON: u32 = 0x340;
#[cfg(test)]
const REG_LVT_LINT0: u32 = 0x350;
#[cfg(test)]
const REG_LVT_LINT1: u32 = 0x360;
#[cfg(test)]
const REG_LVT_ERROR: u32 = 0x370;
#[cfg(test)]
const REG_TIMER_INIT_COUNT: u32 = 0x380;
#[cfg(test)]
const REG_TIMER_CURRENT_COUNT: u32 = 0x390;
#[cfg(test)]
const REG_TIMER_DIVIDE: u32 = 0x3E0;

// ---------------------------------------------------------------------------
// Field / flag masks
// ---------------------------------------------------------------------------

/// LVT mask bit (interrupt masked).
pub const LVT_MASK: u32 = 1 << 16;
#[cfg(test)]
const LVT_PERIODIC: u32 = 1 << 17;

#[cfg(test)]
const SVR_ENABLE: u32 = 1 << 8;

#[cfg(test)]
const ICR_SELF: u32 = 0x1 << 18;

#[cfg(test)]
const DELIVERY_FIXED: u32 = 0;
#[cfg(test)]
const DELIVERY_NMI: u32 = 0x4 << 8;

/// Interrupt trigger mode used by [`LocalApic::signal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApicTrigger {
    Edge,
    Level,
}

type VectorTable = [u32; 8];

#[cfg(test)]
fn table_index(vector: u8) -> usize { (vector / 32) as usize }

#[cfg(test)]
fn table_bit(vector: u8) -> u32 { 1u32 << (vector % 32) }

/// Local APIC (xAPIC) emulation for the BSP.
#[repr(C)]
pub struct LocalApic {
    /// The `IA32_APIC_BASE` base address currently programmed.
    base: u64,
    /// `true` when the APIC is enabled via `IA32_APIC_BASE` bits 10:8.
    enabled: bool,
    /// Local APIC ID (field in bits 27:24).
    id: u32,
    version: u32,
    tpr: u32,
    ppr: u32,
    ldr: u32,
    dfr: u32,
    svr: u32,
    irr: VectorTable,
    isr: VectorTable,
    tmr: VectorTable,
    /// Vectors that were signalled level-triggered and not yet accepted.
    level_pending: VectorTable,
    esr: u32,
    icr_hi: u32,
    icr_lo: u32,
    lvt_timer: u32,
    lvt_thermal: u32,
    lvt_perfmon: u32,
    lvt_lint0: u32,
    lvt_lint1: u32,
    lvt_error: u32,
    timer_initial_count: u32,
    timer_current_count: u32,
    timer_divide: u32,
    /// `true` while the count-down timer is running.
    timer_running: bool,
    /// Host timestamp of the last timer advance; ignored until seeded.
    timer_last_ns: u64,
    timer_seeded: bool,
    /// Set when the timer has wrapped to zero and the LVT vector is pending.
    timer_fired: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalApicState {
    pub base: u64,
    pub enabled: bool,
    pub id: u32,
    pub version: u32,
    pub tpr: u32,
    pub ppr: u32,
    pub ldr: u32,
    pub dfr: u32,
    pub svr: u32,
    pub irr: [u32; 8],
    pub isr: [u32; 8],
    pub tmr: [u32; 8],
    pub level_pending: [u32; 8],
    pub esr: u32,
    pub icr_hi: u32,
    pub icr_lo: u32,
    pub lvt_timer: u32,
    pub lvt_thermal: u32,
    pub lvt_perfmon: u32,
    pub lvt_lint0: u32,
    pub lvt_lint1: u32,
    pub lvt_error: u32,
    pub timer_initial_count: u32,
    pub timer_current_count: u32,
    pub timer_divide: u32,
    pub timer_running: bool,
    pub timer_last_ns: Option<u64>,
    pub timer_fired: bool,
}

unsafe extern "C" {
    fn ghostos_vm_apic_size() -> usize;
    fn ghostos_vm_apic_init(apic: *mut LocalApic, id: u8);
    fn ghostos_vm_apic_base_msr(apic: *const LocalApic) -> u64;
    fn ghostos_vm_apic_set_base_msr(apic: *mut LocalApic, value: u64);
    fn ghostos_vm_apic_signal(apic: *mut LocalApic, vector: u8, level: bool);
    fn ghostos_vm_apic_pending(apic: *mut LocalApic) -> i32;
    fn ghostos_vm_apic_in_service(apic: *const LocalApic) -> i32;
    fn ghostos_vm_apic_accept(apic: *mut LocalApic, vector: u8);
    fn ghostos_vm_apic_eoi(apic: *mut LocalApic);
    fn ghostos_vm_apic_advance(apic: *mut LocalApic, now_ns: u64);
    fn ghostos_vm_apic_read(apic: *const LocalApic, address: u64) -> u32;
    fn ghostos_vm_apic_write(apic: *mut LocalApic, address: u64, value: u32);
}

impl LocalApic {
    /// Create a local APIC with APIC ID `id` (0 for the BSP).
    pub fn new(id: u8) -> Self {
        assert_eq!(std::mem::size_of::<Self>(), unsafe { ghostos_vm_apic_size() });
        let mut state = std::mem::MaybeUninit::<Self>::uninit();
        unsafe {
            ghostos_vm_apic_init(state.as_mut_ptr(), id);
            state.assume_init()
        }
    }

    /// Current value of the `IA32_APIC_BASE` MSR.
    pub fn apic_base_msr(&self) -> u64 {
        unsafe { ghostos_vm_apic_base_msr(self) }
    }

    pub(crate) fn snapshot_state(&self) -> LocalApicState {
        LocalApicState {
            base: self.base,
            enabled: self.enabled,
            id: self.id,
            version: self.version,
            tpr: self.tpr,
            ppr: self.ppr,
            ldr: self.ldr,
            dfr: self.dfr,
            svr: self.svr,
            irr: self.irr,
            isr: self.isr,
            tmr: self.tmr,
            level_pending: self.level_pending,
            esr: self.esr,
            icr_hi: self.icr_hi,
            icr_lo: self.icr_lo,
            lvt_timer: self.lvt_timer,
            lvt_thermal: self.lvt_thermal,
            lvt_perfmon: self.lvt_perfmon,
            lvt_lint0: self.lvt_lint0,
            lvt_lint1: self.lvt_lint1,
            lvt_error: self.lvt_error,
            timer_initial_count: self.timer_initial_count,
            timer_current_count: self.timer_current_count,
            timer_divide: self.timer_divide,
            timer_running: self.timer_running,
            timer_last_ns: self.timer_seeded.then_some(self.timer_last_ns),
            timer_fired: self.timer_fired,
        }
    }

    pub(crate) fn restore_state(&mut self, state: &LocalApicState) {
        self.base = state.base;
        self.enabled = state.enabled;
        self.id = state.id;
        self.version = state.version;
        self.tpr = state.tpr;
        self.ppr = state.ppr;
        self.ldr = state.ldr;
        self.dfr = state.dfr;
        self.svr = state.svr;
        self.irr = state.irr;
        self.isr = state.isr;
        self.tmr = state.tmr;
        self.level_pending = state.level_pending;
        self.esr = state.esr;
        self.icr_hi = state.icr_hi;
        self.icr_lo = state.icr_lo;
        self.lvt_timer = state.lvt_timer;
        self.lvt_thermal = state.lvt_thermal;
        self.lvt_perfmon = state.lvt_perfmon;
        self.lvt_lint0 = state.lvt_lint0;
        self.lvt_lint1 = state.lvt_lint1;
        self.lvt_error = state.lvt_error;
        self.timer_initial_count = state.timer_initial_count;
        self.timer_current_count = state.timer_current_count;
        self.timer_divide = state.timer_divide;
        self.timer_running = state.timer_running;
        // Host monotonic timestamps are not part of guest time. Re-seed the
        // timer on the next VM tick after restore while keeping its count.
        self.timer_seeded = false;
        self.timer_fired = state.timer_fired;
    }

    /// Write the `IA32_APIC_BASE` MSR. x2APIC mode requests are downgraded to
    /// xAPIC (bit 11 set, bit 10 cleared -> xAPIC enable).
    pub fn set_apic_base_msr(&mut self, value: u64) {
        unsafe { ghostos_vm_apic_set_base_msr(self, value) }
    }

    /// Deliver an interrupt into the APIC. `vector` is the 8-bit interrupt
    /// vector; level-triggered sources set the TMR bit on acceptance.
    pub fn signal(&mut self, vector: u8, trigger: ApicTrigger) {
        unsafe { ghostos_vm_apic_signal(self, vector, trigger == ApicTrigger::Level) }
    }

    /// Highest-priority pending vector that may be delivered now.
    ///
    /// Returns `None` when the APIC is disabled, the software-enable (SVR)
    /// bit is clear, TPR blocks the request, or an equal/higher-priority
    /// interrupt is already in service.
    pub fn pending_vector(&mut self) -> Option<u8> {
        let vector = unsafe { ghostos_vm_apic_pending(self) };
        (vector >= 0).then_some(vector as u8)
    }

    /// Accept the vector returned by [`Self::pending_vector`]: move it from
    /// IRR to ISR and record the trigger mode in TMR.
    pub fn accept_pending(&mut self, vector: u8) {
        unsafe { ghostos_vm_apic_accept(self, vector) }
    }

    /// Write the EOI register: retire the highest in-service vector and let
    /// the next pending request become visible.
    pub fn eoi(&mut self) {
        unsafe { ghostos_vm_apic_eoi(self) }
    }

    /// Software reset / RESET state of the local APIC.
    pub fn reset(&mut self) {
        *self = Self::new((self.id >> 24) as u8);
    }

    /// Advance the count-down timer to host time `now_ns`. Safe to call with
    /// a monotonically increasing timestamp.
    pub fn advance(&mut self, now_ns: u64) {
        unsafe { ghostos_vm_apic_advance(self, now_ns) }
    }

    /// Highest vector currently in service (for diagnostics).
    pub fn in_service(&self) -> Option<u8> {
        let vector = unsafe { ghostos_vm_apic_in_service(self) };
        (vector >= 0).then_some(vector as u8)
    }

    /// True when the count-down timer is running.
    pub fn timer_running(&self) -> bool {
        self.timer_running
    }

    /// Read a register value by 16-byte-aligned offset.
    fn read_reg(&self, off: u32) -> u32 {
        unsafe { ghostos_vm_apic_read(self, u64::from(off)) }
    }

    /// Write a register value by 16-byte-aligned offset.
    fn write_reg(&mut self, off: u32, value: u32) {
        unsafe { ghostos_vm_apic_write(self, u64::from(off), value) }
    }

    #[cfg(test)]
    fn icr_lo_write(&mut self, value: u32) {
        self.write_reg(REG_ICR_LO, value)
    }

}

impl Default for LocalApic {
    fn default() -> Self {
        Self::new(0)
    }
}

// ---------------------------------------------------------------------------
// MMIO device plumbing
// ---------------------------------------------------------------------------

impl Device for LocalApic {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        if size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        // Registers are 16-byte aligned dwords. Decoding from the low 12 bits
        // of the access address keeps the device base-agnostic for identity
        // and low-offset-preserving page mappings, which is how the xAPIC is
        // always mapped in practice.
        let off = (addr as u32) & 0xFF0;
        Ok(self.read_reg(off) as u64)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = (addr as u32) & 0xFF0;
        self.write_reg(off, value as u32);
        Ok(())
    }

    fn reset(&mut self) {
        LocalApic::reset(self);
    }
}

impl Device for Rc<RefCell<LocalApic>> {
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
    use std::cell::RefCell;
    use std::rc::Rc;

    fn apic() -> LocalApic {
        LocalApic::new(0)
    }

    #[test]
    fn defaults_version_and_enable() {
        let a = apic();
        assert_eq!(a.read_reg(REG_ID), 0);
        assert_eq!(a.read_reg(REG_VERSION), (6 << 16) | 0x14);
        assert_eq!(a.read_reg(REG_SVR), SVR_ENABLE);
        assert_eq!(a.apic_base_msr(), APIC_BASE_DEFAULT | 0x900);
        for lvt in [REG_LVT_TIMER, REG_LVT_THERMAL, REG_LVT_PERFMON, REG_LVT_LINT0, REG_LVT_LINT1, REG_LVT_ERROR] {
            assert_eq!(a.read_reg(lvt), LVT_MASK);
        }
    }

    #[test]
    fn device_requires_dword_access() {
        let mut a = apic();
        assert_eq!(Device::read(&a, APIC_BASE_DEFAULT + 0x20, 2), Err(DeviceError::UnsupportedSize));
        assert_eq!(Device::read(&a, APIC_BASE_DEFAULT + 0x20, 1), Err(DeviceError::UnsupportedSize));
        assert_eq!(Device::write(&mut a, APIC_BASE_DEFAULT + 0x20, 1, 2), Err(DeviceError::UnsupportedSize));
    }

    #[test]
    fn priority_roundtrip_through_device() {
        let mut a = apic();
        // Signal 0x30 then 0x40; pending must pick the higher one.
        a.signal(0x30, ApicTrigger::Edge);
        a.signal(0x40, ApicTrigger::Edge);
        assert_eq!(a.pending_vector(), Some(0x40));

        a.accept_pending(0x40);
        assert_eq!(a.in_service(), Some(0x40));
        // Lower-priority IRR is blocked while 0x40 is in service.
        assert_eq!(a.pending_vector(), None);

        a.eoi();
        assert_eq!(a.in_service(), None);
        assert_eq!(a.pending_vector(), Some(0x30));

        // MMIO access to the EOI register behaves identically.
        let off = (APIC_BASE_DEFAULT + REG_EOI as u64) as u32 & 0xFF0;
        Device::write(&mut a, off as u64, 0, 4).unwrap();
        assert_eq!(a.in_service(), None);
    }

    #[test]
    fn tpr_blocks_low_priority() {
        let mut a = apic();
        a.signal(0x20, ApicTrigger::Edge);
        assert_eq!(a.pending_vector(), Some(0x20));
        a.write_reg(REG_TPR, 0x30);
        // 0x20's priority class (2) is not above TPR's (3).
        assert_eq!(a.pending_vector(), None);
        a.signal(0x40, ApicTrigger::Edge);
        assert_eq!(a.pending_vector(), Some(0x40));
    }

    #[test]
    fn svr_disable_blocks_pending() {
        let mut a = apic();
        a.signal(0x40, ApicTrigger::Edge);
        assert_eq!(a.pending_vector(), Some(0x40));
        a.write_reg(REG_SVR, 0); // clear software enable
        assert_eq!(a.pending_vector(), None);
        a.write_reg(REG_SVR, SVR_ENABLE);
        assert_eq!(a.pending_vector(), Some(0x40));
    }

    #[test]
    fn level_trigger_sets_tmr_on_accept() {
        let mut a = apic();
        a.signal(0x41, ApicTrigger::Level);
        a.accept_pending(0x41);
        assert_ne!(a.tmr[table_index(0x41)] & table_bit(0x41), 0);
        a.eoi();
        assert_eq!(a.tmr[table_index(0x41)] & table_bit(0x41), 0);
    }

    #[test]
    fn icr_self_fixed_delivers() {
        let mut a = apic();
        a.icr_lo_write(ICR_SELF | DELIVERY_FIXED | 0x51);
        assert_eq!(a.pending_vector(), Some(0x51));
    }

    #[test]
    fn icr_nmi_delivers_vector_two() {
        let mut a = apic();
        a.icr_lo_write(ICR_SELF | DELIVERY_NMI | 0x44);
        assert_eq!(a.pending_vector(), Some(2));
    }

    #[test]
    fn nmi_bypasses_tpr_masking() {
        let mut a = apic();
        a.write_reg(REG_TPR, 0xF0);
        a.icr_lo_write(ICR_SELF | DELIVERY_NMI | 0x44);
        assert_eq!(a.pending_vector(), Some(2));
    }

    #[test]
    fn icr_physical_dest_other_is_ignored() {
        let mut a = apic();
        a.icr_hi = 3 << 24; // APIC ID 3
        a.icr_lo_write(DELIVERY_FIXED | 0x42);
        assert_eq!(a.pending_vector(), None);
    }

    #[test]
    fn timer_one_shot_fires_and_pends_lvt_vector() {
        let mut a = apic();
        a.write_reg(REG_LVT_TIMER, 0x32); // fixed, unmasked, vector 0x32
        a.write_reg(REG_TIMER_DIVIDE, 0b1011); // /1
        a.write_reg(REG_TIMER_INIT_COUNT, 1000);
        assert!(a.timer_running());

        // The first advance (0ns) establishes the count-down phase; the
        // 1000 ticks at 10ns each then fire at 10,000 ns.
        a.advance(0);
        a.advance(9_900);
        assert_eq!(a.pending_vector(), None);
        a.advance(10_000);
        assert_eq!(a.pending_vector(), Some(0x32));
        assert!(!a.timer_running(), "one-shot must stop after firing");
    }

    #[test]
    fn timer_is_delayed_by_divide_config() {
        let mut a = apic();
        a.write_reg(REG_LVT_TIMER, 0x32);
        a.write_reg(REG_TIMER_DIVIDE, 0b0000); // /2
        a.write_reg(REG_TIMER_INIT_COUNT, 100);
        // Count 100 at /2 fires after 200 raw ticks -> 2,000 ns.
        a.advance(0);
        a.advance(1_900); // 190 ticks / 2 = 95 effective; not fired yet.
        assert_eq!(a.pending_vector(), None);
        a.advance(2_000); // +100ns = +10 ticks /2 -> reaches 100 effective.
        assert_eq!(a.pending_vector(), Some(0x32));
    }

    #[test]
    fn timer_masked_holds_fired_flag() {
        let mut a = apic();
        a.write_reg(REG_LVT_TIMER, 0x32); // unmasked
        a.write_reg(REG_TIMER_DIVIDE, 0b1011);
        a.write_reg(REG_TIMER_INIT_COUNT, 10);
        // 10 ticks at 10ns -> 100 ns.
        a.advance(0);
        a.advance(100);
        assert_eq!(a.pending_vector(), Some(0x32));

        // Masking clears the fired flag; a reload re-arms the phase.
        a.write_reg(REG_LVT_TIMER, 0x32 | LVT_MASK);
        a.write_reg(REG_TIMER_INIT_COUNT, 10);
        a.advance(100); // seeds the new phase
        a.advance(300); // 20 ticks while masked -> holds the fired flag
        assert_eq!(a.pending_vector(), None);
    }

    #[test]
    fn timer_periodic_reloads() {
        let mut a = apic();
        a.write_reg(REG_LVT_TIMER, 0x32 | LVT_PERIODIC);
        a.write_reg(REG_TIMER_DIVIDE, 0b1011);
        a.write_reg(REG_TIMER_INIT_COUNT, 100);
        a.advance(0);
        a.advance(1000); // exactly one period -> reload
        assert!(a.timer_running());
        assert_eq!(a.pending_vector(), Some(0x32));
        // After accept the periodic timer still has a fresh count.
        a.accept_pending(0x32);
        assert_eq!(a.read_reg(REG_TIMER_CURRENT_COUNT), 100);
    }

    #[test]
    fn base_msr_write_read_roundtrip() {
        let mut a = apic();
        a.set_apic_base_msr(0xFEE0_0000 | (1 << 11));
        assert!(a.enabled);
        assert_eq!(a.apic_base_msr() & 0xFFFF_F000, 0xFEE0_0000);

        a.set_apic_base_msr(0xFEE0_0000); // disable
        assert!(!a.enabled);
        a.signal(0x40, ApicTrigger::Edge);
        assert_eq!(a.pending_vector(), None);
    }

    #[test]
    fn shared_rc_device_impl_routes() {
        let dev: Rc<RefCell<LocalApic>> = Rc::new(RefCell::new(LocalApic::new(0)));
        Device::write(&mut dev.clone(), APIC_BASE_DEFAULT + REG_TPR as u64, 0x50, 4).unwrap();
        let a = dev.borrow();
        assert_eq!(a.read_reg(REG_TPR), 0x50);
        assert_eq!(a.read_reg(REG_PPR), 0x50);
    }
}
