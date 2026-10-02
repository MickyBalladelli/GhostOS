//! Guest integration devices: an agent mailbox, a paravirtual clock, and
//! memory hot-plug state.

use super::{ApicTrigger, Device, DeviceError, LocalApic};
use crate::memory::Mmu;
use crate::replay::SharedReplay;
use std::cell::RefCell;
use std::ffi::c_void;
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

#[repr(C)]
struct CAgent {
    _private: [u8; 0],
}

#[repr(C)]
struct CEvent {
    kind: u32,
    base: u64,
    size: u64,
}

#[derive(Default)]
#[repr(C)]
struct CHotplug {
    current: u64,
    maximum: u64,
    pending_base: u64,
    pending_size: u64,
    request: u64,
    has_pending: bool,
    has_request: bool,
}

#[derive(Default)]
#[repr(C)]
struct CPvClock {
    system_time_page: u64,
    wall_clock_page: u64,
    version: u32,
    has_system_time_page: bool,
    has_wall_clock_page: bool,
}

const _: () = {
    assert!(std::mem::size_of::<CEvent>() == 24);
    assert!(std::mem::offset_of!(CEvent, base) == 8);
    assert!(std::mem::size_of::<CHotplug>() == 48);
    assert!(std::mem::align_of::<CHotplug>() == 8);
    assert!(std::mem::offset_of!(CHotplug, has_pending) == 40);
    assert!(std::mem::size_of::<CPvClock>() == 24);
    assert!(std::mem::align_of::<CPvClock>() == 8);
    assert!(std::mem::offset_of!(CPvClock, version) == 16);
    assert!(std::mem::offset_of!(CPvClock, has_system_time_page) == 20);
};

type MemoryWrite = unsafe extern "C" fn(*mut c_void, u64, *const u8, usize);
type WallTime = unsafe extern "C" fn(*mut c_void) -> u64;

unsafe extern "C" {
    fn ghostos_vm_guest_agent_new() -> *mut CAgent;
    fn ghostos_vm_guest_agent_free(agent: *mut CAgent);
    fn ghostos_vm_guest_agent_clear(agent: *mut CAgent);
    fn ghostos_vm_guest_agent_send(agent: *mut CAgent, bytes: *const u8, length: usize) -> bool;
    fn ghostos_vm_guest_agent_output_length(agent: *const CAgent) -> usize;
    fn ghostos_vm_guest_agent_take_output(agent: *mut CAgent, output: *mut u8, capacity: usize) -> usize;
    fn ghostos_vm_guest_agent_notify(agent: *mut CAgent, event: *const CEvent) -> bool;
    fn ghostos_vm_guest_agent_pending_events(agent: *const CAgent) -> usize;
    fn ghostos_vm_guest_agent_read(agent: *mut CAgent, address: u64, size: u8, value: *mut u64) -> u8;
    fn ghostos_vm_guest_agent_write(agent: *mut CAgent, address: u64, value: u64, size: u8) -> u8;
    fn ghostos_vm_memory_hotplug_init(state: *mut CHotplug, current: u64, maximum: u64);
    fn ghostos_vm_memory_hotplug_add(state: *mut CHotplug, base: u64, size: u64) -> bool;
    fn ghostos_vm_memory_hotplug_take_request(state: *mut CHotplug, request: *mut u64) -> bool;
    fn ghostos_vm_memory_hotplug_clear_pending(state: *mut CHotplug);
    fn ghostos_vm_memory_hotplug_reset(state: *mut CHotplug);
    fn ghostos_vm_memory_hotplug_read(state: *const CHotplug, address: u64, size: u8, value: *mut u64) -> u8;
    fn ghostos_vm_memory_hotplug_write(state: *mut CHotplug, address: u64, value: u64, size: u8) -> u8;
    fn ghostos_vm_pvclock_reset(state: *mut CPvClock);
    fn ghostos_vm_pvclock_write_msr(state: *mut CPvClock, msr: u32, value: u64);
    fn ghostos_vm_pvclock_configured(state: *const CPvClock) -> bool;
    fn ghostos_vm_pvclock_update(state: *mut CPvClock, monotonic_ns: u64, wall_time: WallTime,
        write: MemoryWrite, context: *mut c_void);
}

