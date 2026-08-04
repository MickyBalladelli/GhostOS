//! Legacy Virtio PCI devices.
//!
//! The VM exposes the legacy (0.9) Virtio transport because it is small,
//! widely supported by early guest kernels, and fits the existing port I/O
//! device model.  The block, console, and RNG devices share the split
//! virtqueue implementation below.

use crate::devices::storage::DiskImage;
use crate::devices::serial::write_host_console;
use crate::devices::{ApicTrigger, Device, DeviceError, LocalApic, PortDevice};
use crate::memory::Mmu;
use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Write};
use std::rc::Rc;

const CONSOLE_OUTPUT_LIMIT: usize = 1024 * 1024;
const CONSOLE_OUTPUT_COMPACTION_THRESHOLD: usize = CONSOLE_OUTPUT_LIMIT * 2;
use std::time::{SystemTime, UNIX_EPOCH};

pub const VIRTIO_PCI_VENDOR_ID: u16 = 0x1AF4;
pub const VIRTIO_BLK_DEVICE_ID: u16 = 0x1001;
pub const VIRTIO_CONSOLE_DEVICE_ID: u16 = 0x1003;
pub const VIRTIO_RNG_DEVICE_ID: u16 = 0x1004;
pub const VIRTIO_PCI_BAR0_SIZE: u64 = 0x100;

pub const VIRTIO_BLK_CLASS: u8 = 0x01;
pub const VIRTIO_BLK_SUBCLASS: u8 = 0x00;
pub const VIRTIO_BLK_PROG_IF: u8 = 0x00;
pub const VIRTIO_CONSOLE_CLASS: u8 = 0x07;
pub const VIRTIO_CONSOLE_SUBCLASS: u8 = 0x00;
pub const VIRTIO_CONSOLE_PROG_IF: u8 = 0x00;
pub const VIRTIO_RNG_CLASS: u8 = 0x08;
pub const VIRTIO_RNG_SUBCLASS: u8 = 0x00;
pub const VIRTIO_RNG_PROG_IF: u8 = 0x00;

const REG_DEVICE_FEATURES: u16 = 0x00;
const REG_GUEST_FEATURES: u16 = 0x04;
const REG_QUEUE_PFN: u16 = 0x08;
const REG_QUEUE_SIZE: u16 = 0x0C;
const REG_QUEUE_SEL: u16 = 0x0E;
const REG_QUEUE_NOTIFY: u16 = 0x10;
const REG_STATUS: u16 = 0x12;
const REG_ISR_STATUS: u16 = 0x13;
const REG_CONFIG: u16 = 0x14;

const STATUS_DRIVER_OK: u8 = 1 << 2;
const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;
const DESC_INDIRECT: u16 = 4;
const QUEUE_SIZE: u16 = 128;
const DESC_SIZE: u64 = 16;
const MAX_CHAIN: usize = 64;

const BLK_T_IN: u32 = 0;
const BLK_T_OUT: u32 = 1;
const BLK_T_FLUSH: u32 = 4;
const BLK_S_OK: u8 = 0;
const BLK_S_IOERR: u8 = 1;
const BLK_S_UNSUPP: u8 = 2;

#[derive(Clone, Copy)]
struct Descriptor {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

struct VirtioQueue {
    pfn: u32,
    avail_last: u16,
    used_idx: u16,
}

impl VirtioQueue {
    fn new() -> Self {
        Self {
            pfn: 0,
            avail_last: 0,
            used_idx: 0,
        }
    }

    fn reset(&mut self) {
        self.pfn = 0;
        self.avail_last = 0;
        self.used_idx = 0;
    }

    fn enabled(&self) -> bool {
        self.pfn != 0
    }

    fn desc_base(&self) -> u64 {
        (self.pfn as u64) << 12
    }

    fn avail_base(&self) -> u64 {
        self.desc_base() + QUEUE_SIZE as u64 * DESC_SIZE
    }

    fn used_base(&self) -> u64 {
        let avail_end = self.avail_base() + 4 + QUEUE_SIZE as u64 * 2;
        (avail_end + 3) & !3
    }

    fn next_available(&mut self, mmu: &Mmu) -> Option<u16> {
        if !self.enabled() {
            return None;
        }
        let avail_idx = read_u16(mmu, self.avail_base() + 2)?;
        if self.avail_last == avail_idx {
            return None;
        }
        let slot = self.avail_last as u64 & (QUEUE_SIZE as u64 - 1);
        let head = read_u16(mmu, self.avail_base() + 4 + slot * 2)?;
        self.avail_last = self.avail_last.wrapping_add(1);
        Some(head)
    }

