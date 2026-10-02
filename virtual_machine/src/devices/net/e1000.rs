//! Intel e1000 NIC emulation (82540EM, PCI ID 8086:100E).
//!
//! Exposes the legacy RX/TX descriptor ring model over a 128 KiB MMIO BAR.
//! Rings are processed in [`E1000::poll`], called from the machine loop
//! outside the CPU step so guest physical memory can be borrowed safely.

use crate::devices::{ApicTrigger, Device, DeviceError, LocalApic};
use crate::memory::Mmu;
use crate::net::{MacAddress, NetBackend, NetError, NetQueueState};
use std::cell::RefCell;
use std::rc::Rc;
use std::ffi::c_void;

pub const E1000_MMIO_SIZE: u64 = 0x20000;
pub const E1000_VENDOR_ID: u16 = 0x8086;
pub const E1000_DEVICE_ID: u16 = 0x100E;
pub const E1000_CLASS: u8 = 0x02;
pub const E1000_SUBCLASS: u8 = 0x00;
pub const E1000_PROG_IF: u8 = 0x00;

#[cfg(test)]
const REG_CTRL: usize = 0x0000;
#[cfg(test)]
const REG_STATUS: usize = 0x0008;
#[cfg(test)]
const REG_ICR: usize = 0x00C0;
#[cfg(test)]
const REG_IMS: usize = 0x00D0;
#[cfg(test)]
const REG_RCTL: usize = 0x0100;
#[cfg(test)]
const REG_TCTL: usize = 0x0400;
#[cfg(test)]
const REG_RDBAL: usize = 0x2800;
#[cfg(test)]
const REG_RDLEN: usize = 0x2808;
#[cfg(test)]
const REG_RDT: usize = 0x2818;
#[cfg(test)]
const REG_TDBAL: usize = 0x3800;
#[cfg(test)]
const REG_TDLEN: usize = 0x3808;
#[cfg(test)]
const REG_RAL0: usize = 0x5400;
#[cfg(test)]
const REG_RAH0: usize = 0x5404;
#[cfg(test)]
const CTRL_RST: u32 = 1 << 26;
#[cfg(test)]
const STATUS_LU: u32 = 1 << 1;
#[cfg(test)]
const RCTL_EN: u32 = 1 << 0;
#[cfg(test)]
const TCTL_EN: u32 = 1 << 0;
#[cfg(test)]
const ICR_TXDW: u32 = 1 << 0;

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
    fn ghostos_vm_e1000_new(mac: *const u8) -> *mut c_void;
    fn ghostos_vm_e1000_free(net: *mut c_void);
    fn ghostos_vm_e1000_reset(net: *mut c_void);
    fn ghostos_vm_e1000_clear_rx(net: *mut c_void);
    fn ghostos_vm_e1000_take_error(net: *mut c_void) -> i32;
    fn ghostos_vm_e1000_read(net: *const c_void, address: u64, size: u8, carrier: bool, output: *mut u64) -> u32;
    fn ghostos_vm_e1000_write(net: *mut c_void, address: u64, size: u8, value: u32, io: *const CIo) -> u32;
    fn ghostos_vm_e1000_poll(net: *mut c_void, io: *const CIo, checked: bool) -> u32;
    #[cfg(test)]
    fn ghostos_vm_e1000_signal(net: *mut c_void, cause: u32, io: *const CIo);
}

struct PollContext<'a> {
    mmu: Option<&'a mut Mmu>,
    backend: &'a mut Option<Box<dyn NetBackend>>,
    apic: &'a Option<Rc<RefCell<LocalApic>>>,
    vector: u8,
    received: Vec<u8>,
}

impl PollContext<'_> {
    fn io(&mut self) -> CIo {
        CIo { read_memory, write_memory, receive, transmit, interrupt,
            context: (self as *mut PollContext<'_>).cast() }
    }
}

unsafe extern "C" fn read_memory(raw: *mut c_void, address: u64, output: *mut u8, length: usize) -> bool {
    let context = unsafe { &mut *raw.cast::<PollContext<'_>>() };
    let Some(mmu) = context.mmu.as_ref() else { return false };
    match mmu.read_phys(address, length) {
        Ok(bytes) => {
            if length != 0 { unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, length) }; }
            true
        }
        Err(_) => false,
    }
}

