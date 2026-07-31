//! Intel e1000 NIC emulation (82540EM, PCI ID 8086:100E).
//!
//! Exposes the legacy RX/TX descriptor ring model over a 128 KiB MMIO BAR.
//! Rings are processed in [`E1000::poll`], called from the machine loop
//! outside the CPU step so guest physical memory can be borrowed safely.

use crate::devices::{ApicTrigger, Device, DeviceError, LocalApic};
use crate::memory::Mmu;
use crate::net::{MacAddress, NetBackend, PacketQueue, ETHERNET_FRAME_MAX, ETHERNET_FRAME_MIN};
use std::cell::RefCell;
use std::rc::Rc;

pub const E1000_MMIO_SIZE: u64 = 0x20000;
pub const E1000_VENDOR_ID: u16 = 0x8086;
pub const E1000_DEVICE_ID: u16 = 0x100E;
pub const E1000_CLASS: u8 = 0x02;
pub const E1000_SUBCLASS: u8 = 0x00;
pub const E1000_PROG_IF: u8 = 0x00;

// Register offsets (dword-aligned).
const REG_CTRL: usize = 0x0000;
const REG_STATUS: usize = 0x0008;
const REG_EERD: usize = 0x0014;
const REG_ICR: usize = 0x00C0;
const REG_IMS: usize = 0x00D0;
const REG_IMC: usize = 0x00D8;
const REG_RCTL: usize = 0x0100;
const REG_TCTL: usize = 0x0400;
const REG_RDBAL: usize = 0x2800;
const REG_RDBAH: usize = 0x2804;
const REG_RDLEN: usize = 0x2808;
const REG_RDH: usize = 0x2810;
const REG_RDT: usize = 0x2818;
const REG_TDBAL: usize = 0x3800;
const REG_TDBAH: usize = 0x3804;
const REG_TDLEN: usize = 0x3808;
const REG_TDH: usize = 0x3810;
const REG_TDT: usize = 0x3818;
const REG_RAL0: usize = 0x5400;
const REG_RAH0: usize = 0x5404;

// Control bits.
const CTRL_RST: u32 = 1 << 26;
const CTRL_SLU: u32 = 1 << 6;
const STATUS_LU: u32 = 1 << 1;
const RCTL_EN: u32 = 1 << 0;
const RCTL_BAM: u32 = 1 << 15;
const TCTL_EN: u32 = 1 << 0;

// Interrupt cause bits.
const ICR_TXDW: u32 = 1 << 0;
const ICR_RXDW: u32 = 1 << 6;

// EERD (EEPROM read) bits.
const EERD_START: u32 = 1 << 0;
const EERD_DONE: u32 = 1 << 4;
const EERD_DATA_SHIFT: u32 = 16;

const DESC_SIZE: u64 = 16;
const MAX_RX_QUEUE: usize = 128;

pub struct E1000 {
    mac: MacAddress,
    ctrl: u32,
    ims: u32,
    icr: u32,
    rctl: u32,
    tctl: u32,
    rdbal: u32,
    rdbah: u32,
    rdlen: u32,
    rdh: u32,
    rdt: u32,
    tdbal: u32,
    tdbah: u32,
    tdlen: u32,
    tdh: u32,
    tdt: u32,
    mta: [u32; 128],
    pending_rx: PacketQueue,
    promiscuous: bool,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
    backend: Option<Box<dyn NetBackend>>,
}

