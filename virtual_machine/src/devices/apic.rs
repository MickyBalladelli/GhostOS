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

/// Nominal host tick length used to drive the timer (100 MHz bus clock).
const APIC_TIMER_TICK_NS: u64 = 10;

// ---------------------------------------------------------------------------
// Register offsets (xAPIC, 16-byte aligned dwords)
// ---------------------------------------------------------------------------

const REG_ID: u32 = 0x020;
const REG_VERSION: u32 = 0x030;
const REG_TPR: u32 = 0x080;
const REG_PPR: u32 = 0x0A0;
const REG_EOI: u32 = 0x0B0;
const REG_LDR: u32 = 0x0D0;
const REG_DFR: u32 = 0x0E0;
const REG_SVR: u32 = 0x0F0;
const REG_ISR: u32 = 0x100; // ISR0..ISR7
const REG_ISR_END: u32 = REG_ISR + 28;
const REG_TMR: u32 = 0x180; // TMR0..TMR7
const REG_TMR_END: u32 = REG_TMR + 28;
const REG_IRR: u32 = 0x200; // IRR0..IRR7
const REG_IRR_END: u32 = REG_IRR + 28;
const REG_ESR: u32 = 0x280;
const REG_ICR_HI: u32 = 0x300;
const REG_ICR_LO: u32 = 0x310;
const REG_LVT_TIMER: u32 = 0x320;
const REG_LVT_THERMAL: u32 = 0x330;
const REG_LVT_PERFMON: u32 = 0x340;
const REG_LVT_LINT0: u32 = 0x350;
const REG_LVT_LINT1: u32 = 0x360;
const REG_LVT_ERROR: u32 = 0x370;
const REG_TIMER_INIT_COUNT: u32 = 0x380;
const REG_TIMER_CURRENT_COUNT: u32 = 0x390;
const REG_TIMER_DIVIDE: u32 = 0x3E0;

// ---------------------------------------------------------------------------
// Field / flag masks
// ---------------------------------------------------------------------------

/// LVT mask bit (interrupt masked).
pub const LVT_MASK: u32 = 1 << 16;
const LVT_PERIODIC: u32 = 1 << 17;
const LVT_VECTOR: u32 = 0xFF;

const SVR_ENABLE: u32 = 1 << 8;

const ICR_SELF: u32 = 0x1 << 18;
const ICR_ALL_INCL_SELF: u32 = 0x2 << 18;
const ICR_ALL_EXCL_SELF: u32 = 0x3 << 18;
const ICR_SHORTHAND: u32 = 0x3 << 18;
const ICR_DELIVERY_MODE: u32 = 0x700;
const ICR_DELIVERY_STATUS: u32 = 1 << 12;

const DELIVERY_FIXED: u32 = 0;
const DELIVERY_NMI: u32 = 0x4 << 8;
const DELIVERY_INIT: u32 = 0x5 << 8;
const DELIVERY_EXTINT: u32 = 0x7 << 8;

/// Interrupt trigger mode used by [`LocalApic::signal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApicTrigger {
    Edge,
    Level,
}

/// Priority queue used for IRR/ISR/TMR: `table[i]` holds vectors `i*32..=i*32+31`.
type VectorTable = [u32; 8];

#[inline]
fn table_index(vector: u8) -> usize {
    (vector / 32) as usize
}

#[inline]
fn table_bit(vector: u8) -> u32 {
    1u32 << (vector % 32)
}

/// Highest set vector in a 256-bit vector table (or `None`).
fn highest_set(table: &VectorTable) -> Option<u8> {
    for (i, &word) in table.iter().enumerate().rev() {
        if word != 0 {
            let bit = 31 - word.leading_zeros();
            return Some((i as u8) * 32 + bit as u8);
        }
    }
    None
}

/// Map a raw divide-configuration register value to its divisor.
fn timer_divider(raw: u32) -> u32 {
    match raw & 0xB {
        0b0000 => 2,
        0b0001 => 4,
        0b0010 => 8,
        0b0011 => 16,
        0b1000 => 32,
        0b1001 => 64,
        0b1010 => 128,
        _ => 1, // 0b1011
    }
}

/// Local APIC (xAPIC) emulation for the BSP.
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
    /// Host timestamp of the last timer advance (ns). `None` = needs seeding.
    timer_last_ns: Option<u64>,
    /// Set when the timer has wrapped to zero and the LVT vector is pending.
    timer_fired: bool,
}

