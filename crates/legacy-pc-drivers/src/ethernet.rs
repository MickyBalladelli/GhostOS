use core::ptr::{NonNull, read_volatile, write_volatile};
use ghostos_status::{IntoStatus, Severity, Status, facility};

use crate::pci::{Bar, PciDevice};

const INTEL_VENDOR: u16 = 0x8086;
const VIRTIO_VENDOR: u16 = 0x1af4;
const REALTEK_VENDOR: u16 = 0x10ec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EthernetKind {
    IntelE1000,
    VirtioNet,
    RealtekRtl8169,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EthernetAdapter {
    pub kind: EthernetKind,
    pub registers: Bar,
    pub interrupt_line: u8,
}

impl EthernetAdapter {
    pub fn from_pci(device: &PciDevice) -> Option<Self> {
        if !device.is_ethernet() {
            return None
        }

        let kind = match (device.vendor_id, device.device_id) {
            (
                INTEL_VENDOR,
                0x100e
                | 0x100f
                | 0x1010
                | 0x107c
                | 0x10d3
                | 0x150c
                | 0x1533
                | 0x1539
                | 0x157b,
            ) => EthernetKind::IntelE1000,
            (VIRTIO_VENDOR, 0x1000) => EthernetKind::VirtioNet,
            (REALTEK_VENDOR, 0x8161 | 0x8168 | 0x8169) => {
                EthernetKind::RealtekRtl8169
            }
            _ => return None,
        };
        let registers = match kind {
            EthernetKind::VirtioNet => device
                .bars
                .iter()
                .copied()
                .find(|bar| bar.io_port().is_some())?,
            _ => device
                .bars
                .iter()
                .copied()
                .find(|bar| bar.memory_address().is_some())?,
        };
        Some(Self {
            kind,
            registers,
            interrupt_line: device.interrupt_line,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EthernetError {
    InvalidRegisterBase,
    InvalidRing,
    TimedOut,
    UnsupportedDevice,
}

impl IntoStatus for EthernetError {
    fn status(self) -> Status {
        let code = match self {
            Self::InvalidRegisterBase => 1,
            Self::InvalidRing => 2,
            Self::TimedOut => 3,
            Self::UnsupportedDevice => 4,
        };
        Status::new(Severity::Error, facility::DRIVER, code, 0)
            .unwrap_or(Status::INVALID_ARGUMENT)
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct IntelRxDescriptor {
    pub buffer_address: u64,
    pub length: u16,
    pub checksum: u16,
    pub status: u8,
    pub errors: u8,
    pub special: u16,
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct IntelTxDescriptor {
    pub buffer_address: u64,
    pub length: u16,
    pub checksum_offset: u8,
    pub command: u8,
    pub status: u8,
    pub checksum_start: u8,
    pub special: u16,
}

pub struct IntelE1000 {
    registers: NonNull<u8>,
}

impl IntelE1000 {
    const CTRL: usize = 0x0000;
    const STATUS: usize = 0x0008;
    const EERD: usize = 0x0014;
    const ICR: usize = 0x00c0;
    const IMS: usize = 0x00d0;
    const RCTL: usize = 0x0100;
    const TCTL: usize = 0x0400;
    const RDBAL: usize = 0x2800;
    const RDBAH: usize = 0x2804;
    const RDLEN: usize = 0x2808;
    const RDH: usize = 0x2810;
    const RDT: usize = 0x2818;
    const TDBAL: usize = 0x3800;
    const TDBAH: usize = 0x3804;
    const TDLEN: usize = 0x3808;
    const TDH: usize = 0x3810;
    const TDT: usize = 0x3818;
    const RAL: usize = 0x5400;
    const RAH: usize = 0x5404;

    /// # Safety
    /// `registers` must be an exclusive writable mapping of an Intel E1000
    /// compatible MMIO BAR.
    pub unsafe fn new(registers: usize) -> Result<Self, EthernetError> {
        Ok(Self {
            registers: NonNull::new(registers as *mut u8)
                .ok_or(EthernetError::InvalidRegisterBase)?,
        })
    }

    pub fn reset(&mut self, mut spin_limit: usize) -> Result<(), EthernetError> {
        self.write(Self::CTRL, self.read(Self::CTRL) | 1 << 26);
        while self.read(Self::CTRL) & (1 << 26) != 0 {
            if spin_limit == 0 {
                return Err(EthernetError::TimedOut)
            }
            spin_limit -= 1;
            core::hint::spin_loop()
        }
        self.write(Self::ICR, u32::MAX);
        Ok(())
    }

    pub fn read_mac(&mut self, spin_limit: usize) -> Result<[u8; 6], EthernetError> {
        let low = self.read(Self::RAL);
        let high = self.read(Self::RAH);
        if high & (1 << 31) != 0 {
            return Ok([
                low as u8,
                (low >> 8) as u8,
                (low >> 16) as u8,
                (low >> 24) as u8,
                high as u8,
                (high >> 8) as u8,
            ])
        }

        let mut mac = [0; 6];
        for word in 0..3 {
            self.write(Self::EERD, 1 | (word as u32) << 8);
            let mut spins = spin_limit;
            let value = loop {
                let value = self.read(Self::EERD);
                if value & (1 << 4) != 0 {
                    break (value >> 16) as u16
                }
                if spins == 0 {
                    return Err(EthernetError::TimedOut)
                }
                spins -= 1;
                core::hint::spin_loop()
            };
            mac[word * 2] = value as u8;
            mac[word * 2 + 1] = (value >> 8) as u8;
        }
        Ok(mac)
    }

    pub fn link_up(&self) -> bool {
        self.read(Self::STATUS) & (1 << 1) != 0
    }

    pub fn set_admin_up(&mut self, enabled: bool) {
        if enabled {
            if self.read(Self::RDLEN) != 0 {
                self.write(Self::RCTL, self.read(Self::RCTL) | 1);
            }
            if self.read(Self::TDLEN) != 0 {
                self.write(Self::TCTL, self.read(Self::TCTL) | 1);
            }
        } else {
            self.write(Self::RCTL, self.read(Self::RCTL) & !1);
            self.write(Self::TCTL, self.read(Self::TCTL) & !1);
        }
    }

    pub fn queue_snapshot(&self, receive: bool) -> EthernetQueueSnapshot {
        let (length, head, tail, enabled) = if receive {
            (
                self.read(Self::RDLEN),
                self.read(Self::RDH),
                self.read(Self::RDT),
                self.read(Self::RCTL) & 1 != 0,
            )
        } else {
            (
                self.read(Self::TDLEN),
                self.read(Self::TDH),
                self.read(Self::TDT),
                self.read(Self::TCTL) & 1 != 0,
            )
        };
        EthernetQueueSnapshot {
            ready: enabled && length != 0,
            head: Some(head),
            tail: Some(tail),
            capacity: length / size_of::<IntelRxDescriptor>() as u32,
        }
    }

    pub fn queues_ready(&self) -> bool {
        self.queue_snapshot(true).ready && self.queue_snapshot(false).ready
    }

    pub fn configure_rings(
        &mut self,
        rx_physical: u64,
        rx_descriptors: u16,
        tx_physical: u64,
        tx_descriptors: u16,
    ) -> Result<(), EthernetError> {
        let rx_bytes = rx_descriptors as u32 * size_of::<IntelRxDescriptor>() as u32;
        let tx_bytes = tx_descriptors as u32 * size_of::<IntelTxDescriptor>() as u32;
        if rx_descriptors < 8
            || tx_descriptors < 8
            || rx_bytes % 128 != 0
            || tx_bytes % 128 != 0
        {
            return Err(EthernetError::InvalidRing)
        }

        self.write(Self::RDBAL, rx_physical as u32);
        self.write(Self::RDBAH, (rx_physical >> 32) as u32);
        self.write(Self::RDLEN, rx_bytes);
        self.write(Self::RDH, 0);
        self.write(Self::RDT, rx_descriptors as u32 - 1);
        self.write(Self::TDBAL, tx_physical as u32);
        self.write(Self::TDBAH, (tx_physical >> 32) as u32);
        self.write(Self::TDLEN, tx_bytes);
        self.write(Self::TDH, 0);
        self.write(Self::TDT, 0);
        self.write(Self::RCTL, 1 << 1 | 1 << 15 | 1 << 26);
        self.write(Self::TCTL, 1 << 1 | 1 << 3 | 15 << 4 | 64 << 12);
        self.write(Self::IMS, 1 << 7 | 1 << 4 | 1 << 2);
        Ok(())
    }

    pub fn notify_transmit(&mut self, tail: u16) {
        self.write(Self::TDT, tail as u32)
    }

    pub fn notify_receive(&mut self, tail: u16) {
        self.write(Self::RDT, tail as u32)
    }

    fn read(&self, offset: usize) -> u32 {
        unsafe { read_volatile(self.registers.as_ptr().add(offset).cast()) }
    }

    fn write(&mut self, offset: usize, value: u32) {
        unsafe {
            write_volatile(self.registers.as_ptr().add(offset).cast(), value)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EthernetSnapshot {
    pub mac: [u8; 6],
    pub link_up: bool,
    pub admin_up: bool,
    pub rx_queue: EthernetQueueSnapshot,
    pub tx_queue: EthernetQueueSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EthernetQueueSnapshot {
    pub ready: bool,
    pub head: Option<u32>,
    pub tail: Option<u32>,
    pub capacity: u32,
}

pub struct EthernetRuntime {
    kind: EthernetKind,
    e1000: Option<IntelE1000>,
    virtio_port: Option<u16>,
    mac: [u8; 6],
    admin_up: bool,
}

impl EthernetRuntime {
    /// Opens a supported PCI NIC without allocating memory or taking
    /// ownership of its DMA rings. `physical_address_offset` is the boot-time
    /// mapping offset used for MMIO BARs.
    pub fn open(
        adapter: EthernetAdapter,
        physical_address_offset: u64,
    ) -> Result<Self, EthernetError> {
        match adapter.kind {
            EthernetKind::IntelE1000 => {
                let physical = adapter
                    .registers
                    .memory_address()
                    .ok_or(EthernetError::InvalidRegisterBase)?;
                let virtual_address = physical
                    .checked_add(physical_address_offset)
                    .ok_or(EthernetError::InvalidRegisterBase)?;
                let mut e1000 = unsafe { IntelE1000::new(virtual_address as usize)? };
                e1000.set_admin_up(false);
                let mac = e1000.read_mac(100_000)?;
                Ok(Self {
                    kind: adapter.kind,
                    e1000: Some(e1000),
                    virtio_port: None,
                    mac,
                    admin_up: false,
                })
            }
            EthernetKind::VirtioNet => {
                let port = adapter
                    .registers
                    .io_port()
                    .ok_or(EthernetError::InvalidRegisterBase)? as u16;
                let mut mac = [0; 6];
                for (index, byte) in mac.iter_mut().enumerate() {
                    *byte = unsafe { in_u8(port + 0x14 + index as u16) };
                }
                Ok(Self {
                    kind: adapter.kind,
                    e1000: None,
                    virtio_port: Some(port),
                    mac,
                    admin_up: true,
                })
            }
            EthernetKind::RealtekRtl8169 => Err(EthernetError::UnsupportedDevice),
        }
    }

    pub fn kind(&self) -> EthernetKind {
        self.kind
    }

    pub fn snapshot(&mut self) -> EthernetSnapshot {
        if let Some(e1000) = &mut self.e1000 {
            return EthernetSnapshot {
                mac: self.mac,
                link_up: e1000.link_up(),
                admin_up: self.admin_up,
                rx_queue: e1000.queue_snapshot(true),
                tx_queue: e1000.queue_snapshot(false),
            }
        }

        let status = self.virtio_port.map_or(0, |port| unsafe { in_u8(port + 0x12) });
        let link = self
            .virtio_port
            .map(|port| unsafe { in_u8(port + 0x1a) & 1 != 0 })
            .unwrap_or(false);
        let (rx_queue, tx_queue) = self
            .virtio_port
            .map(|port| {
                let rx = Self::virtio_queue_snapshot(port, 0, status & 4 != 0);
                let tx = Self::virtio_queue_snapshot(port, 1, status & 4 != 0);
                (rx, tx)
            })
            .unwrap_or((EthernetQueueSnapshot::EMPTY, EthernetQueueSnapshot::EMPTY));
        EthernetSnapshot {
            mac: self.mac,
            link_up: link,
            admin_up: self.admin_up,
            rx_queue,
            tx_queue,
        }
    }

    fn virtio_queue_snapshot(
        port: u16,
        queue: u16,
        driver_ready: bool,
    ) -> EthernetQueueSnapshot {
        unsafe { out_u16(port + 0x0e, queue) };
        let page = unsafe { in_u32(port + 0x08) };
        let capacity = unsafe { in_u16(port + 0x0c) } as u32;
        EthernetQueueSnapshot {
            ready: driver_ready && page != 0,
            head: None,
            tail: None,
            capacity,
        }
    }

    pub fn set_admin_up(&mut self, enabled: bool) {
        self.admin_up = enabled;
        if let Some(e1000) = &mut self.e1000 {
            e1000.set_admin_up(enabled)
        }
    }
}

impl EthernetQueueSnapshot {
    pub const EMPTY: Self = Self {
        ready: false,
        head: None,
        tail: None,
        capacity: 0,
    };
}

#[cfg(target_arch = "x86_64")]
unsafe fn in_u8(port: u16) -> u8 {
    let value;
    unsafe {
        core::arch::asm!(
            "in al, dx",
            in("dx") port,
            out("al") value,
            options(nomem, nostack, preserves_flags),
        )
    }
    value
}

#[cfg(target_arch = "x86_64")]
unsafe fn in_u16(port: u16) -> u16 {
    let value;
    unsafe {
        core::arch::asm!(
            "in ax, dx",
            in("dx") port,
            out("ax") value,
            options(nomem, nostack, preserves_flags),
        )
    }
    value
}

#[cfg(target_arch = "x86_64")]
unsafe fn in_u32(port: u16) -> u32 {
    let value;
    unsafe {
        core::arch::asm!(
            "in eax, dx",
            in("dx") port,
            out("eax") value,
            options(nomem, nostack, preserves_flags),
        )
    }
    value
}

#[cfg(target_arch = "x86_64")]
unsafe fn out_u16(port: u16, value: u16) {
    unsafe {
        core::arch::asm!(
            "out dx, ax",
            in("dx") port,
            in("ax") value,
            options(nomem, nostack, preserves_flags),
        )
    }
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn in_u8(_port: u16) -> u8 {
    0
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn in_u16(_port: u16) -> u16 {
    0
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn in_u32(_port: u16) -> u32 {
    0
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn out_u16(_port: u16, _value: u16) {}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct RealtekDescriptor {
    pub command: u32,
    pub vlan: u32,
    pub buffer_address: u64,
}

pub struct RealtekRtl8169 {
    registers: NonNull<u8>,
}

impl RealtekRtl8169 {
    const TX_POLL: usize = 0x38;
    const COMMAND: usize = 0x37;
    const INTERRUPT_MASK: usize = 0x3c;
    const INTERRUPT_STATUS: usize = 0x3e;
    const TX_CONFIG: usize = 0x40;
    const RX_CONFIG: usize = 0x44;
    const TX_DESCRIPTOR_LOW: usize = 0x20;
    const TX_DESCRIPTOR_HIGH: usize = 0x24;
    const RX_DESCRIPTOR_LOW: usize = 0xe4;
    const RX_DESCRIPTOR_HIGH: usize = 0xe8;

    /// # Safety
    /// `registers` must be an exclusive writable mapping of an RTL8169-family
    /// MMIO BAR.
    pub unsafe fn new(registers: usize) -> Result<Self, EthernetError> {
        Ok(Self {
            registers: NonNull::new(registers as *mut u8)
                .ok_or(EthernetError::InvalidRegisterBase)?,
        })
    }

    pub fn reset(&mut self, mut spin_limit: usize) -> Result<(), EthernetError> {
        self.write_u8(Self::COMMAND, 1 << 4);
        while self.read_u8(Self::COMMAND) & (1 << 4) != 0 {
            if spin_limit == 0 {
                return Err(EthernetError::TimedOut)
            }
            spin_limit -= 1;
            core::hint::spin_loop()
        }
        self.write_u16(Self::INTERRUPT_STATUS, u16::MAX);
        Ok(())
    }

    pub fn read_mac(&self) -> [u8; 6] {
        let mut mac = [0; 6];
        for (index, byte) in mac.iter_mut().enumerate() {
            *byte = self.read_u8(index)
        }
        mac
    }

    pub fn configure_rings(
        &mut self,
        rx_physical: u64,
        tx_physical: u64,
    ) -> Result<(), EthernetError> {
        if rx_physical & 0xff != 0 || tx_physical & 0xff != 0 {
            return Err(EthernetError::InvalidRing)
        }

        self.write_u32(Self::TX_DESCRIPTOR_LOW, tx_physical as u32);
        self.write_u32(Self::TX_DESCRIPTOR_HIGH, (tx_physical >> 32) as u32);
        self.write_u32(Self::RX_DESCRIPTOR_LOW, rx_physical as u32);
        self.write_u32(Self::RX_DESCRIPTOR_HIGH, (rx_physical >> 32) as u32);
        self.write_u32(Self::TX_CONFIG, 7 << 8);
        self.write_u32(Self::RX_CONFIG, 7 << 8 | 1 << 7 | 1 << 3);
        self.write_u16(Self::INTERRUPT_MASK, 1 << 4 | 1 << 2 | 1);
        self.write_u8(Self::COMMAND, 1 << 3 | 1 << 2);
        Ok(())
    }

    pub fn notify_transmit(&mut self) {
        self.write_u8(Self::TX_POLL, 1 << 6)
    }

    fn read_u8(&self, offset: usize) -> u8 {
        unsafe { read_volatile(self.registers.as_ptr().add(offset)) }
    }

    fn write_u8(&mut self, offset: usize, value: u8) {
        unsafe { write_volatile(self.registers.as_ptr().add(offset), value) }
    }

    fn write_u16(&mut self, offset: usize, value: u16) {
        unsafe {
            write_volatile(self.registers.as_ptr().add(offset).cast(), value)
        }
    }

    fn write_u32(&mut self, offset: usize, value: u32) {
        unsafe {
            write_volatile(self.registers.as_ptr().add(offset).cast(), value)
        }
    }
}