impl E1000 {
    pub fn new(mac: MacAddress) -> Self {
        Self {
            mac,
            ctrl: 0,
            ims: 0,
            icr: 0,
            rctl: 0,
            tctl: 0,
            rdbal: 0,
            rdbah: 0,
            rdlen: 0,
            rdh: 0,
            rdt: 0,
            tdbal: 0,
            tdbah: 0,
            tdlen: 0,
            tdh: 0,
            tdt: 0,
            mta: [0; 128],
            pending_rx: PacketQueue::new(MAX_RX_QUEUE, ETHERNET_FRAME_MAX * 2),
            promiscuous: false,
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

    fn link_up(&self) -> bool {
        self.backend.as_ref().map(|b| b.link_up()).unwrap_or(true)
    }

    fn promiscuous(&self) -> bool {
        self.promiscuous
    }

    fn register_offset(&self, addr: u64) -> usize {
        ((addr & (E1000_MMIO_SIZE - 1)) >> 2) as usize
    }

    fn read_dword(&self, off: usize) -> u32 {
        match off {
            REG_CTRL => self.ctrl,
            REG_STATUS => (self.status() & !STATUS_LU) | if self.link_up() { STATUS_LU } else { 0 },
            REG_EERD => 0x0000_0020, // done bit set for fast MAC read
            REG_ICR => self.icr,
            REG_IMS => self.ims,
            REG_RCTL => self.rctl,
            REG_TCTL => self.tctl,
            REG_RDBAL => self.rdbal,
            REG_RDBAH => self.rdbah,
            REG_RDLEN => self.rdlen,
            REG_RDH => self.rdh,
            REG_RDT => self.rdt,
            REG_TDBAL => self.tdbal,
            REG_TDBAH => self.tdbah,
            REG_TDLEN => self.tdlen,
            REG_TDH => self.tdh,
            REG_TDT => self.tdt,
            REG_RAL0 => u32::from_le_bytes([self.mac.0[0], self.mac.0[1], self.mac.0[2], self.mac.0[3]]),
            REG_RAH0 => {
                let mut v = u32::from_le_bytes([self.mac.0[4], self.mac.0[5], 0, 0]);
                v |= 0x8000_0000; // Address Valid
                v
            }
            _ => 0,
        }
    }

    fn status(&self) -> u32 {
        0x8000_0000 | 0x20 // full duplex + tx off
    }

    fn eeprom_word(&self, addr: u32) -> u32 {
        match addr {
            0 => u32::from_le_bytes([self.mac.0[0], self.mac.0[1], self.mac.0[2], self.mac.0[3]]),
            1 => u32::from_le_bytes([self.mac.0[4], self.mac.0[5], 0, 0]),
            _ => 0,
        }
    }

    fn write_dword(&mut self, off: usize, value: u32) {
        match off {
            REG_CTRL => {
                if value & CTRL_RST != 0 {
                    self.reset();
                } else {
                    let slu = value & CTRL_SLU;
                    let mask = CTRL_SLU;
                    self.ctrl = (self.ctrl & !mask) | slu;
                }
            }
            REG_ICR => self.icr &= !value,
            REG_IMS => {
                self.ims |= value;
                self.check_interrupt();
            }
            REG_IMC => {
                self.ims &= !value;
                self.check_interrupt();
            }
            REG_EERD => {
                if value & EERD_START != 0 {
                    // Synchronous read completes immediately.
                }
            }
            REG_RCTL => {
                let was = self.rctl & RCTL_EN;
                let now = value & RCTL_EN;
                self.rctl = value;
                if was == 0 && now != 0 {
                    self.pending_rx.clear();
                    self.rdh = 0;
                }
            }
            REG_TCTL => self.tctl = value,
            REG_RDBAL => self.rdbal = value,
            REG_RDBAH => self.rdbah = value,
            REG_RDLEN => self.rdlen = value,
            REG_RDH => self.rdh = value,
            REG_RDT => self.rdt = value,
            REG_TDBAL => self.tdbal = value,
            REG_TDBAH => self.tdbah = value,
            REG_TDLEN => self.tdlen = value,
            REG_TDH => self.tdh = value,
            REG_TDT => self.tdt = value,
            _ => {}
        }
    }

    fn check_interrupt(&mut self) {
        if self.icr & self.ims != 0 && self.irq_vector != 0 {
            if let Some(apic) = &self.apic {
                apic.borrow_mut().signal(self.irq_vector, ApicTrigger::Edge);
            }
        }
    }

    pub fn poll(&mut self, mmu: &mut Mmu) {
        if self.tctl & TCTL_EN != 0 {
            self.poll_tx(mmu);
        }
        if self.rctl & RCTL_EN != 0 {
            if let Some(backend) = &mut self.backend {
                while let Ok(Some(packet)) = backend.receive() {
                    if !self.pending_rx.push(packet) {
                        break;
                    }
                }
            }
            self.poll_rx(mmu);
        }
    }

    fn poll_tx(&mut self, mmu: &mut Mmu) {
        let count = self.ring_len(self.tdlen);
        if count == 0 {
            return;
        }
        let base = self.base_addr(self.tdbal, self.tdbah);
        let mut sent = 0usize;
        while sent < count {
            let idx = self.tdh as usize % count;
            let mut desc = [0u8; 16];
            if !Self::dma_read(mmu, base + (idx as u64) * DESC_SIZE, &mut desc) {
                break;
            }
            let addr = u64::from_le_bytes(desc[0..8].try_into().unwrap());
            let cmd_len = u32::from_le_bytes(desc[8..12].try_into().unwrap());
            let length = (cmd_len & 0x0FFF) as usize;
            if length == 0 {
                break;
            }
            let mut packet = vec![0u8; length];
            if !Self::dma_read(mmu, addr, &mut packet) {
                break;
            }
            if let Some(backend) = &mut self.backend {
                let _ = backend.transmit(&packet);
            }
            let mut status = desc[12];
            status |= 0x03; // DD | EOP
            let _ = Self::dma_write(mmu, base + (idx as u64) * DESC_SIZE + 12, &[status]);
            self.tdh = (self.tdh + 1) % count as u32;
            sent += 1;
        }
        if sent > 0 {
            self.icr |= ICR_TXDW;
            self.check_interrupt();
        }
    }

    fn base_addr(&self, low: u32, high: u32) -> u64 {
        (low as u64) | ((high as u64) << 32)
    }

    fn ring_len(&self, len_reg: u32) -> usize {
        (len_reg as usize / DESC_SIZE as usize).min(4096)
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

    fn poll_rx(&mut self, mmu: &mut Mmu) {
        let count = self.ring_len(self.rdlen);
        if count == 0 {
            return;
        }
        let base = self.base_addr(self.rdbal, self.rdbah);
        let mut delivered = 0usize;
        while let Some(packet) = self.pending_rx.peek() {
            let idx = (self.rdt as usize + 1) % count;
            let mut desc = [0u8; 16];
            if !Self::dma_read(mmu, base + (idx as u64) * DESC_SIZE, &mut desc) {
                break;
            }
            let buf_addr = u64::from_le_bytes(desc[0..8].try_into().unwrap());
            let buf_len = (u32::from_le_bytes(desc[8..12].try_into().unwrap()) & 0xFFFF) as usize;
            let frame_len = packet.len();
            if frame_len > buf_len {
                break;
            }
            if !Self::dma_write(mmu, buf_addr, packet) {
                break;
            }
            let dst = &packet[..6];
            let mut status = 0x03u8; // DD | EOP
            if dst == [0xFF; 6] {
                status |= 0x04; // PIF (broadcast)
            }
            let mut l = (frame_len as u32).to_le_bytes();
            l[3] = status;
            let _ = Self::dma_write(mmu, base + (idx as u64) * DESC_SIZE + 8, &l);
            let _ = self.pending_rx.pop();
            self.rdt = idx as u32;
            delivered += 1;
        }
        if delivered > 0 {
            self.icr |= ICR_RXDW;
            self.check_interrupt();
        }
    }

    fn reset(&mut self) {
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

impl Device for E1000 {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        if size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        Ok(self.read_dword(self.register_offset(addr)) as u64)
    }

    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        self.write_dword(self.register_offset(addr), value as u32);
        Ok(())
    }

    fn reset(&mut self) {
        E1000::reset(self);
    }
}

impl Device for Rc<RefCell<E1000>> {
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
