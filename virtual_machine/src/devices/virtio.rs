//! Legacy Virtio block, console, and RNG devices.
//! C owns transport state, request execution, DMA ordering, console capture,
//! and entropy fallback. Rust connects MMU, disk, host I/O, and shared APIC.

use crate::devices::storage::DiskImage;
use crate::devices::{ApicTrigger, Device, DeviceError, LocalApic, PortDevice};
use crate::memory::Mmu;
use std::cell::RefCell;
use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Write};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use crate::devices::virtio_queue::{VirtioQueue, QUEUE_SIZE};

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

#[cfg(test)]
const REG_DEVICE_FEATURES: u16 = 0;
#[cfg(test)]
const REG_GUEST_FEATURES: u16 = 4;
#[cfg(test)]
const REG_QUEUE_PFN: u16 = 8;
#[cfg(test)]
const REG_QUEUE_NOTIFY: u16 = 0x10;
#[cfg(test)]
const REG_STATUS: u16 = 0x12;
#[cfg(test)]
const REG_CONFIG: u16 = 0x14;
#[cfg(test)]
const STATUS_DRIVER_OK: u8 = 4;

#[derive(Default)]
#[repr(C)]
struct CTransport {
    guest_features: u32,
    pfn: u32,
    avail_last: u16,
    used_idx: u16,
    queue_sel: u16,
    status: u8,
    interrupt_status: u8,
    pending: bool,
}

const _: () = {
    assert!(std::mem::size_of::<CTransport>() == 20);
    assert!(std::mem::offset_of!(CTransport, status) == 14);
    assert!(std::mem::size_of::<CIo>() == 72);
};

#[repr(C)]
struct CConsole {
    _private: [u8; 0],
}

#[repr(C)]
struct CIo {
    read_memory: unsafe extern "C" fn(*mut c_void, u64, *mut u8, usize) -> bool,
    write_memory: unsafe extern "C" fn(*mut c_void, u64, *const u8, usize) -> bool,
    read_sector: unsafe extern "C" fn(*mut c_void, u64, *mut u8) -> bool,
    write_sector: unsafe extern "C" fn(*mut c_void, u64, *const u8) -> bool,
    flush_disk: unsafe extern "C" fn(*mut c_void) -> bool,
    entropy: unsafe extern "C" fn(*mut c_void, *mut u8, usize) -> bool,
    console: unsafe extern "C" fn(*mut c_void, *const u8, usize),
    interrupt: unsafe extern "C" fn(*mut c_void),
    context: *mut c_void,
}

unsafe extern "C" {
    fn ghostos_vm_virtio_transport_init(transport: *mut CTransport);
    #[cfg(test)]
    fn ghostos_vm_virtio_read_common(transport: *mut CTransport, offset: u16,
        features: u32, value: *mut u64) -> bool;
    #[cfg(test)]
    fn ghostos_vm_virtio_write_common(transport: *mut CTransport, offset: u16, value: u32) -> bool;
    #[cfg(test)]
    fn ghostos_vm_virtio_take_pending(transport: *mut CTransport) -> bool;
    fn ghostos_vm_virtio_device_read(transport: *mut CTransport, kind: u8,
        port: u16, size: u8, sectors: u64, value: *mut u64) -> u8;
    fn ghostos_vm_virtio_device_write(transport: *mut CTransport, port: u16, value: u64, size: u8) -> u8;
    fn ghostos_vm_virtio_console_new() -> *mut CConsole;
    fn ghostos_vm_virtio_console_free(console: *mut CConsole);
    fn ghostos_vm_virtio_console_reset(console: *mut CConsole);
    fn ghostos_vm_virtio_console_output(console: *const CConsole, length: *mut usize) -> *const u8;
    fn ghostos_vm_virtio_console_clear_output(console: *mut CConsole);
    #[cfg(test)]
    fn ghostos_vm_virtio_console_append(console: *mut CConsole, bytes: *const u8, length: usize) -> bool;
    #[cfg(test)]
    fn ghostos_vm_virtio_fill_random(fallback_state: *mut u64, bytes: *mut u8, length: usize, io: *const CIo);
    fn ghostos_vm_virtio_block_poll(transport: *mut CTransport, io: *const CIo) -> bool;
    fn ghostos_vm_virtio_console_poll(transport: *mut CTransport, console: *mut CConsole, io: *const CIo) -> bool;
    fn ghostos_vm_virtio_rng_poll(transport: *mut CTransport, fallback_state: *mut u64, io: *const CIo) -> bool;
}

