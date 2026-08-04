//! Minimal ACPI power-control port used by guest shutdown and reboot paths.

use super::{DeviceError, PortDevice};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub const POWER_CONTROL_PORT: u16 = 0x604;
const SLP_EN: u16 = 1 << 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerState {
    Running,
    Shutdown,
    Reboot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerNotification {
    Shutdown,
    Reboot,
}

pub struct PowerControl {
    state: Rc<RefCell<PowerState>>,
    notifications: Vec<Rc<RefCell<VecDeque<PowerNotification>>>>,
}

impl PowerControl {
    pub fn new(state: Rc<RefCell<PowerState>>) -> Self {
        Self {
            state,
            notifications: Vec::new(),
        }
    }

    pub fn attach_notifications(
        &mut self,
        notifications: Rc<RefCell<VecDeque<PowerNotification>>>,
    ) {
        self.notifications.push(notifications)
    }

    fn write_value(&mut self, value: u16) {
        if value & SLP_EN == 0 {
            return
        }

        let sleep_type = (value >> 10) & 0x07;
        let state = if sleep_type == 5 {
            PowerState::Shutdown
        } else {
            PowerState::Reboot
        };
        *self.state.borrow_mut() = state;
        let notification = match state {
                PowerState::Shutdown => PowerNotification::Shutdown,
                PowerState::Reboot => PowerNotification::Reboot,
                PowerState::Running => return,
        };
        for notifications in &self.notifications {
            notifications.borrow_mut().push_back(notification);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_and_reboot_are_notified_and_reset() {
        let state = Rc::new(RefCell::new(PowerState::Running));
        let events = Rc::new(RefCell::new(VecDeque::new()));
        let mut power = PowerControl::new(state.clone());
        power.attach_notifications(events.clone());

        power.write(POWER_CONTROL_PORT, (5 << 10) | SLP_EN as u64, 2).unwrap();
        assert_eq!(*state.borrow(), PowerState::Shutdown);
        assert_eq!(events.borrow_mut().pop_front(), Some(PowerNotification::Shutdown));

        power.write(POWER_CONTROL_PORT, (0 << 10) | SLP_EN as u64, 4).unwrap();
        assert_eq!(*state.borrow(), PowerState::Reboot);
        assert_eq!(events.borrow_mut().pop_front(), Some(PowerNotification::Reboot));

        power.write(POWER_CONTROL_PORT, 0, 2).unwrap();
        assert_eq!(*state.borrow(), PowerState::Reboot);
        power.reset();
        assert_eq!(*state.borrow(), PowerState::Running);
    }

    #[test]
    fn invalid_power_access_is_rejected() {
        let state = Rc::new(RefCell::new(PowerState::Running));
        let mut power = PowerControl::new(state);
        assert_eq!(power.read(POWER_CONTROL_PORT + 1, 2), Err(DeviceError::UnsupportedSize));
        assert_eq!(power.write(POWER_CONTROL_PORT, 0, 1), Err(DeviceError::UnsupportedSize));
    }
}
