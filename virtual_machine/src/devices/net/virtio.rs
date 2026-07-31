//! Legacy virtio-net device (single RX/TX virtqueue pair, 0.9.5 layout).

use crate::devices::{ApicTrigger, Device, DeviceError, LocalApic, PortDevice};
use crate::memory::Mmu;
use crate::net::{MacAddress, NetBackend, PacketQueue, ETHERNET_FRAME_MAX};
use std::cell::RefCell;
use std::rc::Rc;

pub const VIRTIO_NET_VENDOR_ID: u16 = 0x1AF4;
pub const VIRTIO_NET_DEVICE_ID: u16 = 0x1000;
pub const VIRTIO_NET_CLASS: u8 = 0x02;
pub const VIRTIO_NET_SUBCLASS: u8 = 0x00;
pub const VIRTIO_NET_PROG_IF: u8 = 0x00;
pub const VIRTIO_NET_PCI_BAR0_SIZE: u64 = 0x100;

const REG_DEVICE_FEATURES: u16 = 0x00;
const REG_GUEST_FEATURES: u16 = 0x04;
const REG_QUEUE_PFN: u16 = 0x08;
const REG_QUEUE_SIZE: u16 = 0x0C;
const REG_QUEUE_SEL: u16 = 0x0E;
const REG_QUEUE_NOTIFY: u16 = 0x10;
const REG_STATUS: u16 = 0x12;

const DEVICE_FEATURES: u32 = (1 << 5) | (1 << 16); // VIRTIO_NET_F_MAC | STATUS

const QUEUE_RX: u16 = 0;
const QUEUE_TX: u16 = 1;
const QUEUE_SIZE: u16 = 128;
const DESC_SIZE: u64 = 16;
const HEADER_LEN: usize = 10; // virtio-net header written before each frame
const MAX_RX_BACKLOG: usize = 128;

pub struct VirtioNet {
    mac: MacAddress,
    guest_features: u32,
    queue_sel: u16,
    queue_pfn: u32,
    status: u8,
    queue_enabled: [bool; 2],
    avail_last: [u16; 2],
    used_count: [u16; 2],
    pending_rx: PacketQueue,
    poll_pending: bool,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
    backend: Option<Box<dyn NetBackend>>,
}

