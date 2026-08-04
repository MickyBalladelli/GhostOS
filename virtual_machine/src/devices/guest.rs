//! Guest integration devices: an agent mailbox, a paravirtual clock, and
//! memory hot-plug state.

use super::{ApicTrigger, Device, DeviceError, LocalApic};
use crate::memory::Mmu;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

pub const GUEST_AGENT_MMIO_BASE: u64 = 0xFEBF_0000;
pub const GUEST_AGENT_MMIO_SIZE: u64 = 0x1000;
pub const MEMORY_HOTPLUG_MMIO_BASE: u64 = 0xFEBE_0000;
pub const MEMORY_HOTPLUG_MMIO_SIZE: u64 = 0x1000;

pub const GUEST_AGENT_MAGIC: u32 = u32::from_le_bytes(*b"SOGA");
pub const GUEST_AGENT_VERSION: u32 = 1;
pub const GUEST_AGENT_FEATURE_AGENT: u64 = 1 << 0;
pub const GUEST_AGENT_FEATURE_PV_CLOCK: u64 = 1 << 1;
pub const GUEST_AGENT_FEATURE_POWER_EVENTS: u64 = 1 << 2;
pub const GUEST_AGENT_FEATURE_MEMORY_HOTPLUG: u64 = 1 << 3;

pub const GUEST_AGENT_IRQ_VECTOR: u8 = 0x35;
pub const MEMORY_HOTPLUG_IRQ_VECTOR: u8 = 0x36;

pub const KVM_SYSTEM_TIME_NEW: u32 = 0x4B56_4D01;
pub const KVM_WALL_CLOCK_NEW: u32 = 0x4B56_4D00;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuestEvent {
    Shutdown,
    Reboot,
    MemoryAdded { base: u64, size: u64 },
}

impl GuestEvent {
    fn kind(&self) -> u32 {
        match self {
            Self::Shutdown => 1,
            Self::Reboot => 2,
            Self::MemoryAdded { .. } => 3,
        }
    }

    fn data(&self) -> u64 {
        match self {
            Self::MemoryAdded { base, .. } => *base,
            _ => 0,
        }
    }

    fn size(&self) -> u64 {
        match self {
            Self::MemoryAdded { size, .. } => *size,
            _ => 0,
        }
    }
}

struct GuestAgentState {
    host_to_guest: VecDeque<u8>,
    guest_to_host: VecDeque<u8>,
    events: VecDeque<GuestEvent>,
    active_event: Option<GuestEvent>,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

/// A small, stable mailbox for a guest agent. The guest sees it at
/// `GUEST_AGENT_MMIO_BASE`; host code uses the queue methods instead of
/// reaching into device internals.
pub struct GuestAgent {
    state: RefCell<GuestAgentState>,
}

impl GuestAgent {
    pub fn new() -> Self {
        Self {
            state: RefCell::new(GuestAgentState {
                host_to_guest: VecDeque::new(),
                guest_to_host: VecDeque::new(),
                events: VecDeque::new(),
                active_event: None,
                apic: None,
                irq_vector: GUEST_AGENT_IRQ_VECTOR,
            }),
        }
    }

    pub fn attach_apic(&self, apic: Rc<RefCell<LocalApic>>) {
        self.state.borrow_mut().apic = Some(apic)
    }

    pub fn set_irq_vector(&self, vector: u8) {
        self.state.borrow_mut().irq_vector = vector
    }

    pub fn send_to_guest(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return
        }
        let mut state = self.state.borrow_mut();
        state.host_to_guest.extend(bytes.iter().copied());
        signal_irq(&state.apic, state.irq_vector)
    }

    pub fn take_from_guest(&self) -> Vec<u8> {
        self.state.borrow_mut().guest_to_host.drain(..).collect()
    }

    pub fn guest_to_host_pending(&self) -> bool {
        !self.state.borrow().guest_to_host.is_empty()
    }

    pub fn notify(&self, event: GuestEvent) {
        let mut state = self.state.borrow_mut();
        state.events.push_back(event);
        signal_irq(&state.apic, state.irq_vector)
    }

    pub fn pending_events(&self) -> usize {
        self.state.borrow().events.len()
    }

