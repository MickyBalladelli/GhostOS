//! 8254 Programmable Interval Timer (PIT) emulation.
//!
//! Implements the three counters at ports 0x40..=0x43 with the standard
//! command and read-back protocols, operating modes 0-5, count latching, and
//! host-time-driven count-down. Channel 0 is wired to the guest IRQ0 line:
//! when the machine has a local APIC attached, terminal-count pulses signal
//! the configured APIC vector (as the I/O APIC would in a real PC).

use crate::devices::{ApicTrigger, DeviceError, LocalApic, PortDevice};
use std::cell::RefCell;
use std::rc::Rc;

/// Base frequency of the 8254 oscillator (1.193182 MHz).
pub const PIT_FREQUENCY_HZ: u64 = 1_193_182;

pub const PIT_CH0_PORT: u16 = 0x40;
pub const PIT_CH1_PORT: u16 = 0x41;
pub const PIT_CH2_PORT: u16 = 0x42;
pub const PIT_CMD_PORT: u16 = 0x43;
/// Number of I/O ports occupied by the PIT (0x40..=0x43).
pub const PIT_PORT_COUNT: u16 = 4;

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct CChannel {
    reload: u16,
    count: u16,
    latched_count: u16,
    mode: u8,
    access: u8,
    latched_status: u8,
    wr_bytes: u8,
    rd_bytes: u8,
    bcd: bool,
    gate: bool,
    output: bool,
    running: bool,
    null_count: bool,
    has_latched_count: bool,
    has_latched_status: bool,
}

#[derive(Default)]
#[repr(C)]
struct CState {
    channels: [CChannel; 3],
    last_ns: u64,
    has_last_ns: bool,
}

const _: () = {
    assert!(std::mem::size_of::<CChannel>() == 18);
    assert!(std::mem::align_of::<CChannel>() == 2);
    assert!(std::mem::size_of::<CState>() == 72);
    assert!(std::mem::align_of::<CState>() == 8);
    assert!(std::mem::offset_of!(CState, last_ns) == 56);
};

unsafe extern "C" {
    fn ghostos_vm_pit_init(pit: *mut CState);
    fn ghostos_vm_pit_read(pit: *mut CState, port: u16, size: u8, value: *mut u64) -> u8;
    fn ghostos_vm_pit_write(pit: *mut CState, port: u16, value: u64, size: u8) -> u8;
    fn ghostos_vm_pit_advance(pit: *mut CState, now_ns: u64) -> bool;
}

/// C-owned 8254 counters with the VM's shared APIC adapter.
pub struct Pit {
    state: CState,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq0_vector: u8,
}

impl Pit {
    pub fn new() -> Self {
        let mut state = CState::default();
        unsafe { ghostos_vm_pit_init(&mut state) };
        Self { state, apic: None, irq0_vector: 0 }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic);
    }

    pub fn set_irq0_vector(&mut self, vector: u8) {
        self.irq0_vector = vector;
    }

    pub fn advance(&mut self, now_ns: u64) {
        let fired = unsafe { ghostos_vm_pit_advance(&mut self.state, now_ns) };
        if fired && self.irq0_vector != 0 {
            if let Some(apic) = &self.apic {
                apic.borrow_mut().signal(self.irq0_vector, ApicTrigger::Edge);
            }
        }
    }
}

impl Default for Pit {
    fn default() -> Self {
        Self::new()
    }
}

fn port_result(code: u8) -> Result<(), DeviceError> {
    match code {
        0 => Ok(()),
        1 => Err(DeviceError::UnsupportedSize),
        _ => Err(DeviceError::InvalidAddress),
    }
}

impl PortDevice for Pit {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        let mut value = 0;
        let code = unsafe { ghostos_vm_pit_read(&mut self.state, port, size, &mut value) };
        port_result(code)?;
        Ok(value)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        port_result(unsafe { ghostos_vm_pit_write(&mut self.state, port, value, size) })
    }

    fn reset(&mut self) {
        unsafe { ghostos_vm_pit_init(&mut self.state) };
    }
}

