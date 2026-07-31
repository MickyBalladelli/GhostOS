//! Device emulation: memory-mapped devices, port-mapped I/O, and the PCI host bridge.

mod interrupt_controller;
mod serial;

pub use interrupt_controller::{IdtGate, InterruptController};
pub use serial::Serial16550;

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceError {
    InvalidAddress,
    AccessDenied,
    NotReady,
    UnsupportedSize,
    NotFound,
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::InvalidAddress => write!(f, "invalid device address"),
            DeviceError::AccessDenied => write!(f, "access denied"),
            DeviceError::NotReady => write!(f, "device not ready"),
            DeviceError::UnsupportedSize => write!(f, "unsupported access size"),
            DeviceError::NotFound => write!(f, "device not found"),
        }
    }
}

impl std::error::Error for DeviceError {}

/// A memory-mapped device attached to the MMU.
pub trait Device {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError>;
    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError>;
    fn reset(&mut self);
}

/// A contiguous memory-mapped I/O region owned by the MMU.
pub struct MmioRegion {
    pub base: u64,
    pub size: u64,
    device: Box<dyn Device>,
}

impl MmioRegion {
    pub fn new(base: u64, size: u64, device: Box<dyn Device>) -> Self {
        Self { base, size, device }
    }

    pub fn contains(&self, addr: u64, size: u64) -> bool {
        addr >= self.base && addr.saturating_add(size) <= self.base.saturating_add(self.size)
    }

    pub fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        self.device.read(addr, size)
    }

    pub fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        self.device.write(addr, value, size)
    }

    pub fn reset(&mut self) {
        self.device.reset();
    }
}

/// A port-mapped I/O device attached to the CPU's port bus.
pub trait PortDevice {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError>;
    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError>;
    fn reset(&mut self);
}

/// Bus that dispatches `in`/`out` instructions to port-mapped devices.
pub struct PortBus {
    devices: Vec<(u16, u16, Box<dyn PortDevice>)>,
}

impl PortBus {
    pub fn new() -> Self {
        Self { devices: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    pub fn attach(&mut self, base: u16, size: u16, device: Box<dyn PortDevice>) {
        self.devices.push((base, size, device));
    }

    pub fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        for (base, len, dev) in &mut self.devices {
            if port >= *base && (port as u32).saturating_add(size as u32) <= (*base as u32).saturating_add(*len as u32) {
                return dev.read(port, size);
            }
        }
        // Unhandled port reads return all-ones, as real hardware does.
        Ok((1u64 << (size * 8)).wrapping_sub(1))
    }

    pub fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        for (base, len, dev) in &mut self.devices {
            if port >= *base && (port as u32).saturating_add(size as u32) <= (*base as u32).saturating_add(*len as u32) {
                return dev.write(port, value, size);
            }
        }
        // Unhandled port writes are discarded.
        Ok(())
    }

    pub fn reset(&mut self) {
        for (_, _, dev) in &mut self.devices {
            dev.reset();
        }
    }
}

impl Default for PortBus {
    fn default() -> Self {
        Self::new()
    }
}

/// PCI device identifier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PciDeviceId {
    pub vendor: u16,
    pub device: u16,
    pub revision: u8,
    pub prog_if: u8,
    pub subclass: u8,
    pub class: u8,
}

/// A function on a PCI device.
struct PciFunction {
    id: PciDeviceId,
    config: [u8; 256],
    command: u16,
    status: u16,
    bars: [u32; 6],
    irq_line: u8,
    irq_pin: u8,
}

impl PciFunction {
    fn new(id: PciDeviceId) -> Self {
        let mut f = Self {
            id,
            config: [0; 256],
            command: 0,
            status: 0,
            bars: [0; 6],
            irq_line: 0xFF,
            irq_pin: 1,
        };
        f.config[0..2].copy_from_slice(&id.vendor.to_le_bytes());
        f.config[2..4].copy_from_slice(&id.device.to_le_bytes());
        f.config[8] = id.revision;
        f.config[9] = id.prog_if;
        f.config[10] = id.subclass;
        f.config[11] = id.class;
        f.config[12] = 0x00; // cache line size
        f.config[13] = 0x00; // latency timer
        f.config[14] = 0x00; // header type
        f.config[15] = 0x00; // BIST
        f
    }