    pub fn clear(&self) {
        let mut state = self.state.borrow_mut();
        state.host_to_guest.clear();
        state.guest_to_host.clear();
        state.events.clear();
        state.active_event = None;
    }

    fn status(&self) -> u32 {
        let state = self.state.borrow();
        let mut status = 0;
        if !state.host_to_guest.is_empty() {
            status |= 1;
        }
        if !state.guest_to_host.is_empty() {
            status |= 1 << 1;
        }
        if !state.events.is_empty() || state.active_event.is_some() {
            status |= 1 << 2;
        }
        status
    }
}

impl Default for GuestAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl Device for GuestAgent {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        let offset = addr.saturating_sub(GUEST_AGENT_MMIO_BASE);
        match offset {
            0x00 if size == 4 => Ok(GUEST_AGENT_MAGIC as u64),
            0x04 if size == 4 => Ok(GUEST_AGENT_VERSION as u64),
            0x08 if size == 8 => Ok((GUEST_AGENT_FEATURE_AGENT
                | GUEST_AGENT_FEATURE_PV_CLOCK
                | GUEST_AGENT_FEATURE_POWER_EVENTS
                | GUEST_AGENT_FEATURE_MEMORY_HOTPLUG) as u64),
            0x0C if size == 4 => Ok(self.status() as u64),
            0x10 if size == 1 => Ok(self
                .state
                .borrow_mut()
                .host_to_guest
                .pop_front()
                .unwrap_or(0) as u64),
            0x18 if size == 4 => {
                let mut state = self.state.borrow_mut();
                state.active_event = state.events.pop_front();
                Ok(state.active_event.as_ref().map(GuestEvent::kind).unwrap_or(0) as u64)
            }
            0x20 if size == 8 => Ok(self
                .state
                .borrow()
                .active_event
                .as_ref()
                .map(GuestEvent::data)
                .unwrap_or(0)),
            0x28 if size == 8 => Ok(self
                .state
                .borrow()
                .active_event
                .as_ref()
                .map(GuestEvent::size)
                .unwrap_or(0)),
            _ => Err(DeviceError::UnsupportedSize),
        }
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        let offset = addr.saturating_sub(GUEST_AGENT_MMIO_BASE);
        match offset {
            0x14 if size == 1 => {
                self.state.borrow_mut().guest_to_host.push_back(value as u8);
                Ok(())
            }
            0x1C if size == 4 => {
                if value as u32 != 0 {
                    self.state.borrow_mut().active_event = None;
                }
                Ok(())
            }
            0x2C if size == 4 => {
                if value & 1 != 0 {
                    self.clear();
                }
                Ok(())
            }
            _ => Err(DeviceError::UnsupportedSize),
        }
    }

    fn reset(&mut self) {
        self.clear()
    }
}

impl Device for Rc<RefCell<GuestAgent>> {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        self.borrow().read(addr, size)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        self.borrow_mut().write(addr, value, size)
    }

    fn reset(&mut self) {
        self.borrow_mut().reset()
    }
}

struct MemoryHotplugState {
    current: u64,
    maximum: u64,
    pending: Option<(u64, u64)>,
    request: Option<u64>,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

/// Guest-visible memory hot-plug control and notification registers.
pub struct MemoryHotplugDevice {
    state: RefCell<MemoryHotplugState>,
}

impl MemoryHotplugDevice {
    pub fn new(current: u64, maximum: u64) -> Self {
        Self {
            state: RefCell::new(MemoryHotplugState {
                current,
                maximum: maximum.max(current),
                pending: None,
                request: None,
                apic: None,
                irq_vector: MEMORY_HOTPLUG_IRQ_VECTOR,
            }),
        }
    }

    pub fn attach_apic(&self, apic: Rc<RefCell<LocalApic>>) {
        self.state.borrow_mut().apic = Some(apic)
    }

    pub fn current_memory(&self) -> u64 {
        self.state.borrow().current
    }

    pub fn add_region(&self, base: u64, size: u64) {
        let mut state = self.state.borrow_mut();
        if size == 0 || state.current.saturating_add(size) > state.maximum {
            return
        }
        state.current = state.current.saturating_add(size);
        state.pending = Some((base, size));
        signal_irq(&state.apic, state.irq_vector)
    }

    pub fn take_request(&self) -> Option<u64> {
        self.state.borrow_mut().request.take()
    }

