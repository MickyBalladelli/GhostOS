//! Legacy virtio-net device backed by the C controller.

use crate::devices::{ApicTrigger, Device, DeviceError, LocalApic, PortDevice};
use crate::memory::Mmu;
use crate::net::{MacAddress, NetBackend, NetError, NetQueueState};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

pub const VIRTIO_NET_VENDOR_ID: u16 = 0x1AF4;
pub const VIRTIO_NET_DEVICE_ID: u16 = 0x1000;
pub const VIRTIO_NET_CLASS: u8 = 0x02;
pub const VIRTIO_NET_SUBCLASS: u8 = 0x00;
pub const VIRTIO_NET_PROG_IF: u8 = 0x00;
pub const VIRTIO_NET_PCI_BAR0_SIZE: u64 = 0x100;

#[cfg(test)]
const REG_DEVICE_FEATURES: u16 = 0;
#[cfg(test)]
const REG_GUEST_FEATURES: u16 = 4;
#[cfg(test)]
const REG_QUEUE_PFN: u16 = 8;
#[cfg(test)]
const REG_QUEUE_SEL: u16 = 0x0e;
#[cfg(test)]
const REG_QUEUE_NOTIFY: u16 = 0x10;
#[cfg(test)]
const REG_STATUS: u16 = 0x12;
#[cfg(test)]
const REG_CONFIG: u16 = 0x14;
#[cfg(test)]
const DEVICE_FEATURES: u32 = (1 << 5) | (1 << 16);
#[cfg(test)]
const QUEUE_TX: u16 = 1;

#[repr(C)]
struct CIo {
    read_memory: unsafe extern "C" fn(*mut c_void, u64, *mut u8, usize) -> bool,
    write_memory: unsafe extern "C" fn(*mut c_void, u64, *const u8, usize) -> bool,
    receive: unsafe extern "C" fn(*mut c_void, *mut *const u8, *mut usize) -> i32,
    transmit: unsafe extern "C" fn(*mut c_void, *const u8, usize) -> i32,
    interrupt: unsafe extern "C" fn(*mut c_void),
    context: *mut c_void,
}

unsafe extern "C" {
    fn ghostos_vm_virtio_net_new(mac: *const u8) -> *mut c_void;
    fn ghostos_vm_virtio_net_free(net: *mut c_void);
    fn ghostos_vm_virtio_net_reset(net: *mut c_void);
    fn ghostos_vm_virtio_net_clear_rx(net: *mut c_void);
    fn ghostos_vm_virtio_net_pending(net: *const c_void) -> bool;
    fn ghostos_vm_virtio_net_notify(net: *mut c_void);
    fn ghostos_vm_virtio_net_take_error(net: *mut c_void) -> i32;
    fn ghostos_vm_virtio_net_read(net: *mut c_void, port: u16, carrier: bool) -> u64;
    fn ghostos_vm_virtio_net_write(net: *mut c_void, port: u16, value: u32);
    fn ghostos_vm_virtio_net_poll(net: *mut c_void, io: *const CIo) -> bool;
}

struct PollContext<'a> {
    mmu: &'a mut Mmu,
    backend: &'a mut Option<Box<dyn NetBackend>>,
    apic: &'a Option<Rc<RefCell<LocalApic>>>,
    vector: u8,
    received: Vec<u8>,
}

unsafe extern "C" fn read_memory(raw: *mut c_void, addr: u64, out: *mut u8, len: usize) -> bool {
    let context = unsafe { &mut *raw.cast::<PollContext<'_>>() };
    match context.mmu.read_phys(addr, len) {
        Ok(bytes) => {
            if len != 0 { unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, len) }; }
            true
        }
        Err(_) => false,
    }
}

unsafe extern "C" fn write_memory(raw: *mut c_void, addr: u64, bytes: *const u8, len: usize) -> bool {
    let context = unsafe { &mut *raw.cast::<PollContext<'_>>() };
    let bytes = if len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(bytes, len) } };
    context.mmu.write_phys(addr, bytes).is_ok()
}

unsafe extern "C" fn receive(raw: *mut c_void, packet: *mut *const u8, len: *mut usize) -> i32 {
    let context = unsafe { &mut *raw.cast::<PollContext<'_>>() };
    let Some(backend) = context.backend.as_mut() else { return -6 };
    match backend.receive() {
        Ok(Some(bytes)) => {
            context.received = bytes;
            unsafe { *packet = context.received.as_ptr(); *len = context.received.len(); }
            1
        }
        Ok(None) => 0,
        Err(error) => -(error as i32 + 1),
    }
}

unsafe extern "C" fn transmit(raw: *mut c_void, packet: *const u8, len: usize) -> i32 {
    let context = unsafe { &mut *raw.cast::<PollContext<'_>>() };
    let Some(backend) = context.backend.as_mut() else { return 5 };
    let packet = if len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(packet, len) } };
    backend.transmit(packet).map_or_else(|error| error as i32, |_| -1)
}