    fn reg_dword(&self, off: usize) -> u32 {
        let off = (off & 0xFC) as usize;
        u32::from_le_bytes([
            self.config[off],
            self.config[off + 1],
            self.config[off + 2],
            self.config[off + 3],
        ])
    }

    fn set_reg_dword(&mut self, off: usize, value: u32) {
        let off = (off & 0xFC) as usize;
        self.config[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn read_dword(&self, off: usize) -> u32 {
        match off & 0xFC {
            0x00 => self.id.vendor as u32 | (self.id.device as u32) << 16,
            0x04 => self.command as u32 | (self.status as u32) << 16,
            0x08 => self.reg_dword(0x08),
            0x0C => self.reg_dword(0x0C),
            0x10..=0x24 => self.bars[((off & 0xFC) - 0x10) / 4],
            0x2C => self.reg_dword(0x2C),
            0x3C => self.irq_line as u32 | (self.irq_pin as u32) << 8,
            _ => self.reg_dword(off),
        }
    }

    fn write_dword(&mut self, off: usize, value: u32) {
        match off & 0xFC {
            0x04 => {
                self.command = (value & 0xFFFF) as u16;
                self.status |= ((value >> 16) & 0xFFFF) as u16;
            }
            0x10..=0x24 => {
                self.bars[((off & 0xFC) - 0x10) / 4] = value;
                self.set_reg_dword(off, value);
            }
            0x3C => {
                self.irq_line = (value & 0xFF) as u8;
                self.irq_pin = ((value >> 8) & 0xFF) as u8;
            }
            _ => self.set_reg_dword(off, value),
        }
    }
}

struct PciSlot {
    bus: u8,
    dev: u8,
    fns: Vec<PciFunction>,
}

/// PCI Express host bridge with bus enumeration and port-based configuration
/// space access (0xCF8 / 0xCFC).
pub struct PciHostBridge {
    slots: Vec<PciSlot>,
    config_address: u32,
}

impl PciHostBridge {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            config_address: 0,
        }
    }

    pub fn add_device(&mut self, bus: u8, dev: u8, fn_: u8, id: PciDeviceId) {
        if fn_ == 0 {
            self.slots.push(PciSlot {
                bus,
                dev,
                fns: Vec::new(),
            });
        }
        if let Some(slot) = self.slots.iter_mut().find(|s| s.bus == bus && s.dev == dev) {
            while slot.fns.len() <= fn_ as usize {
                slot.fns.push(PciFunction::new(PciDeviceId::default()));
            }
            slot.fns[fn_ as usize] = PciFunction::new(id);
        }
    }

    /// Walk every bus/device/function and (re)initialize configuration spaces.
    pub fn enumerate(&mut self) -> Result<(), DeviceError> {
        for _ in &mut self.slots {
            // Config spaces are already populated by add_device; in a full
            // implementation this would probe for devices on the bus.
        }
        Ok(())
    }

    pub fn find(&self, bus: u8, device: u8, function: u8) -> Option<&PciFunction> {
        self.slots
            .iter()
            .find(|s| s.bus == bus && s.dev == device)
            .and_then(|s| s.fns.get(function as usize))
    }

    pub fn find_mut(&mut self, bus: u8, device: u8, function: u8) -> Option<&mut PciFunction> {
        self.slots
            .iter_mut()
            .find(|s| s.bus == bus && s.dev == device)
            .and_then(|s| s.fns.get_mut(function as usize))
    }

    pub fn read_config(&self, bus: u8, device: u8, function: u8, offset: u8) -> u32 {
        self.find(bus, device, function)
            .map(|f| f.read_dword(offset as usize))
            .unwrap_or(0xFFFF_FFFF)
    }

    pub fn write_config(&mut self, bus: u8, device: u8, function: u8, offset: u8, value: u32) {
        if let Some(f) = self.find_mut(bus, device, function) {
            f.write_dword(offset as usize, value);
        }
    }

    pub fn reset(&mut self) {
        self.config_address = 0;
        self.slots.clear();
    }
}