impl VirtioNet {
    pub fn new(mac: MacAddress) -> Self {
        Self {
            mac,
            guest_features: 0,
            queue_sel: 0,
            queue_pfn: 0,
            status: 0,
            queue_enabled: [false; 2],
            avail_last: [0; 2],
            used_count: [0; 2],
            pending_rx: PacketQueue::new(MAX_RX_BACKLOG, ETHERNET_FRAME_MAX * 2),
            poll_pending: false,
            apic: None,
            irq_vector: 0,
            backend: None,
        }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic);
    }

    pub fn set_irq_vector(&mut self, vector: u8) {
        self.irq_vector = vector;
    }

    pub fn attach_backend(&mut self, backend: Box<dyn NetBackend>) {
        self.pending_rx.clear();
        self.backend = Some(backend);
    }

    pub fn mac(&self) -> MacAddress {
        self.mac
    }

    pub fn has_pending(&self) -> bool {
        self.poll_pending || !self.pending_rx.is_empty()
    }

    fn desc_base(&self, q: u16) -> u64 {
        (self.queue_pfn as u64) << 12
    }

    fn avail_base(&self, q: u16) -> u64 {
        self.desc_base(q) + QUEUE_SIZE as u64 * DESC_SIZE
    }

    fn used_base(&self, q: u16) -> u64 {
        self.avail_base(q) + 6 + (2 + QUEUE_SIZE as u64) * 2
    }

    fn read_io(&self, port: u16) -> u32 {
        let off = (port & (VIRTIO_NET_PCI_BAR0_SIZE as u16 - 1)) as u16;
        match off {
            REG_DEVICE_FEATURES => DEVICE_FEATURES,
            REG_GUEST_FEATURES => self.guest_features,
            REG_QUEUE_PFN => self.queue_pfn,
            REG_QUEUE_SIZE => QUEUE_SIZE as u32,
            REG_QUEUE_SEL => self.queue_sel as u32,
            REG_STATUS => self.status as u32,
            _ => 0,
        }
    }

    fn write_io(&mut self, port: u16, value: u32) {
        let off = (port & (VIRTIO_NET_PCI_BAR0_SIZE as u16 - 1)) as u16;
        match off {
            REG_GUEST_FEATURES => self.guest_features = value,
            REG_QUEUE_PFN => {
                self.queue_pfn = value;
                let q = self.queue_sel as usize;
                self.queue_enabled[q] = value != 0;
                self.avail_last[q] = 0;
                self.used_count[q] = 0;
            }
            REG_QUEUE_SEL => self.queue_sel = (value & 1) as u16,
            REG_QUEUE_NOTIFY => {
                self.poll_pending = true;
            }
            REG_STATUS => self.status = value as u8,
            _ => {}
        }
    }

    pub fn on_notify(&mut self, _q: u16) {
        self.poll_pending = true;
    }

    pub fn poll(&mut self, mmu: &mut Mmu) {
        if let Some(backend) = &mut self.backend {
            while let Ok(Some(packet)) = backend.receive() {
                if !self.pending_rx.push(packet) {
                    break;
                }
            }
        }
        if self.poll_pending || !self.pending_rx.is_empty() {
            self.poll_tx(mmu);
            self.poll_rx(mmu);
        }
        self.poll_pending = false;
    }

    fn dma_read(mmu: &Mmu, addr: u64, buf: &mut [u8]) -> bool {
        match mmu.read_phys(addr, buf.len()) {
            Ok(bytes) => {
                buf.copy_from_slice(&bytes);
                true
            }
            Err(_) => false,
        }
    }

    fn dma_write(mmu: &mut Mmu, addr: u64, buf: &[u8]) -> bool {
        mmu.write_phys(addr, buf).is_ok()
    }

    fn update_avail(&mut self, mmu: &Mmu, q: u16) {
        let mut buf = [0u8; 2];
        if Self::dma_read(mmu, self.avail_base(q) + 2, &mut buf) {
            self.avail_last[q as usize] = u16::from_le_bytes(buf);
        }
    }

    fn poll_tx(&mut self, mmu: &mut Mmu) {
        let q = QUEUE_TX as usize;
        if !self.queue_enabled[q] {
            return;
        }
        self.update_avail(mmu, QUEUE_TX);
        let base = self.desc_base(QUEUE_TX);
        while self.avail_last[q] != self.used_count[q] {
            let idx = self.used_count[q] as u64 & (QUEUE_SIZE as u64 - 1);
            let mut desc = [0u8; 16];
            if !Self::dma_read(mmu, base + idx * DESC_SIZE, &mut desc) {
                break;
            }
            let addr = u64::from_le_bytes(desc[0..8].try_into().unwrap());
            let len = u32::from_le_bytes(desc[8..12].try_into().unwrap()) as usize;
            if len == 0 {
                break;
            }
            let mut packet = vec![0u8; len];
            if !Self::dma_read(mmu, addr, &mut packet) {
                break;
            }
            if let Some(backend) = &mut self.backend {
                let _ = backend.transmit(&packet);
            }
            self.used_count[q] = self.used_count[q].wrapping_add(1);
        }
        self.check_interrupt();
    }

    fn poll_rx(&mut self, mmu: &mut Mmu) {
        let q = QUEUE_RX as usize;
        if !self.queue_enabled[q] || self.pending_rx.is_empty() {
            return;
        }
        self.update_avail(mmu, QUEUE_RX);
        let base = self.desc_base(QUEUE_RX);
        let used = self.used_base(QUEUE_RX);
        let mut delivered = 0u16;
        while self.avail_last[q] != self.used_count[q] {
            if self.pending_rx.is_empty() {
                break;
            }
            let idx = self.used_count[q] as u64 & (QUEUE_SIZE as u64 - 1);
            let mut desc = [0u8; 16];
            if !Self::dma_read(mmu, base + idx * DESC_SIZE, &mut desc) {
                break;
            }
            let addr = u64::from_le_bytes(desc[0..8].try_into().unwrap());
            let len = u32::from_le_bytes(desc[8..12].try_into().unwrap()) as usize;
            let Some(packet) = self.pending_rx.pop() else { break };
            if packet.len() + HEADER_LEN > len {
                break;
            }
            let mut frame = vec![0u8; HEADER_LEN + packet.len()];
            frame[HEADER_LEN..].copy_from_slice(&packet);
            if !Self::dma_write(mmu, addr, &frame) {
                break;
            }
            let mut entry = [0u8; 8];
            entry[0..2].copy_from_slice(&(idx as u16).to_le_bytes());
            entry[4..8].copy_from_slice(&(frame.len() as u32).to_le_bytes());
            let used_off = used + (self.used_count[q] as u64 & (QUEUE_SIZE as u64 - 1)) * 8;
            if !Self::dma_write(mmu, used_off, &entry) {
                break;
            }
            self.used_count[q] = self.used_count[q].wrapping_add(1);
            delivered += 1;
        }
        if delivered > 0 {
            let mut idx_buf = (self.used_count[q] as u32).to_le_bytes();
            let _ = Self::dma_write(mmu, used + 2, &idx_buf[..2]);
            self.check_interrupt();
        }
    }

    fn check_interrupt(&mut self) {
        if self.irq_vector != 0 {
            if let Some(apic) = &self.apic {
                apic.borrow_mut().signal(self.irq_vector, ApicTrigger::Edge);
            }
        }
    }

    pub fn reset(&mut self) {
        let apic = self.apic.take();
        let v = self.irq_vector;
        let backend = self.backend.take();
        let mac = self.mac;
        *self = Self::new(mac);
        self.apic = apic;
        self.irq_vector = v;
        if let Some(b) = backend {
            self.attach_backend(b);
        }
    }
}

impl PortDevice for VirtioNet {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        if size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        Ok(self.read_io(port) as u64)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 4 {
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