impl PortDevice for Rc<RefCell<Pit>> {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        let mut guard = self.borrow_mut();
        PortDevice::read(&mut *guard, port, size)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        let mut guard = self.borrow_mut();
        PortDevice::write(&mut *guard, port, value, size)
    }

    fn reset(&mut self) {
        self.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Approximate nanoseconds for `ticks` PIT clock ticks.
    fn ns_for_ticks(ticks: u64) -> u64 {
        ticks * 1_000_000_000 / PIT_FREQUENCY_HZ + 1
    }

    #[test]
    fn lsb_msb_reload() {
        let mut pit = Pit::new();
        // ch0, LSB then MSB, mode 3, binary.
        PortDevice::write(&mut pit, PIT_CMD_PORT, 0x36, 1).unwrap();
        PortDevice::write(&mut pit, PIT_CH0_PORT, 0x34, 1).unwrap();
        PortDevice::write(&mut pit, PIT_CH0_PORT, 0x12, 1).unwrap();
        assert_eq!(pit.state.channels[0].reload, 0x1234);
        assert_eq!(pit.state.channels[0].count, 0x1234);
        assert!(pit.state.channels[0].running);
        assert!(!pit.state.channels[0].null_count);
    }

    #[test]
    fn rate_generator_reloads_and_fires() {
        let mut pit = Pit::new();
        // ch0, LSB/MSB, mode 2 (rate generator), binary.
        PortDevice::write(&mut pit, PIT_CMD_PORT, 0x34, 1).unwrap();
        PortDevice::write(&mut pit, PIT_CH0_PORT, 0x0A, 1).unwrap();
        PortDevice::write(&mut pit, PIT_CH0_PORT, 0x00, 1).unwrap();

        // Advance with a monotonic host clock. After a full period the
        // counter reloads; afterwards count-down resumes from the reload.
        pit.advance(0);
        pit.advance(ns_for_ticks(10));
        assert_eq!(pit.state.channels[0].count, 10, "mode 2 must reload at terminal count");
        assert!(pit.state.channels[0].running);

        // Half a period later the counter has counted down from 10.
        pit.advance(ns_for_ticks(15));
        assert_eq!(pit.state.channels[0].count, 5);
        assert!(pit.state.channels[0].running);
    }

    #[test]
    fn mode0_stops_on_terminal_count() {
        let mut pit = Pit::new();
        // ch0, LSB/MSB, mode 0 (interrupt on terminal count).
        PortDevice::write(&mut pit, PIT_CMD_PORT, 0x30, 1).unwrap();
        PortDevice::write(&mut pit, PIT_CH0_PORT, 0x05, 1).unwrap();
        PortDevice::write(&mut pit, PIT_CH0_PORT, 0x00, 1).unwrap();

        pit.advance(0);
        pit.advance(ns_for_ticks(5));
        assert!(!pit.state.channels[0].running);
        assert!(pit.state.channels[0].output);
    }

    #[test]
    fn latch_command_freezes_count_readback() {
        let mut pit = Pit::new();
        PortDevice::write(&mut pit, PIT_CMD_PORT, 0x34, 1).unwrap();
        PortDevice::write(&mut pit, PIT_CH0_PORT, 0x34, 1).unwrap();
        PortDevice::write(&mut pit, PIT_CH0_PORT, 0x12, 1).unwrap();
        // Latch channel 0 via a control word with RW = 00.
        PortDevice::write(&mut pit, PIT_CMD_PORT, 0x00, 1).unwrap();
        assert_eq!(PortDevice::read(&mut pit, PIT_CH0_PORT, 1).unwrap(), 0x34);
        assert_eq!(PortDevice::read(&mut pit, PIT_CH0_PORT, 1).unwrap(), 0x12);
    }

    #[test]
    fn readback_command_returns_status() {
        let mut pit = Pit::new();
        // ch0, LSB/MSB, mode 0, BCD off.
        PortDevice::write(&mut pit, PIT_CMD_PORT, 0x30, 1).unwrap();
        // Read-back command: latch status+count for channel 0.
        PortDevice::write(&mut pit, PIT_CMD_PORT, 0xC2, 1).unwrap();
        let status = PortDevice::read(&mut pit, PIT_CH0_PORT, 1).unwrap() as u8;
        // Mode bits 3:1 = 0 (mode 0), access bits 5:4 = 3 (LSB/MSB).
        assert_eq!((status >> 1) & 0x07, 0);
        assert_eq!((status >> 4) & 0x03, 3);
        assert!(status & 0x40 != 0, "null count should be set before a load");
    }

    #[test]
    fn channel0_signals_apic_in_rate_generator() {
        let mut pit = Pit::new();
        let apic = Rc::new(RefCell::new(LocalApic::new(0)));
        pit.attach_apic(apic.clone());
        pit.set_irq0_vector(0x20);

        PortDevice::write(&mut pit, PIT_CMD_PORT, 0x34, 1).unwrap();
        PortDevice::write(&mut pit, PIT_CH0_PORT, 0x0F, 1).unwrap();
        PortDevice::write(&mut pit, PIT_CH0_PORT, 0x00, 1).unwrap();

        pit.advance(0);
        pit.advance(ns_for_ticks(15));
        assert_eq!(apic.borrow_mut().pending_vector(), Some(0x20));
        // The periodic counter keeps running after the pulse.
        assert!(pit.state.channels[0].running);
    }

    #[test]
    fn byte_only_access_is_enforced() {
        let mut pit = Pit::new();
        assert_eq!(
            PortDevice::write(&mut pit, PIT_CMD_PORT, 0x10, 2),
            Err(DeviceError::UnsupportedSize)
        );
        assert_eq!(
            PortDevice::write(&mut pit, PIT_CMD_PORT, 0x10, 4),
            Err(DeviceError::UnsupportedSize)
        );
    }

    #[test]
    fn shared_rc_routes_through_mmio_like_ports() {
        let dev: Rc<RefCell<Pit>> = Rc::new(RefCell::new(Pit::new()));
        PortDevice::write(&mut dev.clone(), PIT_CMD_PORT, 0x34, 1).unwrap();
        PortDevice::write(&mut dev.clone(), PIT_CH0_PORT, 0x10, 1).unwrap();
        PortDevice::write(&mut dev.clone(), PIT_CH0_PORT, 0x20, 1).unwrap();
        let pit = dev.borrow();
        assert_eq!(pit.state.channels[0].reload, 0x2010);
    }
}