fn device_result(code: u8) -> Result<(), DeviceError> {
    match code {
        0 => Ok(()),
        1 => Err(DeviceError::UnsupportedSize),
        2 => Err(DeviceError::InvalidAddress),
        3 => panic!("could not allocate guest mailbox queue"),
        _ => Err(DeviceError::AccessDenied),
    }
}

struct GuestAgentState {
    device: *mut CAgent,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

/// Stable C-owned guest mailbox with the VM's shared APIC adapter.
pub struct GuestAgent {
    state: RefCell<GuestAgentState>,
}

impl GuestAgent {
    pub fn new() -> Self {
        let device = unsafe { ghostos_vm_guest_agent_new() };
        assert!(!device.is_null(), "could not allocate guest mailbox");
        Self { state: RefCell::new(GuestAgentState {
            device, apic: None, irq_vector: GUEST_AGENT_IRQ_VECTOR,
        }) }
    }

    pub fn attach_apic(&self, apic: Rc<RefCell<LocalApic>>) {
        self.state.borrow_mut().apic = Some(apic)
    }

    pub fn set_irq_vector(&self, vector: u8) {
        self.state.borrow_mut().irq_vector = vector
    }

    pub fn send_to_guest(&self, bytes: &[u8]) {
        if bytes.is_empty() { return }
        let state = self.state.borrow_mut();
        let queued = unsafe { ghostos_vm_guest_agent_send(state.device, bytes.as_ptr(), bytes.len()) };
        assert!(queued, "could not allocate guest mailbox input");
        signal_irq(&state.apic, state.irq_vector)
    }

    pub fn take_from_guest(&self) -> Vec<u8> {
        let state = self.state.borrow_mut();
        let length = unsafe { ghostos_vm_guest_agent_output_length(state.device) };
        let mut output = vec![0; length];
        let taken = unsafe {
            ghostos_vm_guest_agent_take_output(state.device, output.as_mut_ptr(), output.len())
        };
        output.truncate(taken);
        output
    }

    pub fn guest_to_host_pending(&self) -> bool {
        unsafe { ghostos_vm_guest_agent_output_length(self.state.borrow().device) != 0 }
    }

    pub fn notify(&self, event: GuestEvent) {
        let event = match event {
            GuestEvent::Shutdown => CEvent { kind: 1, base: 0, size: 0 },
            GuestEvent::Reboot => CEvent { kind: 2, base: 0, size: 0 },
            GuestEvent::MemoryAdded { base, size } => CEvent { kind: 3, base, size },
        };
        let state = self.state.borrow_mut();
        let queued = unsafe { ghostos_vm_guest_agent_notify(state.device, &event) };
        assert!(queued, "could not allocate guest mailbox event");
        signal_irq(&state.apic, state.irq_vector)
    }

    pub fn pending_events(&self) -> usize {
        unsafe { ghostos_vm_guest_agent_pending_events(self.state.borrow().device) }
    }

    pub fn clear(&self) {
        unsafe { ghostos_vm_guest_agent_clear(self.state.borrow_mut().device) }
    }
}

impl Drop for GuestAgent {
    fn drop(&mut self) {
        unsafe { ghostos_vm_guest_agent_free(self.state.get_mut().device) }
    }
}

impl Default for GuestAgent {
    fn default() -> Self { Self::new() }
}

impl Device for GuestAgent {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        let mut value = 0;
        let code = unsafe {
            ghostos_vm_guest_agent_read(self.state.borrow_mut().device, addr, size, &mut value)
        };
        device_result(code)?;
        Ok(value)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        device_result(unsafe {
            ghostos_vm_guest_agent_write(self.state.borrow_mut().device, addr, value, size)
        })
    }

    fn reset(&mut self) { self.clear() }
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
    device: CHotplug,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

/// C-owned guest memory hot-plug registers and request state.
pub struct MemoryHotplugDevice {
    state: RefCell<MemoryHotplugState>,
}

impl MemoryHotplugDevice {
    pub fn new(current: u64, maximum: u64) -> Self {
        let mut device = CHotplug::default();
        unsafe { ghostos_vm_memory_hotplug_init(&mut device, current, maximum) };
        Self { state: RefCell::new(MemoryHotplugState {
            device, apic: None, irq_vector: MEMORY_HOTPLUG_IRQ_VECTOR,
        }) }
    }