unsafe extern "C" fn interrupt(raw: *mut c_void) {
    let context = unsafe { &mut *raw.cast::<PollContext<'_>>() };
    if context.vector != 0 {
        if let Some(apic) = context.apic {
            apic.borrow_mut().signal(context.vector, ApicTrigger::Edge);
        }
    }
}

pub struct VirtioNet {
    state: *mut c_void,
    mac: MacAddress,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
    backend: Option<Box<dyn NetBackend>>,
}

impl VirtioNet {
    pub fn new(mac: MacAddress) -> Self {
        let state = unsafe { ghostos_vm_virtio_net_new(mac.to_bytes().as_ptr()) };
        assert!(!state.is_null(), "C virtio-net allocation failed");
        Self { state, mac, apic: None, irq_vector: 0, backend: None }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) { self.apic = Some(apic); }
    pub fn set_irq_vector(&mut self, vector: u8) { self.irq_vector = vector; }
    pub fn attach_backend(&mut self, backend: Box<dyn NetBackend>) {
        unsafe { ghostos_vm_virtio_net_clear_rx(self.state) };
        self.backend = Some(backend);
    }
    pub fn mac(&self) -> MacAddress { self.mac }
    pub fn carrier_up(&self) -> bool { self.backend.as_ref().is_some_and(|backend| backend.link_up()) }
    pub fn admin_up(&self) -> bool { self.backend.as_ref().is_none_or(|backend| backend.admin_up()) }
    pub fn queue_state(&self) -> NetQueueState {
        self.backend.as_ref().map_or(NetQueueState::EMPTY, |backend| backend.queue_state())
    }
    pub fn set_admin_up(&mut self, up: bool) {
        if let Some(backend) = &mut self.backend { backend.set_admin_up(up); }
    }
    pub fn take_network_error(&mut self) -> Option<NetError> {
        match unsafe { ghostos_vm_virtio_net_take_error(self.state) } {
            0 => Some(NetError::PacketTooLarge),
            1 => Some(NetError::Truncated),
            2 => Some(NetError::QueueFull),
            3 => Some(NetError::LinkDown),
            4 => Some(NetError::AdminDown),
            5 => Some(NetError::BackendUnavailable),
            _ => None,
        }
    }
    pub fn has_pending(&self) -> bool { unsafe { ghostos_vm_virtio_net_pending(self.state) } }
    fn read_io(&mut self, port: u16) -> u64 {
        let carrier = port & 0xff == 0x1a && self.carrier_up();
        unsafe { ghostos_vm_virtio_net_read(self.state, port, carrier) }
    }
    fn write_io(&mut self, port: u16, value: u32) {
        unsafe { ghostos_vm_virtio_net_write(self.state, port, value) };
    }
    pub fn on_notify(&mut self, _q: u16) { unsafe { ghostos_vm_virtio_net_notify(self.state) }; }
    pub fn poll(&mut self, mmu: &mut Mmu) {
        let mut context = PollContext {
            mmu, backend: &mut self.backend, apic: &self.apic,
            vector: self.irq_vector, received: Vec::new(),
        };
        let io = CIo { read_memory, write_memory, receive, transmit, interrupt,
            context: (&mut context as *mut PollContext<'_>).cast() };
        assert!(unsafe { ghostos_vm_virtio_net_poll(self.state, &io) }, "C virtio-net allocation failed");
    }
    pub fn reset(&mut self) { unsafe { ghostos_vm_virtio_net_reset(self.state) }; }
}

impl Drop for VirtioNet {
    fn drop(&mut self) { unsafe { ghostos_vm_virtio_net_free(self.state) }; }
}

impl PortDevice for VirtioNet {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        if size != 1 && size != 2 && size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        Ok(self.read_io(port))
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 1 && size != 2 && size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        self.write_io(port, value as u32);
        Ok(())
    }

    fn reset(&mut self) {
        VirtioNet::reset(self);
    }
}

impl Device for VirtioNet {
    fn read(&self, _addr: u64, _size: u8) -> Result<u64, DeviceError> {
        Err(DeviceError::InvalidAddress)
    }

    fn write(&mut self, _addr: u64, _value: u64, _size: u8) -> Result<(), DeviceError> {
        Err(DeviceError::InvalidAddress)
    }

    fn reset(&mut self) {
        VirtioNet::reset(self);
    }
}

impl Device for Rc<RefCell<VirtioNet>> {
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

impl PortDevice for Rc<RefCell<VirtioNet>> {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        PortDevice::read(&mut *self.borrow_mut(), port, size)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        PortDevice::write(&mut *self.borrow_mut(), port, value, size)
    }

    fn reset(&mut self) {
        self.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{LoopbackHub, LoopbackPort};

    #[test]
    fn feature_mac_queue_status_and_reset_round_trip() {
        let mac = MacAddress::ghostos_default(3);
        let mut net = VirtioNet::new(mac);
        assert_eq!(net.read_io(REG_DEVICE_FEATURES), DEVICE_FEATURES as u64);
        assert_eq!(net.read_io(REG_CONFIG), mac.0[0] as u64);
        assert_eq!(net.read_io(REG_CONFIG + 5), mac.0[5] as u64);

        net.write_io(REG_GUEST_FEATURES, 0x10000);
        net.write_io(REG_QUEUE_SEL, QUEUE_TX as u32);
        net.write_io(REG_QUEUE_PFN, 4);
        net.write_io(REG_STATUS, 4);
        assert_eq!(net.read_io(REG_QUEUE_PFN), 4);
        assert_eq!(net.read_io(REG_STATUS), 4);
        net.write_io(REG_QUEUE_NOTIFY, 0);
        assert!(net.has_pending());

        net.reset();
        assert_eq!(net.read_io(REG_STATUS), 0);
        assert_eq!(net.read_io(REG_QUEUE_PFN), 0);
    }

    #[test]
    fn loopback_backend_is_attached_and_invalid_access_is_rejected() {
        let hub = Rc::new(RefCell::new(LoopbackHub::new()));
        let mac = MacAddress::ghostos_default(4);
        let mut net = VirtioNet::new(mac);
        net.attach_backend(Box::new(LoopbackPort::new(hub, 0, mac)));
        assert!(net.backend.as_ref().unwrap().link_up());
        assert_eq!(PortDevice::read(&mut net, REG_STATUS, 8), Err(DeviceError::UnsupportedSize));
        assert_eq!(Device::read(&net, 0, 4), Err(DeviceError::InvalidAddress));
    }
}