    fn chain(&self, mmu: &Mmu, head: u16) -> Result<Vec<Descriptor>, ()> {
        if head >= QUEUE_SIZE {
            return Err(());
        }
        let mut descriptors = Vec::new();
        let mut index = head;
        for _ in 0..MAX_CHAIN {
            let addr = self.desc_base() + index as u64 * DESC_SIZE;
            let bytes = mmu.read_phys(addr, DESC_SIZE as usize).map_err(|_| ())?;
            let descriptor = Descriptor {
                addr: u64::from_le_bytes(bytes[0..8].try_into().map_err(|_| ())?),
                len: u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| ())?),
                flags: u16::from_le_bytes(bytes[12..14].try_into().map_err(|_| ())?),
                next: u16::from_le_bytes(bytes[14..16].try_into().map_err(|_| ())?),
            };
            if descriptor.flags & DESC_INDIRECT != 0 {
                return Err(());
            }
            descriptors.push(descriptor);
            if descriptor.flags & DESC_NEXT == 0 {
                return Ok(descriptors);
            }
            index = descriptor.next;
            if index >= QUEUE_SIZE {
                return Err(());
            }
        }
        Err(())
    }

    fn complete(&mut self, mmu: &mut Mmu, head: u16, len: u32) -> bool {
        let slot = self.used_idx as u64 & (QUEUE_SIZE as u64 - 1);
        let entry = self.used_base() + 4 + slot * 8;
        let mut bytes = [0u8; 8];
        bytes[0..2].copy_from_slice(&head.to_le_bytes());
        bytes[4..8].copy_from_slice(&len.to_le_bytes());
        if mmu.write_phys(entry, &bytes).is_err() {
            return false;
        }
        self.used_idx = self.used_idx.wrapping_add(1);
        mmu.write_phys(self.used_base() + 2, &self.used_idx.to_le_bytes())
            .is_ok()
    }
}

fn read_u16(mmu: &Mmu, addr: u64) -> Option<u16> {
    let bytes = mmu.read_phys(addr, 2).ok()?;
    Some(u16::from_le_bytes(bytes.try_into().ok()?))
}

struct VirtioTransport {
    guest_features: u32,
    status: u8,
    queue_sel: u16,
    queue: VirtioQueue,
    interrupt_status: u8,
    pending: bool,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

impl VirtioTransport {
    fn new() -> Self {
        Self {
            guest_features: 0,
            status: 0,
            queue_sel: 0,
            queue: VirtioQueue::new(),
            interrupt_status: 0,
            pending: false,
            apic: None,
            irq_vector: 0,
        }
    }

    fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic);
    }

    fn set_irq_vector(&mut self, vector: u8) {
        self.irq_vector = vector;
    }

    fn read_common(&mut self, off: u16, device_features: u32) -> Option<u64> {
        match off {
            REG_DEVICE_FEATURES => Some(device_features as u64),
            REG_GUEST_FEATURES => Some(self.guest_features as u64),
            REG_QUEUE_PFN => Some(self.queue.pfn as u64),
            REG_QUEUE_SIZE => Some(QUEUE_SIZE as u64),
            REG_QUEUE_SEL => Some(self.queue_sel as u64),
            REG_STATUS => Some(self.status as u64),
            REG_ISR_STATUS => {
                let status = self.interrupt_status;
                self.interrupt_status = 0;
                Some(status as u64)
            }
            _ => None,
        }
    }

    fn write_common(&mut self, off: u16, value: u32) -> bool {
        match off {
            REG_GUEST_FEATURES => self.guest_features = value,
            REG_QUEUE_PFN => {
                self.queue.pfn = value;
                self.queue.avail_last = 0;
                self.queue.used_idx = 0;
            }
            REG_QUEUE_SEL => self.queue_sel = value as u16,
            REG_QUEUE_NOTIFY => self.pending = true,
            REG_STATUS => {
                self.status = value as u8;
                if self.status == 0 {
                    self.queue.reset();
                    self.interrupt_status = 0;
                    self.pending = false;
                }
            }
            _ => {}
        }
        off == REG_QUEUE_NOTIFY
    }

    fn mark_complete(&mut self) {
        self.interrupt_status |= 1;
        if self.irq_vector != 0 {
            if let Some(apic) = &self.apic {
                apic.borrow_mut().signal(self.irq_vector, ApicTrigger::Edge);
            }
        }
    }

    fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    fn reset(&mut self) {
        let apic = self.apic.take();
        let vector = self.irq_vector;
        *self = Self::new();
        self.apic = apic;
        self.irq_vector = vector;
    }
}