impl LocalApic {
    /// Create a local APIC with APIC ID `id` (0 for the BSP).
    pub fn new(id: u8) -> Self {
        Self {
            base: APIC_BASE_DEFAULT,
            enabled: true,
            id: (id as u32) << 24,
            // Version 0x14 with six LVT entries (timer, thermal, perfmon,
            // LINT0, LINT1, error).
            version: (6 << 16) | 0x14,
            tpr: 0,
            ppr: 0,
            ldr: 0,
            dfr: 0,
            // Software enable (bit 8) is set after reset, as on real hardware.
            svr: SVR_ENABLE,
            irr: [0; 8],
            isr: [0; 8],
            tmr: [0; 8],
            level_pending: [0; 8],
            esr: 0,
            icr_hi: 0,
            icr_lo: 0,
            lvt_timer: LVT_MASK,
            lvt_thermal: LVT_MASK,
            lvt_perfmon: LVT_MASK,
            lvt_lint0: LVT_MASK,
            lvt_lint1: LVT_MASK,
            lvt_error: LVT_MASK,
            timer_initial_count: 0,
            timer_current_count: 0,
            timer_divide: 0,
            timer_running: false,
            timer_last_ns: None,
            timer_fired: false,
        }
    }

    /// Current value of the `IA32_APIC_BASE` MSR.
    pub fn apic_base_msr(&self) -> u64 {
        let mut v = self.base & 0xFFFF_F000;
        v |= 1 << 8; // BSP
        if self.enabled {
            v |= 1 << 11; // APIC enable (xAPIC)
        }
        v
    }

    /// Write the `IA32_APIC_BASE` MSR. x2APIC mode requests are downgraded to
    /// xAPIC (bit 11 set, bit 10 cleared -> xAPIC enable).
    pub fn set_apic_base_msr(&mut self, value: u64) {
        self.base = value & 0xFFFF_F000;
        self.enabled = (value >> 11) & 0x1 != 0;
    }

    /// Deliver an interrupt into the APIC. `vector` is the 8-bit interrupt
    /// vector; level-triggered sources set the TMR bit on acceptance.
    pub fn signal(&mut self, vector: u8, trigger: ApicTrigger) {
        let idx = table_index(vector);
        let bit = table_bit(vector);
        self.irr[idx] |= bit;
        match trigger {
            ApicTrigger::Level => self.level_pending[idx] |= bit,
            ApicTrigger::Edge => self.level_pending[idx] &= !bit,
        }
    }

    /// Highest-priority pending vector that may be delivered now.
    ///
    /// Returns `None` when the APIC is disabled, the software-enable (SVR)
    /// bit is clear, TPR blocks the request, or an equal/higher-priority
    /// interrupt is already in service.
    pub fn pending_vector(&mut self) -> Option<u8> {
        if !self.enabled || self.svr & SVR_ENABLE == 0 {
            return None;
        }
        self.fold_timer();
        let vector = highest_set(&self.irr)?;

        // NMI (vector 2) is non-maskable: TPR only filters fixed/lowest
        // priority maskable requests.
        if vector != 2 {
            // TPR masks interrupts at or below its priority class.
            if u32::from(vector >> 4) <= (self.tpr >> 4) {
                return None;
            }
        }
        // An in-service interrupt with equal or higher priority wins.
        if let Some(in_service) = highest_set(&self.isr) {
            if (in_service >> 4) >= (vector >> 4) {
                return None;
            }
        }
        Some(vector)
    }

    /// Accept the vector returned by [`Self::pending_vector`]: move it from
    /// IRR to ISR and record the trigger mode in TMR.
    pub fn accept_pending(&mut self, vector: u8) {
        let idx = table_index(vector);
        let bit = table_bit(vector);
        self.irr[idx] &= !bit;
        self.isr[idx] |= bit;
        if self.level_pending[idx] & bit != 0 {
            self.tmr[idx] |= bit;
        } else {
            self.tmr[idx] &= !bit;
        }
        self.level_pending[idx] &= !bit;

        // One-shot timer interrupts are consumed on delivery.
        if self.lvt_timer & LVT_PERIODIC == 0
            && (self.lvt_timer & LVT_VECTOR) == vector as u32
        {
            self.timer_fired = false;
        }
        self.update_ppr();
    }

