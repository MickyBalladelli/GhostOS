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

const ACCESS_MASK: u8 = 0x30;
const MODE_MASK: u8 = 0x0E;
const BCD_MASK: u8 = 0x01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum AccessMode {
    Lsb = 1,
    Msb = 2,
    LsbMsb = 3,
}

#[derive(Debug, Clone, Copy)]
struct Channel {
    mode: u8,
    access: AccessMode,
    bcd: bool,
    gate: bool,
    output: bool,
    running: bool,
    null_count: bool,
    reload: u16,
    count: u16,
    latched_count: Option<u16>,
    latched_status: Option<u8>,
    wr_bytes: u8,
    rd_bytes: u8,
}

impl Channel {
    fn new() -> Self {
        Self {
            // Power-on default: mode 3 (square wave), LSB-then-MSB access.
            mode: 3,
            access: AccessMode::LsbMsb,
            bcd: false,
            gate: true,
            output: false,
            running: false,
            null_count: true,
            reload: 0xFFFF,
            count: 0xFFFF,
            latched_count: None,
            latched_status: None,
            wr_bytes: 0,
            rd_bytes: 0,
        }
    }

    /// Latch the running count without disturbing the status register.
    /// The 8254's LATCH command (RW = 00) latches count only.
    fn latch_count(&mut self) {
        if self.latched_count.is_none() {
            self.latched_count = Some(self.count);
        }
    }

    fn status_byte(&self) -> u8 {
        let mut s = 0u8;
        if self.output {
            s |= 0x80;
        }
        if self.null_count {
            s |= 0x40;
        }
        s |= (self.access as u8) << 4;
        s |= (self.mode & 0x07) << 1;
        if self.bcd {
            s |= 0x01;
        }
        s
    }

    /// Apply a control word (mode / access / BCD) for the channel.
    fn set_control(&mut self, control: u8) {
        self.mode = (control & MODE_MASK) >> 1;
        if self.mode > 5 {
            // Modes 6 and 7 alias to modes 2 and 3.
            self.mode &= 0x03;
        }
        self.bcd = control & BCD_MASK != 0;
        self.access = match (control & ACCESS_MASK) >> 4 {
            1 => AccessMode::Lsb,
            2 => AccessMode::Msb,
            _ => AccessMode::LsbMsb,
        };
        self.null_count = true;
        self.wr_bytes = 0;
        self.rd_bytes = 0;
        self.latched_count = None;
        self.latched_status = None;
    }

    /// Program one byte of the reload value using the channel's access mode.
    fn write_reload(&mut self, byte: u8) {
        match self.access {
            AccessMode::Lsb => {
                self.reload = (self.reload & 0xFF00) | byte as u16;
                self.load_count();
            }
            AccessMode::Msb => {
                self.reload = (self.reload & 0x00FF) | (byte as u16) << 8;
                self.load_count();
            }
            AccessMode::LsbMsb => {
                if self.wr_bytes == 0 {
                    self.reload = (self.reload & 0xFF00) | byte as u16;
                    self.wr_bytes = 1;
                } else {
                    self.reload = (self.reload & 0x00FF) | (byte as u16) << 8;
                    self.wr_bytes = 0;
                    self.load_count();
                }
            }
        }
    }

    fn load_count(&mut self) {
        self.count = self.reload;
        self.running = self.reload != 0;
        self.output = false;
        self.null_count = false;
        self.rd_bytes = 0;
    }

    /// Read one byte from the data port. A latched status register (from a
    /// read-back command) is returned first; otherwise a latched or live
    /// count is streamed according to the access mode.
    fn read_byte(&mut self) -> u8 {
        if let Some(status) = self.latched_status.take() {
            return status;
        }
        if let Some(latched) = self.latched_count.take() {
            match self.access {
                AccessMode::Lsb => return (latched & 0xFF) as u8,
                AccessMode::Msb => return (latched >> 8) as u8,
                AccessMode::LsbMsb => {
                    if self.rd_bytes == 0 {
                        self.rd_bytes = 1;
                        return (latched & 0xFF) as u8;
                    }
                    self.rd_bytes = 0;
                    return (latched >> 8) as u8;
                }
            }
        }
        match self.access {
            AccessMode::Lsb => (self.count & 0xFF) as u8,
            AccessMode::Msb => (self.count >> 8) as u8,
            AccessMode::LsbMsb => {
                if self.rd_bytes == 0 {
                    self.rd_bytes = 1;
                    (self.count & 0xFF) as u8
                } else {
                    self.rd_bytes = 0;
                    (self.count >> 8) as u8
                }
            }
        }
    }
}