/// Legacy Virtio block device backed by a [`DiskImage`].
pub struct VirtioBlk {
    transport: VirtioTransport,
    disk: Option<DiskImage>,
}

impl VirtioBlk {
    pub fn new() -> Self {
        Self {
            transport: VirtioTransport::new(),
            disk: None,
        }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.transport.attach_apic(apic)
    }

    pub fn set_irq_vector(&mut self, vector: u8) {
        self.transport.set_irq_vector(vector)
    }

    pub fn attach_disk(&mut self, disk: DiskImage) -> Option<DiskImage> {
        self.disk.replace(disk)
    }

    pub fn detach_disk(&mut self) -> Option<DiskImage> {
        self.disk.take()
    }

    pub fn flush_disk(&mut self) -> Result<(), crate::devices::storage::StorageError> {
        self.disk
            .as_mut()
            .ok_or_else(|| crate::devices::storage::StorageError::InvalidImage("no disk".into()))?
            .flush()
    }

    pub fn sync_disk(&mut self) -> Result<(), crate::devices::storage::StorageError> {
        self.disk
            .as_mut()
            .ok_or_else(|| crate::devices::storage::StorageError::InvalidImage("no disk".into()))?
            .sync()
    }

    pub fn sector_count(&self) -> Option<u64> {
        self.disk.as_ref().map(DiskImage::sector_count)
    }

    pub fn has_pending(&self) -> bool {
        self.transport.pending
    }

    pub fn poll(&mut self, mmu: &mut Mmu) {
        if !self.transport.take_pending() || self.transport.status & STATUS_DRIVER_OK == 0 {
            return;
        }
        while let Some(head) = self.transport.queue.next_available(mmu) {
            let (status, used_len) = self.process_request(mmu, head);
            let _ = self.write_request_status(mmu, head, status);
            if self.transport.queue.complete(mmu, head, used_len) {
                self.transport.mark_complete();
            }
        }
    }

    fn process_request(&mut self, mmu: &mut Mmu, head: u16) -> (u8, u32) {
        let chain = match self.transport.queue.chain(mmu, head) {
            Ok(chain) if chain.len() >= 2 => chain,
            _ => return (BLK_S_IOERR, 1),
        };
        let header = &chain[0];
        if header.flags & DESC_WRITE != 0 || header.len < 16 {
            return (BLK_S_IOERR, 1);
        }
        let header_bytes = match mmu.read_phys(header.addr, 16) {
            Ok(bytes) => bytes,
            Err(_) => return (BLK_S_IOERR, 1),
        };
        let request_type = u32::from_le_bytes(header_bytes[0..4].try_into().unwrap());
        let sector = u64::from_le_bytes(header_bytes[8..16].try_into().unwrap());
        if !matches!(
            chain.last(),
            Some(desc) if desc.flags & DESC_WRITE != 0 && desc.len >= 1
        ) {
            return (BLK_S_IOERR, 1);
        }
        let data = &chain[1..chain.len() - 1];
        let data_len = data.iter().map(|desc| desc.len as usize).sum::<usize>();

        match request_type {
            BLK_T_IN => {
                if data_len == 0 || data_len % 512 != 0 {
                    return (BLK_S_IOERR, 1);
                }
                let sectors = data_len / 512;
                let mut bytes = vec![0u8; data_len];
                let Some(disk) = self.disk.as_mut() else {
                    return (BLK_S_IOERR, 1);
                };
                for index in 0..sectors {
                    let mut sector_bytes = [0u8; 512];
                    if disk.read_sector(sector + index as u64, &mut sector_bytes).is_err() {
                        return (BLK_S_IOERR, 1);
                    }
                    bytes[index * 512..(index + 1) * 512].copy_from_slice(&sector_bytes);
                }
                if !scatter_write(mmu, data, &bytes) {
                    return (BLK_S_IOERR, 1);
                }
                (BLK_S_OK, data_len as u32)
            }
            BLK_T_OUT => {
                if data_len == 0 || data_len % 512 != 0 {
                    return (BLK_S_IOERR, 1);
                }
                let Some(bytes) = gather_read(mmu, data) else {
                    return (BLK_S_IOERR, 1);
                };
                let Some(disk) = self.disk.as_mut() else {
                    return (BLK_S_IOERR, 1);
                };
                for (index, sector_bytes) in bytes.chunks_exact(512).enumerate() {
                    let sector_bytes: &[u8; 512] = sector_bytes.try_into().unwrap();
                    if disk.write_sector(sector + index as u64, sector_bytes).is_err() {
                        return (BLK_S_IOERR, 1);
                    }
                }
                (BLK_S_OK, 1)
            }
            BLK_T_FLUSH => {
                if self.disk.is_some() {
                    (BLK_S_OK, 1)
                } else {
                    (BLK_S_IOERR, 1)
                }
            }
            _ => {
                (BLK_S_UNSUPP, 1)
            }
        }
    }