    /// Write the EOI register: retire the highest in-service vector and let
    /// the next pending request become visible.
    pub fn eoi(&mut self) {
        if let Some(vector) = highest_set(&self.isr) {
            let idx = table_index(vector);
            let bit = table_bit(vector);
            self.isr[idx] &= !bit;
            self.tmr[idx] &= !bit;
        }
        self.update_ppr();
    }

    /// Software reset / RESET state of the local APIC.
    pub fn reset(&mut self) {
        *self = Self::new((self.id >> 24) as u8);
    }

    /// Advance the count-down timer to host time `now_ns`. Safe to call with
    /// a monotonically increasing timestamp.
    pub fn advance(&mut self, now_ns: u64) {
        if !self.timer_running || self.timer_initial_count == 0 {
            return;
        }
        let Some(last) = self.timer_last_ns else {
            self.timer_last_ns = Some(now_ns);
            return;
        };
        if now_ns <= last {
            return;
        }

        let elapsed = now_ns - last;
        let ticks = elapsed / APIC_TIMER_TICK_NS;
        let effective = ticks / timer_divider(self.timer_divide) as u64;
        if effective == 0 {
            self.timer_last_ns = Some(now_ns);
            return;
        }

        let mut remaining = self.timer_current_count as u64;
        if remaining == 0 {
            remaining = self.timer_initial_count as u64;
        }

        if effective >= remaining {
            self.timer_fired = true;
            if self.lvt_timer & LVT_PERIODIC != 0 {
                // Periodic mode: reload the initial count and keep running.
                let leave = effective % remaining;
                self.timer_current_count =
                    (self.timer_initial_count as u64).saturating_sub(leave) as u32;
            } else {
                self.timer_current_count = 0;
                self.timer_running = false;
            }
        } else {
            self.timer_current_count = (remaining - effective) as u32;
        }
        self.timer_last_ns = Some(now_ns);
    }

    /// Highest vector currently in service (for diagnostics).
    pub fn in_service(&self) -> Option<u8> {
        highest_set(&self.isr)
    }

    /// True when the count-down timer is running.
    pub fn timer_running(&self) -> bool {
        self.timer_running
    }

    /// Read a register value by 16-byte-aligned offset.
    fn read_reg(&self, off: u32) -> u32 {
        match off {
            REG_ID => self.id,
            REG_VERSION => self.version,
            REG_TPR => self.tpr,
            REG_PPR => self.ppr,
            REG_LDR => self.ldr,
            REG_DFR => self.dfr,
            REG_SVR => self.svr,
            // ISR0..7
            REG_ISR..=REG_ISR_END => self.isr[((off - REG_ISR) / 4) as usize],
            // TMR0..7
            REG_TMR..=REG_TMR_END => self.tmr[((off - REG_TMR) / 4) as usize],
            // IRR0..7
            REG_IRR..=REG_IRR_END => self.irr[((off - REG_IRR) / 4) as usize],
            REG_ESR => self.esr,
            // ICR is read back with the delivery-status bit cleared (our
            // sends complete synchronously).
            REG_ICR_HI => self.icr_hi,
            REG_ICR_LO => self.icr_lo & !ICR_DELIVERY_STATUS,
            REG_LVT_TIMER => self.lvt_timer,
            REG_LVT_THERMAL => self.lvt_thermal,
            REG_LVT_PERFMON => self.lvt_perfmon,
            REG_LVT_LINT0 => self.lvt_lint0,
            REG_LVT_LINT1 => self.lvt_lint1,
            REG_LVT_ERROR => self.lvt_error,
            REG_TIMER_INIT_COUNT => self.timer_initial_count,
            REG_TIMER_CURRENT_COUNT => self.timer_current_count,
            REG_TIMER_DIVIDE => self.timer_divide,
            _ => 0,
        }
    }

