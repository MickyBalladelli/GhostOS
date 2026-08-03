//! Fast, deterministic coverage for the VM/QEMU integration matrix.
//!
//! These tests never invoke QEMU. The opt-in QEMU tier lives in
//! `qemu_matrix_59_11.rs` and is run by `scripts/test-vm-matrix.sh`.

use std::cell::RefCell;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use synos_vm::devices::{
    Device, DeviceError, DiskFormat, InterruptController, PciDeviceId, PortBus, PortDevice,
};
use synos_vm::firmware::bios::BiosContext;
use synos_vm::net::NetBackend;
use synos_vm::{
    ascii_to_scancodes, framebuffer_info, Cpu, CpuMode, DiskImage, FirmwareMode, LargePageSize,
    Loader, LoopbackHub, LoopbackPort, MacAddress, Mmu, PageFlags, PacketQueue,
    SnapshotChain, TerminalExit, TerminalInputMode, Vm, VmConfig, VmSnapshot, POWER_CONTROL_PORT,
    PAGE_SIZE,
};

fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "synos-vm-matrix-{name}-{}-{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    File::create(&path)
        .expect("create VM test fixture")
        .write_all(bytes)
        .expect("write VM test fixture");
    path
}

fn remove_fixture(path: &Path) {
    let _ = fs::remove_file(path);
}

#[test]
fn cpu_decode_execute_modes_interrupts_hlt_and_reset() {
    let mut mmu = Mmu::new(8 * 1024 * 1024);
    mmu.write_phys(0x1000, &[0x90, 0xF4]).expect("write code");
    let mut cpu = Cpu::new();
    let mut intc = InterruptController::new();
    let mut ports = PortBus::new();
    let mut bios = BiosContext::new();

    cpu.set_rip(0x1000);
    let decoded = cpu.decode_instruction(0x1000, &mmu).expect("decode NOP");
    assert_eq!(decoded.mnemonic, "NOP");
    cpu.step(&mut mmu, &mut intc, &mut ports, &mut bios)
        .expect("execute NOP");
    assert_eq!(cpu.rip(), 0x1001);
    cpu.step(&mut mmu, &mut intc, &mut ports, &mut bios)
        .expect("execute HLT");
    assert!(cpu.state.halted);
    assert!(cpu.handle_exception(32, None, &mut mmu, &mut intc).is_err());

    cpu.reset();
    assert_eq!(cpu.mode(), CpuMode::Real16);
    assert_eq!(cpu.rip(), 0xFFF0);
    assert!(!cpu.state.halted);

    cpu.enter_protected_mode(&mut mmu, 0x2000)
        .expect("enter protected mode");
    assert_eq!(cpu.mode(), CpuMode::Protected32);
    cpu.state.efer |= 1 << 8;
    cpu.enter_long_mode(&mut mmu, 0x3000)
        .expect("enter long mode");
    assert_eq!(cpu.mode(), CpuMode::Long64);
}

#[test]
fn mmu_pages_permissions_cow_balloon_and_invalid_addresses() {
    let mut mmu = Mmu::new(16 * 1024 * 1024);
    let flags = PageFlags::PRESENT | PageFlags::WRITABLE;
    let frame = mmu.alloc_frame().expect("allocate frame");
    mmu.map_page(0x4000, frame, flags).expect("map page");
    mmu.set_paging(true, mmu.cr3());
    mmu.write_byte(0x4003, 0xA5).expect("write mapped page");
    assert_eq!(mmu.read_byte(0x4003).expect("read mapped page"), 0xA5);

    mmu.map_2mb_page(0x20_0000, 0x20_0000, flags)
        .expect("map large page");
    assert!(mmu
        .map_large_page(0x21_0000, 0x20_0000, LargePageSize::TwoMiB, flags)
        .is_err());

    mmu.set_overcommit_limit(2);
    mmu.map_overcommit_page(0x8000, PageFlags::PRESENT | PageFlags::USER)
        .expect("map overcommit page");
    assert!(mmu.is_cow_page(0x8000));
    assert!(mmu.write_byte(0x8000, 0x5A).is_ok());

    let balloon_frame = mmu.alloc_frame().expect("allocate balloon frame");
    mmu.balloon_inflate(&[balloon_frame]).expect("inflate balloon");
    assert_eq!(mmu.ballooned_frames(), 1);
    mmu.balloon_deflate(&[balloon_frame]).expect("deflate balloon");
    assert_eq!(mmu.ballooned_frames(), 0);
    assert!(mmu.read_byte(u64::MAX).is_err());
}

