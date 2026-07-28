use core::ptr::{NonNull, read_volatile, write_volatile};
use synos_status::{IntoStatus, Severity, Status, facility};

use crate::pci::{Bar, PciDevice};

const INTEL_VENDOR: u16 = 0x8086;
const REALTEK_VENDOR: u16 = 0x10ec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EthernetKind {
    IntelE1000,
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
            (REALTEK_VENDOR, 0x8161 | 0x8168 | 0x8169) => {
                EthernetKind::RealtekRtl8169
            }
            _ => return None,
        };
        let registers = device
            .bars
            .iter()
            .copied()
            .find(|bar| bar.memory_address().is_some())?;
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
}

impl IntoStatus for EthernetError {
    fn status(self) -> Status {
        let code = match self {
            Self::InvalidRegisterBase => 1,
            Self::InvalidRing => 2,
            Self::TimedOut => 3,
        };
        Status::new(Severity::Error, facility::DRIVER, code, 0)
            .expect("valid Ethernet status")
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
