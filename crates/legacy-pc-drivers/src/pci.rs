#[cfg(target_arch = "x86_64")]
use core::arch::asm;

pub const INVALID_VENDOR: u16 = 0xffff;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciAddress {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl PciAddress {
    pub const fn new(bus: u8, device: u8, function: u8) -> Option<Self> {
        if device < 32 && function < 8 {
            Some(Self {
                bus,
                device,
                function,
            })
        } else {
            None
        }
    }

    #[cfg(target_arch = "x86_64")]
    const fn config_key(self, offset: u8) -> u32 {
        1 << 31
            | (self.bus as u32) << 16
            | (self.device as u32) << 11
            | (self.function as u32) << 8
            | (offset as u32 & 0xfc)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bar {
    Unused,
    Io { port: u32 },
    Memory32 { address: u32, prefetchable: bool },
    Memory64 { address: u64, prefetchable: bool },
}

impl Bar {
    pub const fn memory_address(self) -> Option<u64> {
        match self {
            Self::Memory32 { address, .. } => Some(address as u64),
            Self::Memory64 { address, .. } => Some(address),
            _ => None,
        }
    }

    pub const fn io_port(self) -> Option<u32> {
        match self {
            Self::Io { port } => Some(port),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciDevice {
    pub address: PciAddress,
    pub vendor_id: u16,
    pub device_id: u16,
    pub revision: u8,
    pub programming_interface: u8,
    pub subclass: u8,
    pub class: u8,
    pub header_type: u8,
    pub bars: [Bar; 6],
    pub interrupt_line: u8,
    pub interrupt_pin: u8,
}

impl PciDevice {
    pub const fn is_class(&self, class: u8, subclass: u8) -> bool {
        self.class == class && self.subclass == subclass
    }

    pub const fn is_ahci(&self) -> bool {
        self.is_class(0x01, 0x06) && self.programming_interface == 0x01
    }

    pub const fn is_nvme(&self) -> bool {
        self.is_class(0x01, 0x08) && self.programming_interface == 0x02
    }

    pub const fn is_ethernet(&self) -> bool {
        self.is_class(0x02, 0x00)
    }
}

/// PCI configuration-space access granted to a driver service.
///
/// # Safety
/// Implementors must serialize accesses and may only touch configuration space
/// authorized for the calling service.
pub unsafe trait ConfigAccess {
    unsafe fn read_u32(&mut self, address: PciAddress, offset: u8) -> u32;

    unsafe fn write_u32(&mut self, address: PciAddress, offset: u8, value: u32);
}

/// Enumerates all conventional PCI buses without allocating memory.
pub fn enumerate<A, F>(access: &mut A, mut visit: F)
where
    A: ConfigAccess,
    F: FnMut(PciDevice),
{
    for bus in 0..=u8::MAX {
        for device in 0..32 {
            let address = PciAddress {
                bus,
                device,
                function: 0,
            };
            let vendor = unsafe { access.read_u32(address, 0) } as u16;
            if vendor == INVALID_VENDOR {
                continue
            }

            let header = unsafe { access.read_u32(address, 0x0c) };
            let function_count = if header.to_le_bytes()[2] & 0x80 != 0 {
                8
            } else {
                1
            };

            for function in 0..function_count {
                let address = PciAddress {
                    bus,
                    device,
                    function,
                };
                if let Some(found) = read_device(access, address) {
                    visit(found)
                }
            }
        }
    }
}

pub fn read_device<A: ConfigAccess>(
    access: &mut A,
    address: PciAddress,
) -> Option<PciDevice> {
    let identity = unsafe { access.read_u32(address, 0) };
    let vendor_id = identity as u16;
    if vendor_id == INVALID_VENDOR {
        return None
    }

    let class = unsafe { access.read_u32(address, 0x08) }.to_le_bytes();
    let header = unsafe { access.read_u32(address, 0x0c) }.to_le_bytes()[2];
    let mut bars = [Bar::Unused; 6];

    if header & 0x7f == 0 {
        let mut index = 0;
        while index < bars.len() {
            let offset = 0x10 + (index * 4) as u8;
            let low = unsafe { access.read_u32(address, offset) };
            if low == 0 {
                index += 1;
                continue
            }

            if low & 1 != 0 {
                bars[index] = Bar::Io { port: low & !3 };
                index += 1;
                continue
            }

            let prefetchable = low & 8 != 0;
            if low & 6 == 4 && index + 1 < bars.len() {
                let high = unsafe { access.read_u32(address, offset + 4) };
                bars[index] = Bar::Memory64 {
                    address: ((high as u64) << 32) | (low as u64 & !0xf),
                    prefetchable,
                };
                index += 2;
            } else {
                bars[index] = Bar::Memory32 {
                    address: low & !0xf,
                    prefetchable,
                };
                index += 1;
            }
        }
    }

    let interrupt = unsafe { access.read_u32(address, 0x3c) }.to_le_bytes();
    Some(PciDevice {
        address,
        vendor_id,
        device_id: (identity >> 16) as u16,
        revision: class[0],
        programming_interface: class[1],
        subclass: class[2],
        class: class[3],
        header_type: header,
        bars,
        interrupt_line: interrupt[0],
        interrupt_pin: interrupt[1],
    })
}

/// Legacy x86 PCI mechanism #1 using ports CF8/CFC.
///
/// The caller must hold the platform I/O-port capability while this value is
/// used.
#[cfg(target_arch = "x86_64")]
pub struct PortConfig;

#[cfg(target_arch = "x86_64")]
unsafe impl ConfigAccess for PortConfig {
    unsafe fn read_u32(&mut self, address: PciAddress, offset: u8) -> u32 {
        unsafe {
            out_u32(0xcf8, address.config_key(offset));
            in_u32(0xcfc)
        }
    }

    unsafe fn write_u32(&mut self, address: PciAddress, offset: u8, value: u32) {
        unsafe {
            out_u32(0xcf8, address.config_key(offset));
            out_u32(0xcfc, value)
        }
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn out_u32(port: u16, value: u32) {
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack))
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn in_u32(port: u16) -> u32 {
    let value;
    unsafe {
        asm!("in eax, dx", out("eax") value, in("dx") port, options(nomem, nostack))
    }
    value
}