    pub fn attach_apic(&self, apic: Rc<RefCell<LocalApic>>) {
        self.state.borrow_mut().apic = Some(apic)
    }

    pub fn current_memory(&self) -> u64 {
        self.state.borrow().device.current
    }

    pub fn add_region(&self, base: u64, size: u64) {
        let mut state = self.state.borrow_mut();
        if unsafe { ghostos_vm_memory_hotplug_add(&mut state.device, base, size) } {
            signal_irq(&state.apic, state.irq_vector)
        }
    }

    pub fn take_request(&self) -> Option<u64> {
        let mut request = 0;
        let pending = unsafe {
            ghostos_vm_memory_hotplug_take_request(&mut self.state.borrow_mut().device, &mut request)
        };
        pending.then_some(request)
    }

    pub fn clear_pending(&self) {
        unsafe { ghostos_vm_memory_hotplug_clear_pending(&mut self.state.borrow_mut().device) }
    }
}

impl Device for MemoryHotplugDevice {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        let mut value = 0;
        let code = unsafe {
            ghostos_vm_memory_hotplug_read(&self.state.borrow().device, addr, size, &mut value)
        };
        device_result(code)?;
        Ok(value)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        device_result(unsafe {
            ghostos_vm_memory_hotplug_write(&mut self.state.borrow_mut().device, addr, value, size)
        })
    }

    fn reset(&mut self) {
        unsafe { ghostos_vm_memory_hotplug_reset(&mut self.state.borrow_mut().device) }
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

/// C-owned KVM-compatible pvclock state and shared time-page encoding.
/// Rust supplies host/replay wall time and MMU writes.
pub struct PvClock {
    state: CPvClock,
    replay: Option<SharedReplay>,
}

struct ClockUpdateContext<'a> {
    mmu: &'a mut Mmu,
    replay: &'a Option<SharedReplay>,
}

unsafe extern "C" fn write_clock_memory(context: *mut c_void, address: u64,
    bytes: *const u8, length: usize) {
    // C calls synchronously with live stack buffers and the exclusive MMU.
    let context = unsafe { &mut *context.cast::<ClockUpdateContext<'_>>() };
    let bytes = unsafe { std::slice::from_raw_parts(bytes, length) };
    let _ = context.mmu.write_phys(address, bytes);
}

unsafe extern "C" fn clock_wall_time(context: *mut c_void) -> u64 {
    let context = unsafe { &mut *context.cast::<ClockUpdateContext<'_>>() };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let host_ns = now.as_nanos().min(u64::MAX as u128) as u64;
    context.replay.as_ref().map(|replay| {
        replay.borrow_mut().instruction_input(0, 0x5056_434C_4F43_4B54, 8, host_ns)
            .unwrap_or(host_ns)
    }).unwrap_or(host_ns)
}

impl PvClock {
    pub fn new() -> Self {
        Self { state: CPvClock::default(), replay: None }
    }

    pub fn attach_replay(&mut self, replay: SharedReplay) {
        self.replay = Some(replay)
    }

    pub fn write_msr(&mut self, msr: u32, value: u64) {
        unsafe { ghostos_vm_pvclock_write_msr(&mut self.state, msr, value) }
    }

    pub fn configured(&self) -> bool {
        unsafe { ghostos_vm_pvclock_configured(&self.state) }
    }

    pub fn reset(&mut self) {
        unsafe { ghostos_vm_pvclock_reset(&mut self.state) }
    }

    pub fn update(&mut self, mmu: &mut Mmu, monotonic_ns: u64) {
        let mut context = ClockUpdateContext { mmu, replay: &self.replay };
        unsafe {
            ghostos_vm_pvclock_update(&mut self.state, monotonic_ns, clock_wall_time,
                write_clock_memory, (&mut context as *mut ClockUpdateContext<'_>).cast())
        }
    }
}

impl Default for PvClock {
    fn default() -> Self { Self::new() }
}

fn signal_irq(apic: &Option<Rc<RefCell<LocalApic>>>, vector: u8) {
    if let Some(apic) = apic {
        apic.borrow_mut().signal(vector, ApicTrigger::Edge)
    }
}
