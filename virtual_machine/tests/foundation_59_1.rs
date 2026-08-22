//! Test foundation for the VM.
//!
//! Keep this target fast and deterministic. Process, QEMU, cluster, and
//! hardware work belongs to the opt-in targets named in `inventory.toml`.

use std::cell::RefCell;
use std::rc::Rc;

use ghostos_test_support::{
    golden_bytes, golden_text, FakeDevice, FakeDeviceError, FailureInjector, Fault, FaultPlan,
    FaultPoint, GoldenFixture, TestScope,
};
use ghostos_vm::devices::{ApicTrigger, Device, DeviceError, PortDevice};
use ghostos_vm::{FirmwareMode, PageFlags, Vm, VmConfig, COM1_PORT};

#[derive(Clone, Debug)]
struct MmioFake {
    device: Rc<RefCell<FakeDevice>>,
}

impl Device for MmioFake {
    fn read(&self, address: u64, _size: u8) -> Result<u64, DeviceError> {
        self.device
            .borrow_mut()
            .read_register(address)
            .map_err(|_| DeviceError::NotReady)
    }

    fn write(&mut self, address: u64, value: u64, _size: u8) -> Result<(), DeviceError> {
        self.device
            .borrow_mut()
            .write_register(address, value)
            .map_err(|_| DeviceError::NotReady)
    }

    fn reset(&mut self) {
        self.device.borrow_mut().reset()
    }
}

fn fixture_vm() -> (TestScope, Vm) {
    let scope = TestScope::new(0x5359_4e4f_5300_0001);
    let vm = Vm::with_config(VmConfig {
        memory_size: 4 * 1024 * 1024,
        firmware: FirmwareMode::Bios,
        smp_cores: 1,
        ..VmConfig::default()
    });
    (scope, vm)
}

#[test]
fn deterministic_vm_fixture_covers_state_memory_devices_and_input() {
    let (scope, mut vm) = fixture_vm();
    let same_scope = TestScope::new(scope.fixtures.context.seed());
    assert_eq!(
        scope.fixtures.packet.encode(),
        same_scope.fixtures.packet.encode()
    );
    assert_eq!(scope.fixtures.boot.magic, *b"SYNBOOT\0");
    assert_eq!(scope.fixtures.terminal.bytes[0], b'D');
    assert_eq!(scope.fixtures.context.clock().now_us(), 1_700_000_000_000_000);
    let mut disk = scope.fixtures.disk.clone();
    disk.fill_pattern();
    let mut sector = [0; 512];
    disk.read_sector(1, &mut sector).expect("fixture disk read");
    assert_eq!(sector, [17; 512]);

    vm.cpu_mut().state.halted = true;
    vm.cpu_mut().set_rip(0x1000);
    let frame = vm.mmu_mut().alloc_frame().expect("fixture frame");
    vm.mmu_mut()
        .map_page(
            0x4000,
            frame,
            PageFlags::PRESENT | PageFlags::WRITABLE,
        )
        .expect("fixture page table");
    let fake = Rc::new(RefCell::new(FakeDevice::new()));
    vm.mmu_mut().attach_mmio(
        0xf000,
        8,
        Box::new(MmioFake {
            device: fake.clone(),
        }),
    );
    vm.mmu_mut()
        .write_to_addr(0xf000, 0xcafe, 2)
        .expect("fixture MMIO write");
    assert_eq!(
        vm.mmu().read_from_addr(0xf000, 2).expect("fixture MMIO read"),
        0xcafe
    );

    let cr3 = vm.mmu().cr3();
    vm.mmu_mut().set_paging(true, cr3);
    vm.mmu_mut().write_byte(0x4000, 0x5a).expect("fixture memory");
    assert_eq!(vm.mmu().read_byte(0x4000).expect("read fixture memory"), 0x5a);

    vm.queue_serial_input(b"help\n");
    assert!(vm.serial().expect("serial fixture").borrow().input_pending());
    assert!(!vm.cpu().state.halted);
    vm.queue_keyboard_scancode(0x1e);
    assert!(vm.pci().borrow().device_count() > 0);
    vm.apic().borrow_mut().signal(0x40, ApicTrigger::Edge);
    assert_eq!(vm.apic().borrow_mut().pending_vector(), Some(0x40));
    assert_eq!(vm.config().smp_cores, 1);
    assert_eq!(vm.snapshot().memory_size, 4 * 1024 * 1024);
}