    fn write_request_status(&self, mmu: &mut Mmu, head: u16, status: u8) -> bool {
        let Ok(chain) = self.transport.queue.chain(mmu, head) else {
            return false;
        };
        let Some(desc) = chain.last() else {
            return false;
        };
        mmu.write_phys(desc.addr, &[status]).is_ok()
    }

    fn read_io(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        let off = port & (VIRTIO_PCI_BAR0_SIZE as u16 - 1);
        if let Some(value) = self.transport.read_common(off, 1 << 6) {
            return Ok(value);
        }
        if (REG_CONFIG..REG_CONFIG + 8).contains(&off) {
            let capacity = self.sector_count().unwrap_or(0).to_le_bytes();
            let start = (off - REG_CONFIG) as usize;
            let width = size as usize;
            if width != 1 && width != 2 && width != 4 && width != 8 || start + width > capacity.len() {
                return Err(DeviceError::UnsupportedSize);
            }
            return Ok(u64::from_le_bytes({
                let mut bytes = [0u8; 8];
                bytes[..width].copy_from_slice(&capacity[start..start + width]);
                bytes
            }));
        }
        Err(DeviceError::InvalidAddress)
    }

    fn write_io(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 1 && size != 2 && size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = port & (VIRTIO_PCI_BAR0_SIZE as u16 - 1);
        self.transport.write_common(off, value as u32);
        Ok(())
    }

    pub fn reset(&mut self) {
        self.transport.reset();
        self.disk = None;
    }
}

impl Default for VirtioBlk {
    fn default() -> Self {
        Self::new()
    }
}

/// Legacy Virtio console device. Guest output is retained for inspection and
/// also forwarded to the host stdout stream.
pub struct VirtioConsole {
    transport: VirtioTransport,
    output: Vec<u8>,
    host_last_was_cr: bool,
}

impl VirtioConsole {
    pub fn new() -> Self {
        Self {
            transport: VirtioTransport::new(),
            output: Vec::new(),
            host_last_was_cr: false,
        }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.transport.attach_apic(apic)
    }

    pub fn set_irq_vector(&mut self, vector: u8) {
        self.transport.set_irq_vector(vector)
    }

    pub fn has_pending(&self) -> bool {
        self.transport.pending
    }

    pub fn output(&self) -> &[u8] {
        &self.output
    }

    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }

    pub fn poll(&mut self, mmu: &mut Mmu) {
        if !self.transport.take_pending() || self.transport.status & STATUS_DRIVER_OK == 0 {
            return;
        }
        while let Some(head) = self.transport.queue.next_available(mmu) {
            let Ok(chain) = self.transport.queue.chain(mmu, head) else {
                continue;
            };
            let mut completed = 0u32;
            for descriptor in chain {
                if descriptor.flags & DESC_WRITE != 0 {
                    continue;
                }
                let Ok(bytes) = mmu.read_phys(descriptor.addr, descriptor.len as usize) else {
                    continue;
                };
                completed = completed.saturating_add(bytes.len() as u32);
                self.output.extend_from_slice(&bytes);
                let mut stdout = std::io::stdout().lock();
                let _ = write_host_console(&mut stdout, &bytes, &mut self.host_last_was_cr);
                let _ = stdout.flush();
            }
            if self.output.len() > CONSOLE_OUTPUT_COMPACTION_THRESHOLD {
                let excess = self.output.len() - CONSOLE_OUTPUT_LIMIT;
                self.output.drain(..excess);
            }
            if self.transport.queue.complete(mmu, head, completed) {
                self.transport.mark_complete();
            }
        }
    }

    fn read_io(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        let off = port & (VIRTIO_PCI_BAR0_SIZE as u16 - 1);
        if let Some(value) = self.transport.read_common(off, 0) {
            return Ok(value);
        }
        if (REG_CONFIG..REG_CONFIG + 4).contains(&off) {
            if size != 1 && size != 2 && size != 4 {
                return Err(DeviceError::UnsupportedSize);
            }
            return Ok(1);
        }
        Err(DeviceError::InvalidAddress)
    }

    fn write_io(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 1 && size != 2 && size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = port & (VIRTIO_PCI_BAR0_SIZE as u16 - 1);
        self.transport.write_common(off, value as u32);
        Ok(())
    }

    pub fn reset(&mut self) {
        self.transport.reset();
        self.output.clear();
        self.host_last_was_cr = false;
    }
}

