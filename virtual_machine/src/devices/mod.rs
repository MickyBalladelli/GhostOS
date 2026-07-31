pub mod interrupt_controller;

pub use interrupt_controller::InterruptController;

pub trait Device {
    fn read(&self, addr: u64) -> u64;
    fn write(&mut self, addr: u64, value: u64);
    fn reset(&mut self);
}

#[derive(Debug)]
pub enum DeviceError {
    InvalidAddress,
    AccessDenied,
    NotReady,
}

pub struct PciHostBridge {
    pub devices: Vec<(u16, u16, Box<dyn Device>)>,
}

impl PciHostBridge {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
        }
    }

    pub fn enumerate(&mut self) -> Result<(), DeviceError> {
        Ok(())
    }

    pub fn read_config(&self, bus: u8, device: u8, function: u8, offset: u8) -> u32 {
        0
    }

    pub fn write_config(&mut self, bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    }
}

impl Device for PciHostBridge {
    fn read(&self, _addr: u64) -> u64 {
        0
    }

    fn write(&mut self, _addr: u64, _value: u64) {
    }

    fn reset(&mut self) {
        self.devices.clear();
    }
}