struct PollContext<'a> {
    mmu: Option<&'a mut Mmu>,
    disk: Option<&'a mut DiskImage>,
    random: Option<&'a mut File>,
    apic: &'a Option<Rc<RefCell<LocalApic>>>,
    vector: u8,
}

impl PollContext<'_> {
    fn io(&mut self) -> CIo {
        CIo { read_memory, write_memory, read_sector, write_sector, flush_disk,
            entropy, console: write_console, interrupt, context: (self as *mut Self).cast() }
    }
}

// All callbacks are synchronous. C owns each live buffer and supplies a
// pointer to the exclusive, stack-local host context without retaining it.
unsafe extern "C" fn read_memory(context: *mut c_void, address: u64, output: *mut u8, length: usize) -> bool {
    let context = unsafe { &mut *context.cast::<PollContext<'_>>() };
    let Some(mmu) = context.mmu.as_mut() else { return false };
    let Ok(bytes) = mmu.read_phys(address, length) else { return false };
    if length != 0 { unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, length) } }
    true
}

unsafe extern "C" fn write_memory(context: *mut c_void, address: u64, input: *const u8, length: usize) -> bool {
    let context = unsafe { &mut *context.cast::<PollContext<'_>>() };
    let Some(mmu) = context.mmu.as_mut() else { return false };
    let bytes = unsafe { std::slice::from_raw_parts(input, length) };
    mmu.write_phys(address, bytes).is_ok()
}

unsafe extern "C" fn read_sector(context: *mut c_void, sector: u64, output: *mut u8) -> bool {
    let context = unsafe { &mut *context.cast::<PollContext<'_>>() };
    let Some(disk) = context.disk.as_mut() else { return false };
    disk.read_sector(sector, unsafe { &mut *output.cast::<[u8; 512]>() }).is_ok()
}

unsafe extern "C" fn write_sector(context: *mut c_void, sector: u64, input: *const u8) -> bool {
    let context = unsafe { &mut *context.cast::<PollContext<'_>>() };
    let Some(disk) = context.disk.as_mut() else { return false };
    disk.write_sector(sector, unsafe { &*input.cast::<[u8; 512]>() }).is_ok()
}

unsafe extern "C" fn flush_disk(context: *mut c_void) -> bool {
    let context = unsafe { &mut *context.cast::<PollContext<'_>>() };
    let Some(disk) = context.disk.as_mut() else { return false };
    disk.flush().is_ok()
}

unsafe extern "C" fn entropy(context: *mut c_void, output: *mut u8, length: usize) -> bool {
    let context = unsafe { &mut *context.cast::<PollContext<'_>>() };
    let Some(random) = context.random.as_mut() else { return false };
    let bytes = unsafe { std::slice::from_raw_parts_mut(output, length) };
    random.read_exact(bytes).is_ok()
}

unsafe extern "C" fn write_console(_context: *mut c_void, bytes: *const u8, length: usize) {
    let bytes = unsafe { std::slice::from_raw_parts(bytes, length) };
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(bytes);
    let _ = stdout.flush();
}

unsafe extern "C" fn interrupt(context: *mut c_void) {
    let context = unsafe { &mut *context.cast::<PollContext<'_>>() };
    if context.vector != 0 {
        if let Some(apic) = context.apic {
            apic.borrow_mut().signal(context.vector, ApicTrigger::Edge)
        }
    }
}

fn port_result(code: u8) -> Result<(), DeviceError> {
    match code {
        0 => Ok(()),
        1 => Err(DeviceError::UnsupportedSize),
        _ => Err(DeviceError::InvalidAddress),
    }
}

struct VirtioTransport {
    state: CTransport,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

impl VirtioTransport {
    fn new() -> Self {
        Self { state: CTransport::default(), apic: None, irq_vector: 0 }
    }

    fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) { self.apic = Some(apic) }
    fn set_irq_vector(&mut self, vector: u8) { self.irq_vector = vector }
    fn reset(&mut self) { unsafe { ghostos_vm_virtio_transport_init(&mut self.state) } }

    fn read_io(&mut self, kind: u8, port: u16, size: u8, sectors: u64) -> Result<u64, DeviceError> {
        let mut value = 0;
        let code = unsafe { ghostos_vm_virtio_device_read(&mut self.state, kind, port, size, sectors, &mut value) };
        port_result(code)?;
        Ok(value)
    }

    fn write_io(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        port_result(unsafe { ghostos_vm_virtio_device_write(&mut self.state, port, value, size) })
    }

    #[cfg(test)]
    fn read_common(&mut self, offset: u16, features: u32) -> Option<u64> {
        let mut value = 0;
        let found = unsafe { ghostos_vm_virtio_read_common(&mut self.state, offset, features, &mut value) };
        found.then_some(value)
    }
    #[cfg(test)]
    fn write_common(&mut self, offset: u16, value: u32) -> bool {
        unsafe { ghostos_vm_virtio_write_common(&mut self.state, offset, value) }
    }
    #[cfg(test)]
    fn take_pending(&mut self) -> bool { unsafe { ghostos_vm_virtio_take_pending(&mut self.state) } }
}

/// Legacy Virtio block device backed by a DiskImage host adapter.
pub struct VirtioBlk {
    transport: VirtioTransport,
    disk: Option<DiskImage>,
}

impl VirtioBlk {
    pub fn new() -> Self { Self { transport: VirtioTransport::new(), disk: None } }
    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) { self.transport.attach_apic(apic) }
    pub fn set_irq_vector(&mut self, vector: u8) { self.transport.set_irq_vector(vector) }
    pub fn attach_disk(&mut self, disk: DiskImage) -> Option<DiskImage> { self.disk.replace(disk) }
    pub fn detach_disk(&mut self) -> Option<DiskImage> { self.disk.take() }

    pub fn flush_disk(&mut self) -> Result<(), crate::devices::storage::StorageError> {
        self.disk.as_mut().ok_or_else(|| crate::devices::storage::StorageError::InvalidImage("no disk".into()))?.flush()
    }
    pub fn sync_disk(&mut self) -> Result<(), crate::devices::storage::StorageError> {
        self.disk.as_mut().ok_or_else(|| crate::devices::storage::StorageError::InvalidImage("no disk".into()))?.sync()
    }
    pub fn sector_count(&self) -> Option<u64> { self.disk.as_ref().map(DiskImage::sector_count) }
    pub fn has_pending(&self) -> bool { self.transport.state.pending }

    pub fn poll(&mut self, mmu: &mut Mmu) {
        let mut context = PollContext { mmu: Some(mmu), disk: self.disk.as_mut(), random: None,
            apic: &self.transport.apic, vector: self.transport.irq_vector };
        let io = context.io();
        assert!(unsafe { ghostos_vm_virtio_block_poll(&mut self.transport.state, &io) },
            "could not allocate Virtio block DMA buffer")
    }

    fn read_io(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        self.transport.read_io(0, port, size, self.sector_count().unwrap_or(0))
    }
    fn write_io(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        self.transport.write_io(port, value, size)
    }
    pub fn reset(&mut self) { self.transport.reset(); self.disk = None; }
}

impl Default for VirtioBlk { fn default() -> Self { Self::new() } }

/// C-owned console capture with host stdout and shared APIC adapters.
pub struct VirtioConsole {
    transport: VirtioTransport,
    state: *mut CConsole,
}

impl VirtioConsole {
    pub fn new() -> Self {
        let state = unsafe { ghostos_vm_virtio_console_new() };
        assert!(!state.is_null(), "could not allocate Virtio console");
        Self { transport: VirtioTransport::new(), state }
    }
    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) { self.transport.attach_apic(apic) }
    pub fn set_irq_vector(&mut self, vector: u8) { self.transport.set_irq_vector(vector) }
    pub fn has_pending(&self) -> bool { self.transport.state.pending }
    pub fn output(&self) -> &[u8] {
        let mut length = 0;
        let bytes = unsafe { ghostos_vm_virtio_console_output(self.state, &mut length) };
        if length == 0 { &[] } else { unsafe { std::slice::from_raw_parts(bytes, length) } }
    }
    pub fn take_output(&mut self) -> Vec<u8> {
        let output = self.output().to_vec();
        unsafe { ghostos_vm_virtio_console_clear_output(self.state) };
        output
    }
    pub fn poll(&mut self, mmu: &mut Mmu) {
        let mut context = PollContext { mmu: Some(mmu), disk: None, random: None,
            apic: &self.transport.apic, vector: self.transport.irq_vector };
        let io = context.io();
        assert!(unsafe { ghostos_vm_virtio_console_poll(&mut self.transport.state, self.state, &io) },
            "could not allocate Virtio console buffer")
    }
    fn read_io(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        self.transport.read_io(1, port, size, 0)
    }
    fn write_io(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        self.transport.write_io(port, value, size)
    }
    pub fn reset(&mut self) {
        self.transport.reset();
        unsafe { ghostos_vm_virtio_console_reset(self.state) }
    }
    #[cfg(test)]
    fn append_output(&mut self, bytes: &[u8]) {
        assert!(unsafe { ghostos_vm_virtio_console_append(self.state, bytes.as_ptr(), bytes.len()) })
    }
}

impl Default for VirtioConsole { fn default() -> Self { Self::new() } }
impl Drop for VirtioConsole {
    fn drop(&mut self) { unsafe { ghostos_vm_virtio_console_free(self.state) } }
}

/// C-owned RNG queue execution and fallback with a host entropy adapter.
pub struct VirtioRng {
    transport: VirtioTransport,
    random: Option<File>,
    fallback_state: u64,
}

impl VirtioRng {
    pub fn new() -> Self {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64).unwrap_or(0xD1CE_BA5E_1234_5678);
        Self { transport: VirtioTransport::new(), random: File::open("/dev/urandom").ok(),
            fallback_state: seed | 1 }
    }
    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) { self.transport.attach_apic(apic) }
    pub fn set_irq_vector(&mut self, vector: u8) { self.transport.set_irq_vector(vector) }
    pub fn has_pending(&self) -> bool { self.transport.state.pending }
    pub fn poll(&mut self, mmu: &mut Mmu) {
        let mut context = PollContext { mmu: Some(mmu), disk: None, random: self.random.as_mut(),
            apic: &self.transport.apic, vector: self.transport.irq_vector };
        let io = context.io();
        assert!(unsafe { ghostos_vm_virtio_rng_poll(&mut self.transport.state, &mut self.fallback_state, &io) },
            "could not allocate Virtio RNG buffer")
    }
    #[cfg(test)]
    fn fill_random(&mut self, bytes: &mut [u8]) {
        let mut context = PollContext { mmu: None, disk: None, random: self.random.as_mut(),
            apic: &self.transport.apic, vector: self.transport.irq_vector };
        let io = context.io();
        unsafe { ghostos_vm_virtio_fill_random(&mut self.fallback_state, bytes.as_mut_ptr(), bytes.len(), &io) }
    }
    fn read_io(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        self.transport.read_io(2, port, size, 0)
    }
    fn write_io(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        self.transport.write_io(port, value, size)
    }
    pub fn reset(&mut self) { self.transport.reset() }
}

impl Default for VirtioRng { fn default() -> Self { Self::new() } }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::virtio_queue::DESC_INDIRECT;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn transport_negotiates_queue_and_resets() {
        let mut transport = VirtioTransport::new();
        assert_eq!(transport.read_common(REG_DEVICE_FEATURES, 0x55), Some(0x55));
        transport.write_common(REG_GUEST_FEATURES, 0x55);
        transport.write_common(REG_QUEUE_PFN, 2);
        transport.write_common(REG_STATUS, STATUS_DRIVER_OK as u32);
        assert_eq!(transport.state.pfn, 2);
        assert_eq!(transport.state.status, STATUS_DRIVER_OK);

        transport.write_common(REG_QUEUE_NOTIFY, 0);
        assert!(transport.take_pending());
        transport.write_common(REG_STATUS, 0);
        assert_eq!(transport.state.pfn, 0);
        transport.reset();
        assert_eq!(transport.state.status, 0);
        assert_eq!(transport.state.guest_features, 0);
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
        let path = std::env::temp_dir().join(format!("ghostos-virtio-{}", std::process::id()));
        let mut file = File::create(path.clone()).unwrap();
        file.set_len(4096).unwrap();
        file.flush().unwrap();

        let mut block = VirtioBlk::new();
        block.attach_disk(DiskImage::open(path).unwrap());
        assert_eq!(block.sector_count(), Some(8));
        assert_eq!(block.read_io(REG_CONFIG, 8).unwrap(), 8);
        assert_eq!(block.read_io(REG_CONFIG + 7, 2), Err(DeviceError::UnsupportedSize));

        let mut console = VirtioConsole::new();
        console.append_output(b"hello");
        assert_eq!(console.take_output(), b"hello");
        console.reset();
        assert!(console.output().is_empty());

        let mut rng = VirtioRng::new();
        let mut bytes = [0u8; 32];
        rng.fill_random(&mut bytes);
        assert_ne!(bytes, [0; 32]);
    }
}
