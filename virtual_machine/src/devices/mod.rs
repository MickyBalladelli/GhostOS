//! Device emulation: memory-mapped devices, port-mapped I/O, and a PCI
//! Express host bridge with ECAM (MMCONFIG) configuration space, bus
//! enumeration, BAR sizing probes, and PCIe capability blocks.

mod apic;
mod interrupt_controller;
mod serial;

pub use apic::{
    ApicTrigger, LocalApic, IA32_APIC_BASE_MSR, LVT_MASK, APIC_BASE_DEFAULT, APIC_SIZE,
};
pub use interrupt_controller::{IdtGate, InterruptController};
pub use serial::Serial16550;

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

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
            if port >= *base
                && (port as u32).saturating_add(size as u32)
                    <= (*base as u32).saturating_add(*len as u32)
            {
                return dev.read(port, size);
            }
        }
        // Unhandled port reads return all-ones, as real hardware does.
        Ok((1u64 << (size * 8)).wrapping_sub(1))
    }

    pub fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        for (base, len, dev) in &mut self.devices {
            if port >= *base
                && (port as u32).saturating_add(size as u32)
                    <= (*base as u32).saturating_add(*len as u32)
            {
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

// ---------------------------------------------------------------------------
// PCI / PCIe configuration-space register offsets
// ---------------------------------------------------------------------------

const PCI_VENDOR_ID: usize = 0x00;
const PCI_COMMAND: usize = 0x04;
const PCI_STATUS: usize = 0x06;
const PCI_REVISION_ID: usize = 0x08;
const PCI_PROG_IF: usize = 0x09;
const PCI_HEADER_TYPE: usize = 0x0E;
const PCI_CAPABILITIES_POINTER: usize = 0x34;
const PCI_INTERRUPT_LINE: usize = 0x3C;
const PCI_INTERRUPT_PIN: usize = 0x3D;
const PCI_BASE_ADDRESS_0: usize = 0x10;

// PCI-PCI bridge header registers
const PCI_PRIMARY_BUS: usize = 0x18;
const PCI_SECONDARY_BUS: usize = 0x19;
const PCI_SUBORDINATE_BUS: usize = 0x1A;

// PCIe capability block (capability id 0x10)
const PCIE_CAP_ID: u8 = 0x10;
const PCIE_CAP_OFFSET: usize = 0x40;
const PCIE_TYPE_ENDPOINT: u8 = 0x0; // Dev/Port type 0 = PCIe endpoint

const HEADER_TYPE_MASK: u8 = 0x7F;
const HEADER_TYPE_MULTI_FUNCTION: u8 = 0x80;
const HEADER_TYPE_NORMAL: u8 = 0x00;
const HEADER_TYPE_BRIDGE: u8 = 0x01;

const PCI_DEVICE_MISSING: u32 = 0xFFFF_FFFF;

/// Default ECAM (MMCONFIG) base address exposed by the host bridge.
pub const PCIE_ECAM_BASE_DEFAULT: u64 = 0xE000_0000;

// ---------------------------------------------------------------------------
// PCI device model
// ---------------------------------------------------------------------------

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

/// A single PCI function (device, or the downstream side of a PCI-PCI bridge).
pub(crate) struct PciFunction {
    id: PciDeviceId,
    config: [u8; 256],
    command: u16,
    status: u16,
    bars: [u32; 6],
    bar_sizes: [u32; 6],
    bar_probe: [bool; 6],
    irq_line: u8,
    irq_pin: u8,
    is_bridge: bool,
}

impl PciFunction {
    fn new(id: PciDeviceId, is_bridge: bool) -> Self {
        let header_type = if is_bridge {
            HEADER_TYPE_BRIDGE
        } else {
            HEADER_TYPE_NORMAL
        };
        let mut f = Self {
            id,
            config: [0; 256],
            command: 0,
            status: 0,
            bars: [0; 6],
            bar_sizes: [0; 6],
            bar_probe: [false; 6],
            irq_line: 0xFF,
            irq_pin: 1,
            is_bridge,
        };
        f.config[PCI_VENDOR_ID..PCI_VENDOR_ID + 2].copy_from_slice(&id.vendor.to_le_bytes());
        f.config[2..4].copy_from_slice(&id.device.to_le_bytes());
        f.config[PCI_REVISION_ID] = id.revision;
        f.config[PCI_PROG_IF] = id.prog_if;
        f.config[0x0A] = id.subclass;
        f.config[0x0B] = id.class;
        f.config[0x0C] = 0x00; // cache line size
        f.config[0x0D] = 0x00; // latency timer
        f.config[PCI_HEADER_TYPE] = header_type;
        f.config[0x0F] = 0x00; // BIST

        if is_bridge {
            // Secondary status: 66 MHz capable + fast back-to-back capable.
            f.config[0x1E] = 0x40 | 0x02;
        } else {
            // PCIe endpoint: capabilities list pointer + PCIe capability block.
            f.config[PCI_CAPABILITIES_POINTER] = PCIE_CAP_OFFSET as u8;
            f.install_pcie_capability();
        }
        f
    }

    fn header_type(&self) -> u8 {
        self.config[PCI_HEADER_TYPE]
    }

    fn is_present(&self) -> bool {
        self.id.vendor != 0 && self.id.vendor != 0xFFFF
    }

    fn install_pcie_capability(&mut self) {
        let o = PCIE_CAP_OFFSET;
        self.config[o] = PCIE_CAP_ID;
        self.config[o + 1] = 0x00; // next capability pointer
        self.config[o + 2] = 0x00 | (PCIE_TYPE_ENDPOINT << 4) | 0x02; // v2 endpoint
        self.config[o + 3] = 0x00; // link capabilities high dword
        self.config[o + 4] = 0x00; // device capabilities
        self.config[o + 5] = 0x00;
        self.config[o + 6] = 0x00;
        self.config[o + 7] = 0x00;
        self.config[o + 8] = 0x00; // device control
        self.config[o + 9] = 0x10; // device status: D-state
        self.config[o + 0x0A] = 0x00;
        self.config[o + 0x0B] = 0x00;
        self.config[o + 0x0C] = 0x00; // link capabilities
        self.config[o + 0x0D] = 0x00;
        self.config[o + 0x0E] = 0x00;
        self.config[o + 0x0F] = 0x00;
        self.config[o + 0x10] = 0x00; // link control / status
        self.config[o + 0x11] = 0x00;
        self.config[o + 0x12] = 0x00;
        self.config[o + 0x13] = 0x00;
    }

    fn bar_offset(idx: usize) -> usize {
        PCI_BASE_ADDRESS_0 + idx * 4
    }

    fn bar_writable_mask(size: u32) -> u32 {
        if size == 0 {
            0
        } else {
            !(size.wrapping_sub(1))
        }
    }

    fn bar_register(&self, idx: usize) -> u32 {
        if self.bar_probe[idx] {
            let size = self.bar_sizes[idx];
            // A size probe returns the natural alignment mask with the
            // low 4 bits (address space indicator) cleared.
            Self::bar_writable_mask(size)
        } else {
            self.bars[idx]
        }
    }

    fn bar_write(&mut self, idx: usize, value: u32) {
        let size = self.bar_sizes[idx];
        if size == 0 {
            // Not a real BAR; only keep the upper address bits.
            self.bars[idx] = value & 0xFFFF_FFF0;
            self.bar_probe[idx] = false;
        } else {
            let size_mask = Self::bar_writable_mask(size);
            // A write of "all ones" in the address bits is a size probe.
            if value & size_mask == size_mask {
                self.bar_probe[idx] = true;
                self.bars[idx] = value;
            } else {
                self.bar_probe[idx] = false;
                let low = value & 0xF;
                self.bars[idx] = (value & size_mask) | low;
            }
        }
        self.set_reg_dword(Self::bar_offset(idx), self.bars[idx]);
    }

    fn read_dword(&self, off: usize) -> u32 {
        let off = off & 0xFC;
        match off {
            0x00 => self.id.vendor as u32 | (self.id.device as u32) << 16,
            0x04 => self.command as u32 | (self.status as u32) << 16,
            0x08 => self.reg_dword(0x08),
            0x0C => u32::from_le_bytes([
                self.id.revision,
                self.id.prog_if,
                self.id.subclass,
                self.id.class,
            ]),
            0x10..=0x24 if !self.is_bridge => {
                let bar = (off - 0x10) / 4;
                self.bar_register(bar)
            }
            0x10..=0x14 if self.is_bridge => {
                let bar = (off - 0x10) / 4;
                self.bar_register(bar)
            }
            0x3C => self.irq_line as u32 | (self.irq_pin as u32) << 8,
            _ => self.reg_dword(off),
        }
    }

    fn write_dword(&mut self, off: usize, value: u32) {
        let off = off & 0xFC;
        match off {
            0x04 => {
                self.command = (value & 0xFFFF) as u16;
                // Status bits are W1C (write-1-to-clear).
                let w1c = ((value >> 16) & 0xFFFF) as u16;
                self.status &= !(0xF800 & w1c);
            }
            0x10..=0x24 if !self.is_bridge => {
                let bar = (off - 0x10) / 4;
                self.bar_write(bar, value);
            }
            0x10..=0x14 if self.is_bridge => {
                let bar = (off - 0x10) / 4;
                self.bar_write(bar, value);
            }
            0x3C => {
                self.irq_line = (value & 0xFF) as u8;
                self.irq_pin = ((value >> 8) & 0xFF) as u8;
            }
            _ => self.set_reg_dword(off, value),
        }
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

    /// Bus number behind this function when it is a PCI-PCI bridge.
    fn secondary_bus(&self) -> Option<u8> {
        if self.is_bridge {
            Some(self.config[PCI_SECONDARY_BUS])
        } else {
            None
        }
    }
}

struct PciSlot {
    bus: u8,
    dev: u8,
    fns: Vec<PciFunction>,
}

/// PCI Express host bridge.
///
/// Provides three ways to reach configuration space:
///   * Port-mapped I/O via the legacy 0xCF8 / 0xCFC address/data registers;
///   * Memory-mapped ECAM (MMCONFIG) configuration space at [`PCIE_ECAM_BASE_DEFAULT`],
///     where each bus/device/function/register is indexed as
///     `base + (bus << 20) | (dev << 15) | (fn << 12) | reg`;
///   * Direct accessors used by the machine init code.
///
/// Also implements bus enumeration (including PCI-PCI bridge traversal) and
/// BAR size probing for software that performs resource allocation.
pub struct PciHostBridge {
    slots: Vec<PciSlot>,
    config_address: u32,
    ecam_base: u64,
    bus_limit: u8,
    discovered_buses: Vec<u8>,
}

impl PciHostBridge {
    pub fn new() -> Self {
        Self::with_ecam(PCIE_ECAM_BASE_DEFAULT)
    }

    pub fn with_ecam(ecam_base: u64) -> Self {
        Self {
            slots: Vec::new(),
            config_address: 0,
            ecam_base,
            bus_limit: 255,
            discovered_buses: Vec::new(),
        }
    }

    pub fn ecam_base(&self) -> u64 {
        self.ecam_base
    }

    /// Buses discovered by the last [`Self::enumerate`] run.
    pub fn discovered_buses(&self) -> &[u8] {
        &self.discovered_buses
    }

    pub fn set_bus_limit(&mut self, limit: u8) {
        self.bus_limit = limit;
    }

    /// Register a normal PCI/PCIe endpoint function.
    pub fn add_device(&mut self, bus: u8, dev: u8, fn_: u8, id: PciDeviceId) {
        self.insert_function(bus, dev, fn_, id, false);
    }

    /// Register a PCI-PCI bridge function.
    pub fn add_bridge(&mut self, bus: u8, dev: u8, fn_: u8, id: PciDeviceId) {
        self.insert_function(bus, dev, fn_, id, true);
    }

    fn insert_function(&mut self, bus: u8, dev: u8, fn_: u8, id: PciDeviceId, is_bridge: bool) {
        if self.slots.iter().all(|s| !(s.bus == bus && s.dev == dev)) {
            self.slots.push(PciSlot {
                bus,
                dev,
                fns: Vec::new(),
            });
        }
        if let Some(slot) = self.slots.iter_mut().find(|s| s.bus == bus && s.dev == dev) {
            while slot.fns.len() <= fn_ as usize {
                slot.fns.push(PciFunction::new(PciDeviceId::default(), false));
            }
            slot.fns[fn_ as usize] = PciFunction::new(id, is_bridge);
        }
    }

    /// Walk every bus/device/function reachable from bus 0, descending into
    /// PCI-PCI bridges. Returns the number of present functions discovered.
    pub fn enumerate(&mut self) -> Result<usize, DeviceError> {
        self.discovered_buses.clear();
        let mut stack: Vec<u8> = vec![0];
        let mut count = 0usize;

        while let Some(bus) = stack.pop() {
            if bus > self.bus_limit || self.discovered_buses.contains(&bus) {
                continue;
            }
            self.discovered_buses.push(bus);

            for dev in 0u8..32 {
                let fn0_present = self
                    .find(bus, dev, 0)
                    .map(PciFunction::is_present)
                    .unwrap_or(false);
                if !fn0_present {
                    continue;
                }
                count += 1;
                if let Some(f) = self.find(bus, dev, 0) {
                    if let Some(sec) = f.secondary_bus() {
                        stack.push(sec);
                    }
                }

                // Scan remaining functions only if header type marks this
                // device as multi-function.
                let multi = self
                    .find(bus, dev, 0)
                    .map(|f| f.header_type() & HEADER_TYPE_MULTI_FUNCTION != 0)
                    .unwrap_or(false);
                if !multi {
                    continue;
                }
                for func in 1u8..8 {
                    let present = self
                        .find(bus, dev, func)
                        .map(PciFunction::is_present)
                        .unwrap_or(false);
                    if present {
                        count += 1;
                        if let Some(f) = self.find(bus, dev, func) {
                            if let Some(sec) = f.secondary_bus() {
                                stack.push(sec);
                            }
                        }
                    }
                }
            }
        }

        self.discovered_buses.sort_unstable();
        Ok(count)
    }

    /// Number of present functions currently registered.
    pub fn device_count(&self) -> usize {
        self.slots
            .iter()
            .flat_map(|s| s.fns.iter())
            .filter(|f| f.is_present())
            .count()
    }

    pub(crate) fn find(&self, bus: u8, device: u8, function: u8) -> Option<&PciFunction> {
        self.slots
            .iter()
            .find(|s| s.bus == bus && s.dev == device)
            .and_then(|s| s.fns.get(function as usize))
    }

    pub(crate) fn find_mut(
        &mut self,
        bus: u8,
        device: u8,
        function: u8,
    ) -> Option<&mut PciFunction> {
        self.slots
            .iter_mut()
            .find(|s| s.bus == bus && s.dev == device)
            .and_then(|s| s.fns.get_mut(function as usize))
    }

    /// Read a 32-bit dword from configuration space. Missing devices return
    /// all-ones, per the PCI spec.
    pub fn read_config(&self, bus: u8, device: u8, function: u8, offset: u8) -> u32 {
        match self.find(bus, device, function) {
            Some(f) if f.is_present() => f.read_dword(offset as usize),
            _ => PCI_DEVICE_MISSING,
        }
    }

    /// Write a 32-bit dword to configuration space.
    pub fn write_config(&mut self, bus: u8, device: u8, function: u8, offset: u8, value: u32) {
        if let Some(f) = self.find_mut(bus, device, function) {
            if f.is_present() {
                f.write_dword(offset as usize, value);
            }
        }
    }

    /// Read `size` bytes (1, 2, or 4) from configuration space, handling
    /// unaligned access by splitting on dword boundaries.
    fn read_config_bytes(
        &self,
        bus: u8,
        device: u8,
        function: u8,
        reg: usize,
        size: u8,
    ) -> Result<u64, DeviceError> {
        if size != 1 && size != 2 && size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        if reg + size as usize > 256 {
            return Err(DeviceError::InvalidAddress);
        }
        let mut result: u64 = 0;
        for i in 0..size as usize {
            let off = reg + i;
            let dword = self.read_config(bus, device, function, (off & 0xFC) as u8);
            let shift = (off & 3) * 8;
            result |= (((dword >> shift) & 0xFF) as u64) << (i * 8);
        }
        Ok(result)
    }

    /// Write `value` (low `size` bytes) to configuration space, handling
    /// unaligned access with read-modify-write on the containing dword.
    fn write_config_bytes(
        &mut self,
        bus: u8,
        device: u8,
        function: u8,
        reg: usize,
        value: u64,
        size: u8,
    ) -> Result<(), DeviceError> {
        if size != 1 && size != 2 && size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        if reg + size as usize > 256 {
            return Err(DeviceError::InvalidAddress);
        }
        for i in 0..size as usize {
            let off = reg + i;
            let byte = ((value >> (i * 8)) & 0xFF) as u8;
            let dword_off = off & 0xFC;
            let mut dword = self.read_config(bus, device, function, dword_off as u8);
            let shift = (off & 3) * 8;
            dword = (dword & !(0xFF << shift)) | ((byte as u32) << shift);
            self.write_config(bus, device, function, dword_off as u8, dword);
        }
        Ok(())
    }

    /// Configure the natural size/alignment of a BAR so that software-driven
    /// resource allocation (size probes) works. `size` must be a power of two.
    pub fn set_bar_size(
        &mut self,
        bus: u8,
        device: u8,
        function: u8,
        bar: usize,
        size: u32,
    ) -> Result<(), DeviceError> {
        let f = self
            .find_mut(bus, device, function)
            .ok_or(DeviceError::NotFound)?;
        if bar >= 6 || size == 0 || !size.is_power_of_two() {
            return Err(DeviceError::InvalidAddress);
        }
        // Bridges expose only two BARs.
        let bar_limit = if f.is_bridge { 2 } else { 6 };
        if bar >= bar_limit {
            return Err(DeviceError::InvalidAddress);
        }
        f.bar_sizes[bar] = size;
        Ok(())
    }

    /// Decode an ECAM address into (bus, device, function, register).
    fn ecam_offset(&self, addr: u64) -> Option<(u8, u8, u8, usize)> {
        let off = addr.wrapping_sub(self.ecam_base);
        if off >= 256 * 32 * 8 * 4096 {
            return None;
        }
        let bus = ((off >> 20) & 0xFF) as u8;
        let dev = ((off >> 15) & 0x1F) as u8;
        let func = ((off >> 12) & 0x07) as u8;
        let reg = (off & 0xFFF) as usize;
        Some((bus, dev, func, reg))
    }

    pub fn reset(&mut self) {
        self.config_address = 0;
        self.slots.clear();
        self.discovered_buses.clear();
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
                    return Ok(PCI_DEVICE_MISSING as u64);
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
        self.discovered_buses.clear();
    }
}

impl Device for PciHostBridge {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        let (bus, dev, func, reg) = self
            .ecam_offset(addr)
            .ok_or(DeviceError::InvalidAddress)?;
        self.read_config_bytes(bus, dev, func, reg, size)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        let (bus, dev, func, reg) = self
            .ecam_offset(addr)
            .ok_or(DeviceError::InvalidAddress)?;
        self.write_config_bytes(bus, dev, func, reg, value, size)
    }

    fn reset(&mut self) {
        PciHostBridge::reset(self);
    }
}

// ---------------------------------------------------------------------------
// Shared access via Rc<RefCell<>> so the VM can expose the same host bridge
// through both the legacy port config space and the ECAM (MMCONFIG) aperture.
// ---------------------------------------------------------------------------

impl Device for Rc<RefCell<PciHostBridge>> {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        let guard = self.borrow();
        Device::read(&*guard, addr, size)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        let mut guard = self.borrow_mut();
        Device::write(&mut *guard, addr, value, size)
    }

    fn reset(&mut self) {
        self.borrow_mut().reset();
    }
}

impl PortDevice for Rc<RefCell<PciHostBridge>> {
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

    fn ep_id() -> PciDeviceId {
        PciDeviceId {
            vendor: 0x1234,
            device: 0x1111,
            class: 0x02,
            subclass: 0x00,
            prog_if: 0x00,
            revision: 0x01,
        }
    }

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
        pci.add_device(0, 3, 0, ep_id());
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
    fn ecam_config_access() {
        let mut pci = PciHostBridge::new();
        pci.add_device(0, 3, 0, ep_id());

        // ECAM address for bus 0, device 3, function 0, register 0.
        let addr = pci.ecam_base() | (0 << 20) | (3 << 15) | (0 << 12) | 0;
        assert_eq!(Device::read(&pci, addr, 4).unwrap(), 0x1111_1234);

        // Byte access to the class/revision dword.
        let class_addr = pci.ecam_base() | (3 << 15) | 0x0C;
        assert_eq!(Device::read(&pci, class_addr, 1).unwrap(), 0x01); // revision
        assert_eq!(Device::read(&pci, class_addr + 3, 1).unwrap(), 0x02); // class

        // Out-of-range address faults.
        assert_eq!(
            Device::read(&pci, pci.ecam_base() + 256 * 32 * 8 * 4096, 4),
            Err(DeviceError::InvalidAddress)
        );
    }

    #[test]
    fn ecam_write_config() {
        let mut pci = PciHostBridge::new();
        pci.add_device(0, 0, 0, ep_id());

        // Write command register (offset 0x04) via ECAM.
        let cmd_addr = pci.ecam_base() | (0 << 15) | 0x04;
        Device::write(&mut pci, cmd_addr, 0x0000_0007, 4).unwrap();
        assert_eq!(pci.read_config(0, 0, 0, 0x04) & 0xFFFF, 0x0007);

        // Unaligned byte write to the interrupt line (offset 0x3C).
        let irq_addr = pci.ecam_base() | (0 << 15) | 0x3C;
        Device::write(&mut pci, irq_addr, 0x0A, 1).unwrap();
        assert_eq!(pci.read_config(0, 0, 0, 0x3C) & 0xFF, 0x0A);
    }

    #[test]
    fn bar_sizing_probe() {
        let mut pci = PciHostBridge::new();
        pci.add_device(0, 5, 0, ep_id());
        pci.set_bar_size(0, 5, 0, 0, 0x1000).unwrap(); // 4 KiB BAR0
        assert_eq!(pci.set_bar_size(0, 5, 0, 0, 3), Err(DeviceError::InvalidAddress));

        // Size probe: write all-ones, read back the size mask.
        pci.write_config(0, 5, 0, 0x10, 0xFFFF_FFFF);
        assert_eq!(pci.read_config(0, 5, 0, 0x10), 0xFFFF_F000);

        // Real base assignment.
        pci.write_config(0, 5, 0, 0x10, 0x8000_0000);
        assert_eq!(pci.read_config(0, 5, 0, 0x10), 0x8000_0000);
    }

    #[test]
    fn enumeration_with_bridge() {
        let mut pci = PciHostBridge::new();

        // Root bus device 0 = host bridge, device 1 = PCI-PCI bridge
        // to bus 7, device 2 on bus 1 (secondary 7).
        pci.add_device(0, 0, 0, ep_id());
        let bridge_id = PciDeviceId {
            vendor: 0x8086,
            device: 0x0001,
            class: 0x06,
            subclass: 0x04,
            prog_if: 0x00,
            revision: 0x00,
        };
        pci.add_bridge(0, 1, 0, bridge_id);
        // Set the bridge's secondary bus to 7 via config space.
        pci.write_config(0, 1, 0, 0x18, (7u32 << 8) | 7); // primary=0, secondary=7, subord=7
        pci.add_device(7, 2, 0, ep_id());

        let count = pci.enumerate().unwrap();
        assert_eq!(count, 3);
        assert_eq!(pci.discovered_buses(), &[0, 7]);

        // Bridges expose only 2 BARs; BAR 3 is rejected while BAR 1 works.
        pci.set_bar_size(0, 1, 0, 3, 0x1000).unwrap_err();
        pci.set_bar_size(0, 1, 0, 1, 0x1000).unwrap();
    }

    #[test]
    fn pcie_capability_present() {
        let mut pci = PciHostBridge::new();
        pci.add_device(0, 9, 0, ep_id());

        // Capabilities pointer at 0x34 points to offset 0x40.
        assert_eq!(pci.read_config(0, 9, 0, 0x34) & 0xFF, 0x40);
        // PCIe capability id.
        assert_eq!(pci.read_config(0, 9, 0, 0x40) & 0xFF, 0x10);
        // Version (low nibble) = 2, type (high nibble) = 0 (endpoint).
        // Byte 0x42 lives in the dword at 0x40, so shift it out.
        assert_eq!(((pci.read_config(0, 9, 0, 0x40) >> 16) & 0xFF), 0x02);
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