/// Advance a channel by `ticks` PIT clock ticks. Returns true when the
/// channel produced a terminal-count pulse (IRQ line assertion).
fn tick_channel(ch: &mut Channel, ticks: u64) -> bool {
    if !ch.gate || !ch.running || ticks == 0 {
        return false;
    }
    let mut remaining = ticks;
    let mut fired = false;
    while remaining > 0 {
        let count = ch.count as u64;
        if count == 0 {
            ch.output = true;
            ch.running = false;
            fired = true;
            break;
        }
        if count > remaining {
            ch.count = (count - remaining) as u16;
            remaining = 0;
        } else {
            remaining -= count;
            fired = true;
            ch.output = true;
            match ch.mode {
                // One-shot / interrupt-on-terminal-count modes stop.
                0 | 4 | 5 => {
                    ch.running = false;
                    break;
                }
                // Rate generator and square wave reload automatically.
                2 | 3 => {
                    ch.count = ch.reload;
                }
                _ => {
                    ch.running = false;
                    break;
                }
            }
        }
    }
    fired
}

/// 8254 Programmable Interval Timer.
pub struct Pit {
    channels: [Channel; 3],
    apic: Option<Rc<RefCell<LocalApic>>>,
    /// APIC vector signalled when channel 0 (IRQ0) produces a terminal-count
    /// pulse. 0 disables APIC delivery.
    irq0_vector: u8,
    last_ns: Option<u64>,
}

impl Pit {
    pub fn new() -> Self {
        Self {
            channels: [Channel::new(); 3],
            apic: None,
            irq0_vector: 0,
            last_ns: None,
        }
    }

    /// Attach the shared local APIC that receives channel-0 IRQ pulses.
    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic);
    }

    /// Set the APIC vector used for channel 0 (ISA IRQ0).
    pub fn set_irq0_vector(&mut self, vector: u8) {
        self.irq0_vector = vector;
    }

    fn write_control(&mut self, value: u8) {
        let channel = (value >> 6) & 0x03;
        let access = (value >> 4) & 0x03;

        // Read-back command (11 x x COUNT STATUS counter bits).
        if channel == 3 {
            let selected = (value >> 1) & 0x07;
            let latch_count = value & 0x20 == 0;
            let latch_status = value & 0x10 == 0;
            for (i, ch) in self.channels.iter_mut().enumerate() {
                if selected & (1 << i) != 0 {
                    if latch_count {
                        ch.latch_count();
                    }
                    if latch_status {
                        ch.latched_status = Some(ch.status_byte());
                    }
                }
            }
            return;
        }

        // Latch command: RW = 0 latches the current count without changing
        // the operating mode / access mode.
        if access == 0 {
            self.channels[channel as usize].latch_count();
            return;
        }

        self.channels[channel as usize].set_control(value);
    }

    /// Advance all counters to host time `now_ns`. Safe to call with a
    /// monotonically increasing timestamp.
    pub fn advance(&mut self, now_ns: u64) {
        let Some(last) = self.last_ns else {
            self.last_ns = Some(now_ns);
            return;
        };
        if now_ns <= last {
            return;
        }
        let elapsed = now_ns - last;
        self.last_ns = Some(now_ns);

        let ticks = (elapsed as u128 * PIT_FREQUENCY_HZ as u128 / 1_000_000_000) as u64;
        if ticks == 0 {
            return;
        }

        let mut ch0_fired = false;
        for (i, ch) in self.channels.iter_mut().enumerate() {
            if tick_channel(ch, ticks) && i == 0 {
                ch0_fired = true;
            }
        }

        // Channel 0 drives ISA IRQ0, delivered to the APIC as an edge-triggered
        // fixed interrupt with the configured vector.
        if ch0_fired && self.irq0_vector != 0 {
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

impl PortDevice for Pit {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        if size != 1 {
            return Err(DeviceError::UnsupportedSize);
        }
        match port {
            PIT_CH0_PORT..=PIT_CH2_PORT => {
                let idx = (port - PIT_CH0_PORT) as usize;
                Ok(self.channels[idx].read_byte() as u64)
            }
            _ => Err(DeviceError::InvalidAddress),
        }
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 1 {
            return Err(DeviceError::UnsupportedSize);
        }
        match port {
            PIT_CMD_PORT => self.write_control(value as u8),
            PIT_CH0_PORT..=PIT_CH2_PORT => {
                let idx = (port - PIT_CH0_PORT) as usize;
                self.channels[idx].write_reload(value as u8);
            }
            _ => return Err(DeviceError::InvalidAddress),
        }
        Ok(())
    }

    fn reset(&mut self) {
        let apic = self.apic.take();
        let vector = self.irq0_vector;
        *self = Self::new();
        self.apic = apic;
        self.irq0_vector = vector;
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
        assert_eq!(pit.channels[0].reload, 0x1234);
        assert_eq!(pit.channels[0].count, 0x1234);
        assert!(pit.channels[0].running);
        assert!(!pit.channels[0].null_count);
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
        assert_eq!(pit.channels[0].count, 10, "mode 2 must reload at terminal count");
        assert!(pit.channels[0].running);

        // Half a period later the counter has counted down from 10.
        pit.advance(ns_for_ticks(15));
        assert_eq!(pit.channels[0].count, 5);
        assert!(pit.channels[0].running);
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
        assert!(!pit.channels[0].running);
        assert!(pit.channels[0].output);
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
        assert!(pit.channels[0].running);
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
        assert_eq!(pit.channels[0].reload, 0x2010);
    }
}