impl Default for VirtioConsole {
    fn default() -> Self {
        Self::new()
    }
}

/// Legacy Virtio random-number generator backed by the host entropy source.
pub struct VirtioRng {
    transport: VirtioTransport,
    random: Option<File>,
    fallback_state: u64,
}

impl VirtioRng {
    pub fn new() -> Self {
        let fallback_state = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0xD1CE_BA5E_1234_5678);
        Self {
            transport: VirtioTransport::new(),
            random: File::open("/dev/urandom").ok(),
            fallback_state: fallback_state | 1,
        }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.transport.attach_apic(apic)
    }

    pub fn set_irq_vector(&mut self, vector: u8) {
        self.transport.set_irq_vector(vector)
    }

    pub fn has_pending(&self) -> bool {
        self.transport.pending
    }

    pub fn poll(&mut self, mmu: &mut Mmu) {
        if !self.transport.take_pending() || self.transport.status & STATUS_DRIVER_OK == 0 {
            return;
        }
        while let Some(head) = self.transport.queue.next_available(mmu) {
            let Ok(chain) = self.transport.queue.chain(mmu, head) else {
                continue;
            };
            let mut completed = 0u32;
            for descriptor in chain {
                if descriptor.flags & DESC_WRITE == 0 {
                    continue;
                }
                let mut bytes = vec![0u8; descriptor.len as usize];
                self.fill_random(&mut bytes);
                if mmu.write_phys(descriptor.addr, &bytes).is_err() {
                    continue;
                }
                completed = completed.saturating_add(bytes.len() as u32);
            }
            if self.transport.queue.complete(mmu, head, completed) {
                self.transport.mark_complete();
            }
        }
    }

    fn fill_random(&mut self, bytes: &mut [u8]) {
        if let Some(random) = &mut self.random {
            if random.read_exact(bytes).is_ok() {
                return;
            }
        }
        for byte in bytes {
            self.fallback_state ^= self.fallback_state << 7;
            self.fallback_state ^= self.fallback_state >> 9;
            self.fallback_state ^= self.fallback_state << 8;
            *byte = self.fallback_state as u8;
        }
    }

    fn read_io(&mut self, port: u16, _size: u8) -> Result<u64, DeviceError> {
        let off = port & (VIRTIO_PCI_BAR0_SIZE as u16 - 1);
        self.transport
            .read_common(off, 0)
            .ok_or(DeviceError::InvalidAddress)
    }

    fn write_io(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 1 && size != 2 && size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = port & (VIRTIO_PCI_BAR0_SIZE as u16 - 1);
        self.transport.write_common(off, value as u32);
        Ok(())
    }

    pub fn reset(&mut self) {
        self.transport.reset();
    }
}

impl Default for VirtioRng {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! impl_virtio_port_device {
    ($ty:ty) => {
        impl PortDevice for $ty {
            fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
                self.read_io(port, size)
            }

            fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
                self.write_io(port, value, size)
            }

            fn reset(&mut self) {
                <$ty>::reset(self)
            }
        }

        impl Device for $ty {
            fn read(&self, _addr: u64, _size: u8) -> Result<u64, DeviceError> {
                Err(DeviceError::InvalidAddress)
            }

            fn write(&mut self, _addr: u64, _value: u64, _size: u8) -> Result<(), DeviceError> {
                Err(DeviceError::InvalidAddress)
            }

            fn reset(&mut self) {
                <$ty>::reset(self)
            }
        }
    };
}

impl_virtio_port_device!(VirtioBlk);
impl_virtio_port_device!(VirtioConsole);
impl_virtio_port_device!(VirtioRng);