    /// Write a register value by 16-byte-aligned offset.
    fn write_reg(&mut self, off: u32, value: u32) {
        match off {
            REG_ID => self.id = value & 0x0F00_0000,
            REG_TPR => {
                self.tpr = value & 0xFF;
                self.update_ppr();
            }
            REG_EOI => self.eoi(),
            REG_LDR => self.ldr = value & 0xFF00_0000,
            REG_DFR => self.dfr = value & 0xF000_0000,
            REG_SVR => self.svr = value,
            REG_ESR => {
                // A write triggers an error-reporting send; we have no other
                // CPUs, so the error bits simply clear.
                self.esr = 0;
            }
            REG_ICR_HI => self.icr_hi = value,
            REG_ICR_LO => self.icr_lo_write(value),
            REG_LVT_TIMER => {
                self.lvt_timer = value & 0x0003_1FFF;
                // Masking the timer drops a pending timer vector from IRR.
                if self.lvt_timer & LVT_MASK != 0 {
                    self.timer_fired = false;
                    let vector = (self.lvt_timer & LVT_VECTOR) as u8;
                    let idx = table_index(vector);
                    let bit = table_bit(vector);
                    self.irr[idx] &= !bit;
                    self.level_pending[idx] &= !bit;
                    self.tmr[idx] &= !bit;
                }
            }
            REG_LVT_THERMAL => self.lvt_thermal = value & 0x0003_1FFF,
            REG_LVT_PERFMON => self.lvt_perfmon = value & 0x0003_1FFF,
            REG_LVT_LINT0 => self.lvt_lint0 = value & 0x0003_1FFF,
            REG_LVT_LINT1 => self.lvt_lint1 = value & 0x0003_1FFF,
            REG_LVT_ERROR => self.lvt_error = value & 0x0003_1FFF,
            REG_TIMER_INIT_COUNT => {
                self.timer_initial_count = value;
                self.timer_current_count = value;
                self.timer_running = value != 0;
                self.timer_fired = false;
                // Re-seed the phase so count-down starts from the next
                // advance() call.
                self.timer_last_ns = None;
            }
            REG_TIMER_DIVIDE => self.timer_divide = value & 0xB,
            _ => {}
        }
    }

    /// Fold a fired timer into the IRR priority queue when the LVT allows it.
    fn fold_timer(&mut self) {
        if self.timer_fired && self.lvt_timer & LVT_MASK == 0 {
            let vector = (self.lvt_timer & LVT_VECTOR) as u8;
            self.signal(vector, ApicTrigger::Edge);
            self.timer_fired = false;
        }
    }

    fn update_ppr(&mut self) {
        let isr_prio = u32::from(highest_set(&self.isr).map(|v| v >> 4).unwrap_or(0));
        let tpr_prio = self.tpr >> 4;
        self.ppr = isr_prio.max(tpr_prio) << 4;
    }

    fn icr_lo_write(&mut self, value: u32) {
        self.icr_lo = value;

        let shorthand = value & ICR_SHORTHAND;
        let mode = value & ICR_DELIVERY_MODE;
        let vector = (value & LVT_VECTOR) as u8;

        // In this single-CPU machine, only sends that reach the BSP matter.
        let targets_self = match shorthand {
            ICR_SELF | ICR_ALL_INCL_SELF => true,
            ICR_ALL_EXCL_SELF => false,
            _ => {
                // No shorthand: physical or logical destination.
                let dest = (self.icr_hi >> 24) as u8;
                if dest == 0xFF {
                    true // broadcast
                } else if dest == (self.id >> 24) as u8 {
                    true
                } else {
                    self.logical_dest_matches(dest)
                }
            }
        };
        if !targets_self {
            return;
        }

        match mode {
            DELIVERY_FIXED => self.signal(vector, ApicTrigger::Edge),
            DELIVERY_NMI => self.signal(2, ApicTrigger::Edge),
            DELIVERY_INIT => {
                // Single-vCPU: INIT is accepted and effectively a no-op aside
                // from clearing the in-service state.
                self.isr = [0; 8];
                let _ = &mut self.tmr;
            }
            DELIVERY_EXTINT => {
                // No external PIC is wired to the APIC; report an error send.
                self.esr |= 0x2;
            }
            _ => {
                // SMI / reserved modes are not modelled.
                self.esr |= 0x2;
            }
        }
    }

    /// Logical destination match for flat/cluster models (simplified: a
    /// logical ID matches when the DFR says flat and the destination logical
    /// ID bits overlap the LDR mask).
    fn logical_dest_matches(&self, dest: u8) -> bool {
        if self.dfr >> 28 == 0xF {
            // Flat model: each bit maps to one APIC ID.
            let ldr = (self.ldr >> 24) as u8;
            ldr & dest != 0
        } else {
            // Cluster model: top 4 bits are cluster, bottom 4 are the
            // member mask. The BSP responds to its own cluster when the
            // member mask overlaps.
            let cluster = (self.id >> 28) as u8;
            let member = ((self.ldr >> 24) & 0xF) as u8;
            cluster == (dest >> 4) && member & (dest & 0xF) != 0
        }
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