#[derive(Default)]
struct RegisterDevice {
    value: u64,
}

impl Device for RegisterDevice {
    fn read(&self, _addr: u64, _size: u8) -> Result<u64, DeviceError> {
        Ok(self.value)
    }

    fn write(&mut self, _addr: u64, value: u64, _size: u8) -> Result<(), DeviceError> {
        self.value = value;
        Ok(())
    }

    fn reset(&mut self) {
        self.value = 0;
    }
}

#[test]
fn chipset_devices_ports_mmio_display_and_wakeup() {
    let mut mmu = Mmu::new(4 * 1024 * 1024);
    mmu.attach_mmio(0xF000, 8, Box::new(RegisterDevice::default()));
    mmu.write_to_addr(0xF000, 0xCAFE, 2).expect("MMIO write");
    assert_eq!(mmu.read_from_addr(0xF000, 2).expect("MMIO read"), 0xCAFE);

    let mut pci = synos_vm::devices::PciHostBridge::new();
    pci.add_device(
        0,
        1,
        0,
        PciDeviceId {
            vendor: 0x1234,
            device: 0x5678,
            revision: 1,
            prog_if: 0,
            subclass: 0,
            class: 2,
        },
    );
    assert_eq!(pci.enumerate().expect("PCI enumerate"), 1);
    assert_eq!(pci.read_config(0, 1, 0, 0), 0x5678_1234);

    let mut apic = synos_vm::devices::LocalApic::new(0);
    apic.signal(0x40, synos_vm::devices::ApicTrigger::Edge);
    assert_eq!(apic.pending_vector(), Some(0x40));

    let mut serial = synos_vm::devices::Serial16550::new(0x3F8);
    serial.write(0x3F8, b'X' as u64, 1).expect("serial write");
    assert_eq!(serial.output(), b"X");
    serial.push_input(b"ok");
    assert!(serial.input_pending());

    let display = Rc::new(RefCell::new(synos_vm::devices::DisplayState::new()));
    display.borrow_mut().port_write(0x3D4, 0x0E);
    display.borrow_mut().port_write(0x3D5, 0);
    assert_eq!(display.borrow().gop().modes[0].width, 640);
    let _ = framebuffer_info(&display.borrow().gop());

    let mut vm = Vm::with_config(VmConfig {
        memory_size: 4 * 1024 * 1024,
        firmware: FirmwareMode::Bios,
        ..VmConfig::default()
    });
    vm.queue_serial_input(b"help\n");
    vm.queue_keyboard_scancode(0x1E);
    assert!(vm.serial().is_some());
    assert_eq!(vm.config().smp_cores, 1);
    assert_eq!(POWER_CONTROL_PORT, 0x604);
}

#[test]
fn storage_network_and_device_reset_matrix() {
    let image_path = temp_file("raw", &vec![0; 8 * 512]);
    let mut image = DiskImage::open(&image_path).expect("open raw image");
    assert_eq!(image.format(), DiskFormat::Raw);
    let sector = [0x5A; 512];
    image.write_sector(3, &sector).expect("write sector");
    let mut readback = [0; 512];
    image.read_sector(3, &mut readback).expect("read sector");
    assert_eq!(readback, sector);

    let mut blk = synos_vm::devices::VirtioBlk::new();
    assert!(blk.attach_disk(image).is_none());
    assert_eq!(blk.sector_count(), Some(8));
    blk.reset();
    assert_eq!(blk.sector_count(), None);
    remove_fixture(&image_path);

    let hub = Rc::new(RefCell::new(LoopbackHub::new()));
    let left_mac = MacAddress::synos_default(0x60);
    let right_mac = MacAddress::synos_default(0x61);
    let mut left = LoopbackPort::new(hub.clone(), 0, left_mac);
    let mut right = LoopbackPort::new(hub, 1, right_mac);
    let mut frame = vec![0; 60];
    frame[..6].copy_from_slice(&right_mac.to_bytes());
    frame[6..12].copy_from_slice(&left_mac.to_bytes());
    left.transmit(&frame).expect("loopback transmit");
    assert_eq!(right.receive().expect("loopback receive").unwrap().len(), 60);

    let mut queue = PacketQueue::new(1, 4);
    assert!(queue.push(vec![1, 2, 3, 4]));
    assert!(!queue.push(vec![5]));
    assert_eq!(queue.pop(), Some(vec![1, 2, 3, 4]));
    assert_eq!(ascii_to_scancodes(0x03), vec![0x1E, 0x9E]);
    assert_eq!(TerminalInputMode::Serial, TerminalInputMode::Serial);
    assert_eq!(TerminalExit::GuestShutdown, TerminalExit::GuestShutdown);
}