macro_rules! impl_virtio_rc_device {
    ($ty:ty) => {
        impl PortDevice for Rc<RefCell<$ty>> {
            fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
                PortDevice::read(&mut *self.borrow_mut(), port, size)
            }

            fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
                PortDevice::write(&mut *self.borrow_mut(), port, value, size)
            }

            fn reset(&mut self) {
                self.borrow_mut().reset()
            }
        }

        impl Device for Rc<RefCell<$ty>> {
            fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
                Device::read(&*self.borrow(), addr, size)
            }

            fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
                Device::write(&mut *self.borrow_mut(), addr, value, size)
            }

            fn reset(&mut self) {
                self.borrow_mut().reset()
            }
        }
    };
}

impl_virtio_rc_device!(VirtioBlk);
impl_virtio_rc_device!(VirtioConsole);
impl_virtio_rc_device!(VirtioRng);

fn gather_read(mmu: &Mmu, descriptors: &[Descriptor]) -> Option<Vec<u8>> {
    let mut result = Vec::new();
    for descriptor in descriptors {
        if descriptor.flags & DESC_WRITE != 0 {
            return None;
        }
        result.extend_from_slice(&mmu.read_phys(descriptor.addr, descriptor.len as usize).ok()?);
    }
    Some(result)
}

fn scatter_write(mmu: &mut Mmu, descriptors: &[Descriptor], bytes: &[u8]) -> bool {
    let mut offset = 0usize;
    for descriptor in descriptors {
        if descriptor.flags & DESC_WRITE == 0 {
            return false;
        }
        let len = descriptor.len as usize;
        if offset + len > bytes.len() || mmu.write_phys(descriptor.addr, &bytes[offset..offset + len]).is_err() {
            return false;
        }
        offset += len;
    }
    offset == bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn transport_negotiates_queue_and_resets() {
        let mut transport = VirtioTransport::new();
        assert_eq!(transport.read_common(REG_DEVICE_FEATURES, 0x55), Some(0x55));
        transport.write_common(REG_GUEST_FEATURES, 0x55);
        transport.write_common(REG_QUEUE_PFN, 2);
        transport.write_common(REG_STATUS, STATUS_DRIVER_OK as u32);
        assert_eq!(transport.queue.pfn, 2);
        assert_eq!(transport.status, STATUS_DRIVER_OK);

        transport.write_common(REG_QUEUE_NOTIFY, 0);
        assert!(transport.take_pending());
        transport.write_common(REG_STATUS, 0);
        assert!(!transport.queue.enabled());
        transport.reset();
        assert_eq!(transport.status, 0);
        assert_eq!(transport.guest_features, 0);
    }

    #[test]
    fn malformed_descriptor_chain_is_rejected() {
        let mut mmu = Mmu::new(0x20_000);
        let mut queue = VirtioQueue::new();
        queue.pfn = 1;
        let descriptor = [0u8; 16];
        mmu.write_phys(queue.desc_base(), &descriptor).unwrap();
        let mut bad = descriptor;
        bad[12..14].copy_from_slice(&DESC_INDIRECT.to_le_bytes());
        mmu.write_phys(queue.desc_base(), &bad).unwrap();
        assert!(queue.chain(&mmu, 0).is_err());
        assert!(queue.chain(&mmu, QUEUE_SIZE).is_err());
    }

    #[test]
    fn block_capacity_and_console_output_are_visible() {
        let path = std::env::temp_dir().join(format!("synos-virtio-{}", std::process::id()));
        let mut file = File::create(path.clone()).unwrap();
        file.set_len(4096).unwrap();
        file.flush().unwrap();

        let mut block = VirtioBlk::new();
        block.attach_disk(DiskImage::open(path).unwrap());
        assert_eq!(block.sector_count(), Some(8));
        assert_eq!(block.read_io(REG_CONFIG, 8).unwrap(), 8);
        assert_eq!(block.read_io(REG_CONFIG + 7, 2), Err(DeviceError::UnsupportedSize));

        let mut console = VirtioConsole::new();
        console.output.extend_from_slice(b"hello");
        assert_eq!(console.take_output(), b"hello");
        console.reset();
        assert!(console.output().is_empty());

        let mut rng = VirtioRng::new();
        let mut bytes = [0u8; 32];
        rng.fill_random(&mut bytes);
        assert_ne!(bytes, [0; 32]);
    }
}