#[test]
fn fake_device_covers_short_io_dma_descriptors_interrupts_reset_and_removal() {
    let mut device = FakeDevice::new();
    device.write_register(0x10, 0x1234).expect("fake register write");
    assert_eq!(device.read_register(0x10).expect("fake register read"), 0x1234);

    device
        .faults_mut()
        .push(FaultPoint::DeviceRead, Fault::ShortIo { bytes: 2 });
    assert_eq!(
        device.read_register(0x10),
        Err(FakeDeviceError::ShortIo { bytes: 2 })
    );

    device
        .faults_mut()
        .push(FaultPoint::DeviceDma, Fault::DmaOverrun { bytes: 4 });
    assert_eq!(
        device.dma(1),
        Err(FakeDeviceError::DmaOverrun { bytes: 4 })
    );

    device
        .faults_mut()
        .push(FaultPoint::DeviceDescriptor, Fault::InvalidDescriptor);
    assert_eq!(device.dma(1), Err(FakeDeviceError::InvalidDescriptor));
    assert_eq!(device.dma(2), Ok(2));

    device
        .faults_mut()
        .push(FaultPoint::DeviceReset, Fault::ResetDuringIo);
    assert_eq!(device.finish_io(), Err(FakeDeviceError::ResetDuringIo));
    assert!(!device.io_in_progress());

    device
        .faults_mut()
        .push(FaultPoint::DeviceInterrupt, Fault::DroppedInterrupt);
    assert_eq!(
        device.raise_interrupt(),
        Err(FakeDeviceError::InterruptDropped)
    );
    assert!(!device.take_interrupt());
    device.raise_interrupt().expect("fake interrupt");
    assert!(device.take_interrupt());

    device
        .faults_mut()
        .push(FaultPoint::DeviceRemoval, Fault::DeviceRemoved);
    assert_eq!(device.read_register(0), Err(FakeDeviceError::Removed));
    assert!(device.is_removed());
    device.reset();
    assert_eq!(device.read_register(0), Err(FakeDeviceError::Removed));
}

#[test]
fn delayed_interrupt_faults_are_replayable() {
    let mut injector = FailureInjector::new(FaultPlan::once(
        FaultPoint::Interrupt,
        Fault::DelayedInterrupt { ticks: 3 },
    ));
    assert_eq!(injector.interrupt_delay(), Ok(3));
    assert_eq!(injector.interrupt_delay(), Ok(0));
}

#[test]
fn cleanup_and_vm_golden_fixtures_are_registered() {
    let mut scope = TestScope::default();
    let order = Rc::new(RefCell::new(Vec::new()));
    for label in ["disk", "serial", "terminal"] {
        let order = order.clone();
        scope.cleanup.defer(label, move || {
            order.borrow_mut().push(label);
            Ok(())
        });
    }
    scope.cleanup.cleanup().expect("fixture cleanup");
    assert_eq!(&*order.borrow(), &["terminal", "serial", "disk"]);

    for fixture in [
        GoldenFixture::VmDecodedInstruction,
        GoldenFixture::VmFirmwareTables,
        GoldenFixture::VmBootHandoff,
        GoldenFixture::VmDeviceRegisters,
        GoldenFixture::VmSnapshot,
    ] {
        assert!(!golden_bytes(fixture).expect("VM hex golden is valid").is_empty());
    }
    assert!(golden_text(GoldenFixture::VmSerialOutput).contains("serial"));
    assert!(golden_text(GoldenFixture::TerminalOutput).contains("GhostOS"));

    let inventory = include_str!("inventory.toml");
    for tier in [
        "fast-unit",
        "vm-integration",
        "cli",
        "qemu",
        "cluster",
        "performance",
        "hardware-accelerated",
    ] {
        assert!(inventory.contains(&format!("name = \"{tier}\"")));
    }
    assert!(inventory.contains("src/devices/storage/nvme.rs"));
    assert!(inventory.contains("src/terminal.rs"));
}

#[test]
fn serial_fixture_uses_the_public_port_device_boundary() {
    let (_, vm) = fixture_vm();
    let serial = vm.serial().expect("serial fixture");
    serial
        .borrow_mut()
        .write(COM1_PORT, b'X' as u64, 1)
        .expect("serial write");
    assert_eq!(serial.borrow().output(), b"X");
}