#[test]
fn firmware_loader_boot_parameters_and_uefi_state() {
    let kernel_path = temp_file("kernel", &[0xF4, 0x90, 0xC3]);
    let mut loader = Loader::new();
    loader.load_kernel(&kernel_path).expect("load raw kernel");
    loader.set_cmdline("console=serial0 root=/dev/vda");
    assert_eq!(loader.entry_point(), synos_vm::KERNEL_LOAD_ADDR);
    assert_eq!(loader.kernel_size(), 3);

    let mut mmu = Mmu::new(8 * 1024 * 1024);
    loader
        .load_to_memory(&mut mmu, synos_vm::KERNEL_LOAD_ADDR)
        .expect("load kernel to guest RAM");
    assert_eq!(
        mmu.read_byte(synos_vm::KERNEL_LOAD_ADDR).expect("read entry"),
        0xF4
    );
    let header = [0x02, 0xB0, 0xAD, 0x1B, 0, 0, 0, 0, 0xFE, 0x4F, 0x52, 0xE4];
    assert_eq!(synos_vm::boot::multiboot_header(&header), Some(0));

    let mut cpu = Cpu::new();
    let mut intc = InterruptController::new();
    let _ = &mut intc;
    loader
        .install_boot_parameters(
            &mut mmu,
            8 * 1024 * 1024,
            synos_boot_protocol::BootMethod::Bios,
            framebuffer_info(&synos_vm::devices::DisplayState::new().gop()),
        )
        .expect("install boot parameters");
    loader.handoff(&mut cpu, &mut mmu).expect("handoff to kernel");
    assert_eq!(cpu.mode(), CpuMode::Long64);

    let mut uefi = synos_vm::UefiContext::new();
    uefi.set_memory_size(8 * 1024 * 1024);
    uefi.set_efi_application(vec![0x7F, b'E', b'L', b'F']);
    assert!(uefi.init(&mut mmu, &mut cpu.state).is_ok());
    assert!(uefi.efi_application().is_some());
    remove_fixture(&kernel_path);
}

#[test]
fn execution_snapshot_chain_diff_corruption_and_compatibility() {
    let config = VmConfig {
        memory_size: 4 * 1024 * 1024,
        ..VmConfig::default()
    };
    let mut vm = Vm::with_config(config);
    vm.mmu_mut().write_byte(0x2000, 0x11).expect("write base state");
    let base = vm.snapshot();
    vm.mmu_mut().write_byte(0x2000, 0x22).expect("write target state");
    let target = vm.snapshot();
    let diff = base.diff(&target).expect("snapshot diff");
    assert_eq!(diff.apply_to(&base).expect("apply diff").checksum(), target.checksum());

    let mut chain = SnapshotChain::new(base.clone());
    let id = chain.checkpoint(target.clone()).expect("checkpoint chain");
    assert_eq!(chain.latest_id(), id);
    assert_eq!(chain.snapshot(id).expect("materialize chain").checksum(), target.checksum());

    let mut bytes = base.to_bytes();
    bytes[0] ^= 0xFF;
    assert!(VmSnapshot::from_bytes(&bytes).is_err());
    let incompatible = Vm::with_config(VmConfig {
        memory_size: 8 * 1024 * 1024,
        ..VmConfig::default()
    });
    assert!(incompatible.mmu().ram_size() != base.memory_size);
}

#[test]
fn vm_profiles_keep_fast_and_qemu_tiers_separate() {
    let profiles = [
        (FirmwareMode::Bios, 1usize),
        (FirmwareMode::Bios, 2usize),
        (FirmwareMode::Uefi, 1usize),
        (FirmwareMode::Uefi, 2usize),
    ];
    for (firmware, cpus) in profiles {
        let vm = Vm::with_config(VmConfig {
            memory_size: 4 * 1024 * 1024,
            firmware,
            smp_cores: cpus,
            ..VmConfig::default()
        });
        assert_eq!(vm.config().firmware, firmware);
        assert_eq!(vm.config().smp_cores, cpus);
    }
    assert_eq!(PAGE_SIZE, 4096);
}