unsafe extern "C" fn write_memory(raw: *mut c_void, address: u64, bytes: *const u8, length: usize) -> bool {
    let context = unsafe { &mut *raw.cast::<PollContext<'_>>() };
    let Some(mmu) = context.mmu.as_mut() else { return false };
    let bytes = if length == 0 { &[] } else { unsafe { std::slice::from_raw_parts(bytes, length) } };
    mmu.write_phys(address, bytes).is_ok()
}

unsafe extern "C" fn receive(raw: *mut c_void, packet: *mut *const u8, length: *mut usize) -> i32 {
    let context = unsafe { &mut *raw.cast::<PollContext<'_>>() };
    let Some(backend) = context.backend.as_mut() else { return -6 };
    match backend.receive() {
        Ok(Some(bytes)) => {
            context.received = bytes;
            unsafe { *packet = context.received.as_ptr(); *length = context.received.len(); }
            1
        }
        Ok(None) => 0,
        Err(error) => -(error as i32 + 1),
    }
}

unsafe extern "C" fn transmit(raw: *mut c_void, packet: *const u8, length: usize) -> i32 {
    let context = unsafe { &mut *raw.cast::<PollContext<'_>>() };
    let Some(backend) = context.backend.as_mut() else { return 5 };
    let packet = if length == 0 { &[] } else { unsafe { std::slice::from_raw_parts(packet, length) } };
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

pub struct E1000 {
    state: *mut c_void,
    mac: MacAddress,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
    backend: Option<Box<dyn NetBackend>>,
}

impl E1000 {
    pub fn new(mac: MacAddress) -> Self {
        let state = unsafe { ghostos_vm_e1000_new(mac.to_bytes().as_ptr()) };
        assert!(!state.is_null(), "C e1000 allocation failed");
        Self { state, mac, apic: None, irq_vector: 0, backend: None }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) { self.apic = Some(apic); }
    pub fn set_irq_vector(&mut self, vector: u8) { self.irq_vector = vector; }
    pub fn attach_backend(&mut self, backend: Box<dyn NetBackend>) {
        unsafe { ghostos_vm_e1000_clear_rx(self.state) };
        self.backend = Some(backend);
    }
    pub fn mac(&self) -> MacAddress { self.mac }
    pub fn carrier_up(&self) -> bool { self.backend.as_ref().is_none_or(|backend| backend.link_up()) }
    pub fn admin_up(&self) -> bool { self.backend.as_ref().is_none_or(|backend| backend.admin_up()) }
    pub fn queue_state(&self) -> NetQueueState {
        self.backend.as_ref().map_or(NetQueueState::EMPTY, |backend| backend.queue_state())
    }
    pub fn set_admin_up(&mut self, up: bool) {
        if let Some(backend) = &mut self.backend { backend.set_admin_up(up); }
    }
    pub fn take_network_error(&mut self) -> Option<NetError> {
        match unsafe { ghostos_vm_e1000_take_error(self.state) } {
            0 => Some(NetError::PacketTooLarge),
            1 => Some(NetError::Truncated),
            2 => Some(NetError::QueueFull),
            3 => Some(NetError::LinkDown),
            4 => Some(NetError::AdminDown),
            5 => Some(NetError::BackendUnavailable),
            _ => None,
        }
    }
    pub fn transmit_frame(&mut self, packet: &[u8]) -> Result<(), NetError> {
        self.backend.as_mut().ok_or(NetError::BackendUnavailable)?.transmit(packet)
    }
    pub fn receive_frame(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        self.backend.as_mut().ok_or(NetError::BackendUnavailable)?.receive()
    }
    pub fn poll(&mut self, mmu: &mut Mmu) {
        let mut context = PollContext { mmu: Some(mmu), backend: &mut self.backend,
            apic: &self.apic, vector: self.irq_vector, received: Vec::new() };
        let io = context.io();
        assert_eq!(unsafe { ghostos_vm_e1000_poll(self.state, &io, cfg!(debug_assertions)) }, 0,
            "e1000 transmit head overflow");
    }
    fn reset(&mut self) { unsafe { ghostos_vm_e1000_reset(self.state) }; }

    #[cfg(test)]
    fn read_dword(&self, offset: usize) -> u32 {
        Device::read(self, offset as u64, 4).unwrap() as u32
    }

    #[cfg(test)]
    fn write_dword(&mut self, offset: usize, value: u32) {
        Device::write(self, offset as u64, u64::from(value), 4).unwrap()
    }
}

impl Drop for E1000 {
    fn drop(&mut self) { unsafe { ghostos_vm_e1000_free(self.state) }; }
}

impl Device for E1000 {
    fn read(&self, address: u64, size: u8) -> Result<u64, DeviceError> {
        if size != 4 { return Err(DeviceError::UnsupportedSize) }
        let carrier = address & (E1000_MMIO_SIZE - 1) == 8 && self.carrier_up();
        let mut output = 0;
        let code = unsafe { ghostos_vm_e1000_read(self.state, address, size, carrier, &mut output) };
        if code == 0 { Ok(output) } else { Err(DeviceError::UnsupportedSize) }
    }

    fn write(&mut self, address: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 4 { return Err(DeviceError::UnsupportedSize) }
        let mut context = PollContext { mmu: None, backend: &mut self.backend,
            apic: &self.apic, vector: self.irq_vector, received: Vec::new() };
        let io = context.io();
        let code = unsafe { ghostos_vm_e1000_write(self.state, address, size, value as u32, &io) };
        if code == 0 { Ok(()) } else { Err(DeviceError::UnsupportedSize) }
    }

    fn reset(&mut self) { E1000::reset(self); }
}

impl Device for Rc<RefCell<E1000>> {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        Device::read(&*self.borrow(), addr, size)
    }
    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        Device::write(&mut *self.borrow_mut(), addr, value, size)
    }
    fn reset(&mut self) { self.borrow_mut().reset(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{LoopbackHub, LoopbackPort};

    #[test]
    fn registers_mac_link_interrupt_and_reset() {
        let mac = MacAddress::ghostos_default(1);
        let mut nic = E1000::new(mac);
        assert_eq!(Device::read(&nic, REG_STATUS as u64, 4).unwrap() & STATUS_LU as u64, STATUS_LU as u64);
        assert_eq!(Device::read(&nic, REG_RAL0 as u64, 4).unwrap(), 0x1200_5452);
        assert_eq!(Device::read(&nic, REG_RAH0 as u64, 4).unwrap() & 0xFFFF, 0x134);
        assert_eq!(Device::read(&nic, 0, 2), Err(DeviceError::UnsupportedSize));

        Device::write(&mut nic, REG_IMS as u64, ICR_TXDW as u64, 4).unwrap();
        unsafe { ghostos_vm_e1000_signal(nic.state, ICR_TXDW, std::ptr::null()) };
        assert_eq!(nic.read_dword(REG_ICR), ICR_TXDW);
        Device::write(&mut nic, REG_CTRL as u64, CTRL_RST as u64, 4).unwrap();
        assert_eq!(nic.mac(), mac);
        assert_eq!(nic.read_dword(REG_ICR), 0);
    }

    #[test]
    fn tx_and_rx_descriptor_rings_move_a_frame() {
        let hub = Rc::new(RefCell::new(LoopbackHub::new()));
        let tx_mac = MacAddress::ghostos_default(1);
        let rx_mac = MacAddress::ghostos_default(2);
        let mut tx = E1000::new(tx_mac);
        let mut rx = E1000::new(rx_mac);
        tx.attach_backend(Box::new(LoopbackPort::new(hub.clone(), 0, tx_mac)));
        rx.attach_backend(Box::new(LoopbackPort::new(hub, 1, rx_mac)));

        let mut mmu = Mmu::new(0x20_000);
        let frame_addr = 0x3000;
        let tx_ring = 0x1000;
        let rx_ring = 0x2000;
        let mut frame = vec![0u8; 60];
        frame[0..6].copy_from_slice(&rx_mac.0);
        frame[6..12].copy_from_slice(&tx_mac.0);
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        mmu.write_phys(frame_addr, &frame).unwrap();
        let mut tx_desc = [0u8; 16];
        tx_desc[0..8].copy_from_slice(&(frame_addr as u64).to_le_bytes());
        tx_desc[8..12].copy_from_slice(&(frame.len() as u32).to_le_bytes());
        mmu.write_phys(tx_ring, &tx_desc).unwrap();
        tx.write_dword(REG_TDBAL, tx_ring as u32);
        tx.write_dword(REG_TDLEN, 16);
        tx.write_dword(REG_TCTL, TCTL_EN);
        tx.poll(&mut mmu);
        assert_eq!(mmu.read_phys(tx_ring + 12, 1).unwrap()[0] & 3, 3);

        let rx_buffer = 0x4000;
        let mut rx_desc = [0u8; 16];
        rx_desc[0..8].copy_from_slice(&(rx_buffer as u64).to_le_bytes());
        rx_desc[8..12].copy_from_slice(&512u32.to_le_bytes());
        mmu.write_phys(rx_ring + 16, &rx_desc).unwrap();
        rx.write_dword(REG_RDBAL, rx_ring as u32);
        rx.write_dword(REG_RDLEN, 32);
        rx.write_dword(REG_RDT, 0);
        rx.write_dword(REG_RCTL, RCTL_EN);
        rx.poll(&mut mmu);
        assert_eq!(&mmu.read_phys(rx_buffer, frame.len()).unwrap(), &frame);
    }
}