impl Default for PciHostBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl PortDevice for PciHostBridge {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        if size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        match port {
            0xCF8 => Ok(self.config_address as u64),
            0xCFC => {
                if self.config_address & 0x8000_0000 == 0 {
                    return Ok(0xFFFF_FFFF);
                }
                let bus = ((self.config_address >> 16) & 0xFF) as u8;
                let dev = ((self.config_address >> 11) & 0x1F) as u8;
                let function = ((self.config_address >> 8) & 0x07) as u8;
                let offset = (self.config_address & 0xFC) as u8;
                Ok(self.read_config(bus, dev, function, offset) as u64)
            }
            _ => Err(DeviceError::InvalidAddress),
        }
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        match port {
            0xCF8 => {
                self.config_address = value as u32;
            }
            0xCFC => {
                if self.config_address & 0x8000_0000 == 0 {
                    return Ok(());
                }
                let bus = ((self.config_address >> 16) & 0xFF) as u8;
                let dev = ((self.config_address >> 11) & 0x1F) as u8;
                let function = ((self.config_address >> 8) & 0x07) as u8;
                let offset = (self.config_address & 0xFC) as u8;
                self.write_config(bus, dev, function, offset, value as u32);
            }
             _ => {}
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.config_address = 0;
        self.slots.clear();
    }
}

impl Device for PciHostBridge {
    fn read(&self, _addr: u64, _size: u8) -> Result<u64, DeviceError> {
        Ok(0)
    }

    fn write(&mut self, _addr: u64, _value: u64, _size: u8) -> Result<(), DeviceError> {
        Ok(())
    }

    fn reset(&mut self) {
        PciHostBridge::reset(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_bus_handles_unmapped_ports() {
        let mut bus = PortBus::new();
        assert!(bus.is_empty());
        // Reads of unhandled ports return all-ones for the requested size.
        assert_eq!(bus.read(0x1234, 1).unwrap(), 0xFF);
        assert_eq!(bus.read(0x1234, 4).unwrap(), 0xFFFF_FFFF);
        assert!(bus.write(0x1234, 0xDEAD, 4).is_ok());
    }

    #[test]
    fn pci_config_access() {
        let mut pci = PciHostBridge::new();
        pci.add_device(
            0,
            3,
            0,
            PciDeviceId {
                vendor: 0x1234,
                device: 0x1111,
                class: 0x02,
                subclass: 0x00,
                prog_if: 0x00,
                revision: 0x01,
            },
        );
        assert_eq!(pci.read_config(0, 3, 0, 0), 0x1111_1234);
        // Dword at offset 8 stores: [revision, prog_if, subclass, class].
        assert_eq!(pci.read_config(0, 3, 0, 8) & 0xFF, 0x01); // revision
        assert_eq!((pci.read_config(0, 3, 0, 8) >> 8) & 0xFF, 0x00); // prog_if
        assert_eq!((pci.read_config(0, 3, 0, 8) >> 16) & 0xFF, 0x00); // subclass
        assert_eq!((pci.read_config(0, 3, 0, 8) >> 24) & 0xFF, 0x02); // class
        // Missing device returns all-ones
        assert_eq!(pci.read_config(0, 0x1E, 0, 0), 0xFFFF_FFFF);

        // Port-based access
        PortDevice::write(&mut pci, 0xCF8, 0x8000_0000 | (3 << 11) | (0 << 8) | 0x00, 4).unwrap();
        assert_eq!(PortDevice::read(&mut pci, 0xCFC, 4).unwrap(), 0x1111_1234);
    }

    #[test]
    fn mmio_region_routing() {
        struct Dummy(Vec<(u64, u8)>);

        impl Device for Dummy {
            fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
                Ok((addr & 0xFF) | ((size as u64) << 16))
            }
            fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
                self.0.push((addr, value as u8));
                Ok(())
            }
            fn reset(&mut self) {
                self.0.clear();
            }
        }

        let mut region = MmioRegion::new(0x1000, 0x100, Box::new(Dummy(Vec::new())));
        assert!(region.contains(0x1000, 4));
        assert!(!region.contains(0x1100, 4));
        assert_eq!(region.read(0x1020, 2).unwrap(), 0x20 | (2 << 16));
        region.write(0x1030, 0xAA, 1).unwrap();
    }
}