    pub fn clear_pending(&self) {
        self.state.borrow_mut().pending = None
    }
}

impl Device for MemoryHotplugDevice {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        if size != 8 {
            return Err(DeviceError::UnsupportedSize)
        }
        let offset = addr.saturating_sub(MEMORY_HOTPLUG_MMIO_BASE);
        let state = self.state.borrow();
        let value = match offset {
            0x00 => state.current,
            0x08 => state.maximum,
            0x10 => state.pending.map(|(base, _)| base).unwrap_or(0),
            0x18 => state.pending.map(|(_, size)| size).unwrap_or(0),
            0x20 => u64::from(state.pending.is_some()),
            _ => return Err(DeviceError::InvalidAddress),
        };
        Ok(value)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 8 {
            return Err(DeviceError::UnsupportedSize)
        }
        let offset = addr.saturating_sub(MEMORY_HOTPLUG_MMIO_BASE);
        let mut state = self.state.borrow_mut();
        match offset {
            0x28 => {
                if value & 1 != 0 {
                    state.pending = None;
                }
            }
            0x30 => {
                if value == 0 || value % 4096 != 0 || state.current.saturating_add(value) > state.maximum {
                    return Err(DeviceError::AccessDenied)
                }
                state.request = Some(value);
            }
            _ => return Err(DeviceError::InvalidAddress),
        }
        Ok(())
    }

    fn reset(&mut self) {
        let mut state = self.state.borrow_mut();
        state.pending = None;
        state.request = None;
    }
}

impl Device for Rc<RefCell<MemoryHotplugDevice>> {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        self.borrow().read(addr, size)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        self.borrow_mut().write(addr, value, size)
    }

    fn reset(&mut self) {
        self.borrow_mut().reset()
    }
}

/// KVM-compatible pvclock MSR state. The guest supplies the physical address
/// of its shared time page with WRMSR; the VM refreshes that page every run
/// loop iteration.
pub struct PvClock {
    system_time_page: Option<u64>,
    wall_clock_page: Option<u64>,
    version: u32,
}

impl PvClock {
    pub fn new() -> Self {
        Self {
            system_time_page: None,
            wall_clock_page: None,
            version: 0,
        }
    }

    pub fn write_msr(&mut self, msr: u32, value: u64) {
        let enabled = value & 1 != 0;
        let page = value & !0xFFF;
        match msr {
            KVM_SYSTEM_TIME_NEW => self.system_time_page = enabled.then_some(page),
            KVM_WALL_CLOCK_NEW => self.wall_clock_page = enabled.then_some(page),
            _ => {}
        }
    }

    pub fn configured(&self) -> bool {
        self.system_time_page.is_some() || self.wall_clock_page.is_some()
    }

    pub fn reset(&mut self) {
        self.system_time_page = None;
        self.wall_clock_page = None;
        self.version = 0;
    }

    pub fn update(&mut self, mmu: &mut Mmu, monotonic_ns: u64) {
        if let Some(page) = self.system_time_page {
            self.version = self.version.wrapping_add(1) | 1;
            let mut info = [0u8; 32];
            info[0..4].copy_from_slice(&self.version.to_le_bytes());
            info[16..24].copy_from_slice(&monotonic_ns.to_le_bytes());
            info[24..28].copy_from_slice(&1u32.to_le_bytes());
            info[29] = 1;
            let _ = mmu.write_phys(page, &info);
            self.version = self.version.wrapping_add(1) & !1;
            let _ = mmu.write_phys(page, &self.version.to_le_bytes());
        }

        if let Some(page) = self.wall_clock_page {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let mut wall_clock = [0u8; 12];
            wall_clock[4..8].copy_from_slice(&(now.as_secs() as u32).to_le_bytes());
            wall_clock[8..12].copy_from_slice(&now.subsec_nanos().to_le_bytes());
            let _ = mmu.write_phys(page, &wall_clock);
        }
    }
}

impl Default for PvClock {
    fn default() -> Self {
        Self::new()
    }
}

fn signal_irq(apic: &Option<Rc<RefCell<LocalApic>>>, vector: u8) {
    if let Some(apic) = apic {
        apic.borrow_mut().signal(vector, ApicTrigger::Edge)
    }
}
