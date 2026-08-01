//! Minimal ACPI power-control port used by guest shutdown and reboot paths.

use super::{DeviceError, PortDevice};
use std::cell::RefCell;
use std::rc::Rc;

pub const POWER_CONTROL_PORT: u16 = 0x604;
const SLP_EN: u16 = 1 << 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerState {
    Running,
    Shutdown,
    Reboot,
}

pub struct PowerControl {
    state: Rc<RefCell<PowerState>>,
}

impl PowerControl {
    pub fn new(state: Rc<RefCell<PowerState>>) -> Self {
        Self { state }
    }

    fn write_value(&mut self, value: u16) {
        if value & SLP_EN == 0 {
            return
        }

        let sleep_type = (value >> 10) & 0x07;
        *self.state.borrow_mut() = if sleep_type == 5 {
            PowerState::Shutdown
        } else {
            PowerState::Reboot
        };
    }
}

impl PortDevice for PowerControl {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        if port != POWER_CONTROL_PORT || !matches!(size, 2 | 4) {
            return Err(DeviceError::UnsupportedSize)
        }
        Ok(0)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if port != POWER_CONTROL_PORT || !matches!(size, 2 | 4) {
            return Err(DeviceError::UnsupportedSize)
        }
        self.write_value(value as u16);
        Ok(())
    }

    fn reset(&mut self) {
        *self.state.borrow_mut() = PowerState::Running
    }
}
