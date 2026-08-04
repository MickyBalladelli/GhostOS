//! Deterministic coverage for the firmware, boot, and SynOS bring-up plan.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use synos_boot_protocol::FramebufferInfo;
use synos_synfs::SynFs;
use synos_vm::boot::{BOOT_INFO_ADDR, MULTIBOOT_INFO_ADDR};
use synos_vm::devices::{
    SystemDiskInstall, SystemDiskProvisioner, SYNFS_SYSTEM_BLOCKS,
};
use synos_vm::firmware::bios::{Bios, BiosContext, BiosState, MBR_LOAD_ADDR};
use synos_vm::firmware::uefi::{
    UefiContext, UefiState, EFI_BUFFER_TOO_SMALL, EFI_INVALID_PARAMETER,
    EFI_SUCCESS, EFI_UNSUPPORTED, UEFI_IMAGE_BASE, UEFI_MEMORY_MAP_BASE, UEFI_TABLES_BASE,
};
use synos_vm::{
    CpuMode, CpuState, FirmwareMode, Loader, LoaderError, Mmu, Vm, VmConfig, COM1_PORT,
    KERNEL_LOAD_ADDR,
};

fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "synos-vm-10-4-{name}-{}-{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    File::create(&path)
        .expect("create firmware fixture")
        .write_all(bytes)
        .expect("write firmware fixture");
    path
}

fn remove_fixture(path: &Path) {
    let _ = fs::remove_file(path);
}

fn serial_kernel() -> Vec<u8> {
    let mut code = Vec::new();
    for byte in b"SynOS kernel bootstrap\nsynos> " {
        code.extend_from_slice(&[0xBA, 0xF8, 0x03, 0x00, 0x00, 0xB0, *byte, 0xEE]);
    }
    code.push(0xF4);
    code
}

fn valid_pe() -> Vec<u8> {
    let mut image = vec![0u8; 0x401];
    image[0..2].copy_from_slice(b"MZ");
    image[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    image[0x80..0x84].copy_from_slice(b"PE\0\0");

    let coff = 0x84;
    image[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
    image[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    image[coff + 16..coff + 18].copy_from_slice(&0xF0u16.to_le_bytes());

    let optional = coff + 20;
    image[optional..optional + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
    image[optional + 16..optional + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    image[optional + 24..optional + 32].copy_from_slice(&0x0200_0000u64.to_le_bytes());
    image[optional + 56..optional + 60].copy_from_slice(&0x2000u32.to_le_bytes());
    image[optional + 60..optional + 64].copy_from_slice(&0x400u32.to_le_bytes());

    let section = optional + 0xF0;
    image[section..section + 8].copy_from_slice(b".text\0\0\0");
    image[section + 12..section + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    image[section + 16..section + 20].copy_from_slice(&1u32.to_le_bytes());
    image[section + 20..section + 24].copy_from_slice(&0x400u32.to_le_bytes());
    image[0x400] = 0xF4;
    image
}

fn invalid_entry_elf() -> Vec<u8> {
    let mut image = vec![0u8; 0x101];
    image[0..4].copy_from_slice(b"\x7FELF");
    image[4] = 2;
    image[0x18..0x20].copy_from_slice(&0x200000u64.to_le_bytes());
    image[0x20..0x28].copy_from_slice(&0x40u64.to_le_bytes());
    image[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
    image[0x38..0x3A].copy_from_slice(&1u16.to_le_bytes());
    let ph = 0x40;
    image[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes());
    image[ph + 8..ph + 16].copy_from_slice(&0x100u64.to_le_bytes());
    image[ph + 16..ph + 24].copy_from_slice(&KERNEL_LOAD_ADDR.to_le_bytes());
    image[ph + 32..ph + 40].copy_from_slice(&1u64.to_le_bytes());
    image[ph + 40..ph + 48].copy_from_slice(&1u64.to_le_bytes());
    image[0x100] = 0xF4;
    image
}

fn write_stack_arg(mmu: &mut Mmu, rsp: u64, index: u64, value: u64) {
    mmu.write_u64(rsp + 0x28 + (index - 5) * 8, value)
        .expect("write UEFI stack argument");
}

#[test]
fn bios_post_services_bad_media_and_mbr_handoff() {
    let mut bios = Bios::new();
    let mut mmu = Mmu::new(8 * 1024 * 1024);
    let mut cpu = CpuState::default();
    bios.post(&mut mmu, &mut cpu).expect("BIOS POST");
    assert_eq!(bios.context.state, BiosState::Initialized);
    assert_eq!(mmu.read_phys(0xFFFFE, 2).expect("BIOS signature"), [0x55, 0xAA]);
    assert_eq!(mmu.read_phys(0x410, 1).expect("BDA equipment"), [0x21]);

    bios.init(&mut mmu, &mut cpu).expect("BIOS init");
    assert_eq!(cpu.mode, CpuMode::Real16);
    assert_eq!(cpu.rip, synos_vm::firmware::bios::BIOS_ENTRY_LINEAR);

    cpu.rax = 0x0E58;
    cpu.rbx = 0x0700;
    bios.context
        .call_int(0x10, &mut cpu, &mut mmu)
        .expect("INT 10h teletype");
    assert_eq!(bios.context.display().borrow().text_char(0, 0), Some(b'X'));

    cpu.rax = 0xE820;
    cpu.rdx = 0x534D_4150;
    cpu.es.selector = 0x1000;
    cpu.rdi = 0x0200;
    cpu.rcx = 24;
    cpu.rbx = 0;
    bios.context
        .call_int(0x15, &mut cpu, &mut mmu)
        .expect("INT 15h E820");
    assert_eq!(mmu.read_u64(0x10200).expect("E820 base"), 0);
    assert_eq!(cpu.rflags & 1, 0);

    let mut image = vec![0u8; 3 * 512];
    image[512..1024].fill(0xA5);
    bios.context.set_boot_image(image);
    cpu.rax = 0x0201;
    cpu.es.selector = 0x1000;
    cpu.rbx = 0;
    cpu.rcx = 2;
    cpu.rdx = 0;
    bios.context
        .call_int(0x13, &mut cpu, &mut mmu)
        .expect("INT 13h read");
    assert_eq!(cpu.rflags & 1, 0);
    assert_eq!(mmu.read_phys(0x10000, 512).expect("read sector"), [0xA5; 512]);

    cpu.rax = 0x0201;
    cpu.rcx = 0x0004;
    bios.context
        .call_int(0x13, &mut cpu, &mut mmu)
        .expect("INT 13h bad-sector response");
    assert_ne!(cpu.rflags & 1, 0);
    assert_eq!((cpu.rax >> 8) as u8, 0x02);

    bios.context.set_boot_image(vec![0x90; 511]);
    cpu.halted = false;
    bios.context
        .call_int(0x19, &mut cpu, &mut mmu)
        .expect("INT 19h missing boot code");
    assert!(cpu.halted);

    let mut handoff = BiosContext::new();
    let mut handoff_cpu = CpuState::default();
    handoff.set_boot_image(vec![0xCC; 512]);
    handoff
        .call_int(0x19, &mut handoff_cpu, &mut mmu)
        .expect("INT 19h MBR handoff");
    assert_eq!(handoff_cpu.rip, MBR_LOAD_ADDR);
    assert_eq!(mmu.read_phys(MBR_LOAD_ADDR, 512).expect("loaded MBR"), [0xCC; 512]);

    let mut loader = Loader::new();
    loader.load_kernel_bytes(vec![0xF4]).expect("tiny kernel");
    let mut tiny_mmu = Mmu::new(1024 * 1024);
    assert!(matches!(
        loader.load_to_memory(&mut tiny_mmu, KERNEL_LOAD_ADDR),
        Err(LoaderError::LoadFailed)
    ));
}

#[test]
fn uefi_tables_services_variables_images_and_failures() {
    let mut uefi = UefiContext::new();
    uefi.set_memory_size(64 * 1024 * 1024);
    uefi.set_efi_application(valid_pe());
    let mut mmu = Mmu::new(64 * 1024 * 1024);
    let mut cpu = CpuState::default();
    uefi.init(&mut mmu, &mut cpu).expect("UEFI init");
    assert_eq!(uefi.state, UefiState::Running);
    assert_eq!(cpu.mode, CpuMode::Long64);
    assert_eq!(cpu.rip, UEFI_IMAGE_BASE + 0x1000);
    assert_eq!(mmu.read_u64(UEFI_TABLES_BASE).expect("system table signature"), 0x5459_5353_2049_4249);
    assert_eq!(mmu.read_u64(UEFI_TABLES_BASE + 0x200).expect("boot services signature"), 0x4259_5353_2049_4249);

    let map_size = 0x3000;
    let map_key = 0x3008;
    let desc_size = 0x3010;
    let desc_version = 0x3018;
    let stack = 0x4000;
    cpu.rsp = stack;
    mmu.write_u64(map_size, 0).expect("zero memory map capacity");
    cpu.rax = 5;
    cpu.rcx = map_size;
    cpu.rdx = UEFI_MEMORY_MAP_BASE;
    cpu.r8 = map_key;
    cpu.r9 = desc_size;
    write_stack_arg(&mut mmu, stack, 5, desc_version);
    uefi.dispatch(&mut cpu, &mut mmu);
    assert_eq!(cpu.rax, EFI_BUFFER_TOO_SMALL);
    let required = mmu.read_u64(map_size).expect("required memory map size");
    assert!(required > 0);

    mmu.write_u64(map_size, required).expect("set memory map capacity");
    cpu.rax = 5;
    uefi.dispatch(&mut cpu, &mut mmu);
    assert_eq!(cpu.rax, EFI_SUCCESS);
    assert_eq!(mmu.read_u64(desc_size).expect("descriptor size"), 48);
    assert_eq!(mmu.read_u32(desc_version).expect("descriptor version"), 1);
    let key = mmu.read_u64(map_key).expect("memory map key");
    assert!(key > 0);

    let name = 0x7000;
    mmu.write_u16(name, u16::from(b'T')).expect("variable name");
    mmu.write_u16(name + 2, 0).expect("variable name terminator");
    let guid = 0x7100;
    mmu.write_phys(guid, &[0x11; 16]).expect("variable GUID");
    let data = 0x7200;
    mmu.write_phys(data, b"ok").expect("variable data");
    write_stack_arg(&mut mmu, stack, 5, 2);
    write_stack_arg(&mut mmu, stack, 6, data);
    cpu.rax = 15;
    cpu.rcx = name;
    cpu.rdx = guid;
    cpu.r8 = 7;
    cpu.r9 = 0;
    uefi.dispatch(&mut cpu, &mut mmu);
    assert_eq!(cpu.rax, EFI_SUCCESS);

    let size = 0x7300;
    let output = 0x7400;
    mmu.write_u64(size, 2).expect("variable output size");
    write_stack_arg(&mut mmu, stack, 5, output);
    cpu.rax = 14;
    cpu.rcx = name;
    cpu.rdx = guid;
    cpu.r8 = 0x7500;
    cpu.r9 = size;
    uefi.dispatch(&mut cpu, &mut mmu);
    assert_eq!(cpu.rax, EFI_SUCCESS);
    assert_eq!(mmu.read_phys(output, 2).expect("variable output"), b"ok");

    cpu.rax = 99;
    uefi.dispatch(&mut cpu, &mut mmu);
    assert_eq!(cpu.rax, EFI_UNSUPPORTED);
    cpu.rax = 14;
    cpu.rdx = 0;
    uefi.dispatch(&mut cpu, &mut mmu);
    assert_eq!(cpu.rax, EFI_INVALID_PARAMETER);

    let handle = UEFI_TABLES_BASE + 0xC00;
    cpu.rax = 12;
    cpu.rcx = handle;
    uefi.dispatch(&mut cpu, &mut mmu);
    assert_eq!(cpu.rax, EFI_SUCCESS);
    assert_eq!(cpu.rip, UEFI_IMAGE_BASE + 0x1000);

    cpu.rax = 6;
    cpu.rcx = handle;
    cpu.rdx = key;
    uefi.dispatch(&mut cpu, &mut mmu);
    assert_eq!(cpu.rax, EFI_SUCCESS);
    assert_eq!(uefi.state, UefiState::ExitBootServices);
    assert!(!uefi.is_boot_services_active());
    cpu.rax = 5;
    uefi.dispatch(&mut cpu, &mut mmu);
    assert_eq!(cpu.rax, EFI_UNSUPPORTED);

    let mut invalid = UefiContext::new();
    invalid.set_memory_size(32 * 1024 * 1024);
    invalid.set_efi_application(vec![0x4D, 0x5A]);
    let mut invalid_mmu = Mmu::new(32 * 1024 * 1024);
    let mut invalid_cpu = CpuState::default();
    assert!(matches!(
        invalid.init(&mut invalid_mmu, &mut invalid_cpu),
        Err(synos_vm::UefiError::InvalidImage)
    ));
}

#[test]
fn multiboot_boot_info_and_loader_reject_bad_layouts() {
    let mut loader = Loader::new();
    loader
        .load_kernel_bytes(vec![0xF4, 0x90, 0xC3])
        .expect("raw kernel");
    loader.load_initrd_bytes(vec![0xA5; 4096]).expect("initrd");
    loader.set_cmdline("console=serial0 root=/dev/vda");
    let mut mmu = Mmu::new(16 * 1024 * 1024);
    loader
        .load_to_memory(&mut mmu, KERNEL_LOAD_ADDR)
        .expect("load kernel and initrd");
    let framebuffer = FramebufferInfo {
        address: 0xF000_0000,
        size: 640 * 480 * 4,
        width: 640,
        height: 480,
        stride: 640,
        pixel_format: 1,
    };
    loader
        .install_boot_parameters(
            &mut mmu,
            16 * 1024 * 1024,
            synos_boot_protocol::BootMethod::Bios,
            framebuffer,
        )
        .expect("boot parameters");

    assert_eq!(mmu.read_u64(BOOT_INFO_ADDR).expect("boot info magic"), synos_vm::boot::boot_info_magic());
    assert_eq!(mmu.read_u32(BOOT_INFO_ADDR + 8).expect("boot info version"), synos_vm::boot::boot_info_version());
    assert_eq!(mmu.read_u32(BOOT_INFO_ADDR + 12).expect("boot method"), 1);
    assert_eq!(mmu.read_u64(BOOT_INFO_ADDR + 32).expect("framebuffer address"), framebuffer.address);
    assert_eq!(mmu.read_u64(loader.initrd_address).expect("initrd contents"), 0xA5A5_A5A5_A5A5_A5A5);
    assert_eq!(
        mmu.read_phys(0x8000, b"console=serial0 root=/dev/vda\0".len())
            .expect("cmdline"),
        b"console=serial0 root=/dev/vda\0"
    );
    assert_eq!(mmu.read_u32(MULTIBOOT_INFO_ADDR).expect("multiboot flags") & 1, 1);
    assert_eq!(mmu.read_u32(MULTIBOOT_INFO_ADDR + 20).expect("module count"), 1);
    assert_eq!(mmu.read_u32(MULTIBOOT_INFO_ADDR + 24).expect("module pointer"), 0x9080);

    let mut overlap = Loader::new();
    overlap.load_kernel_bytes(vec![0xF4]).expect("overlap kernel");
    let mut overlap_mmu = Mmu::new(4 * 1024 * 1024);
    assert!(matches!(
        overlap.load_to_memory(&mut overlap_mmu, BOOT_INFO_ADDR),
        Err(LoaderError::MemoryOverlap)
    ));

    let mut invalid_entry = Loader::new();
    assert!(matches!(
        invalid_entry.load_kernel_bytes(invalid_entry_elf()),
        Err(LoaderError::InvalidFormat)
    ));
}

#[test]
fn vm_defaults_bounded_boot_and_power_lifecycle() {
    let defaults = Vm::new();
    assert_eq!(defaults.config().memory_size, 128 * 1024 * 1024);
    assert_eq!(defaults.config().smp_cores, 1);
    assert!(defaults.config().enable_serial);
    assert_eq!(defaults.config().serial_port, COM1_PORT);
    assert_eq!(defaults.config().firmware, FirmwareMode::Bios);

    let kernel_path = temp_file("vm-config", &[0xF4]);
    let custom = VmConfig {
        memory_size: 8 * 1024 * 1024,
        max_memory_size: 4 * 1024 * 1024,
        kernel_path: Some(kernel_path.clone()),
        boot_args: "console=serial0 test=1".to_string(),
        smp_cores: 2,
        enable_serial: false,
        serial_port: 0x2F8,
        firmware: FirmwareMode::Bios,
        max_steps: Some(1),
        ..VmConfig::default()
    };
    let mut vm = Vm::with_config(custom);
    assert_eq!(vm.config().max_memory_size, vm.config().memory_size);
    assert_eq!(vm.config().smp_cores, 2);
    assert!(vm.serial().is_none());
    let report = vm.run_for_steps(8).expect("bounded VM boot");
    assert_eq!(report.steps, 1);
    assert!(report.halted);

    let mut shutdown_vm = Vm::with_config(VmConfig {
        memory_size: 8 * 1024 * 1024,
        kernel_path: Some(kernel_path.clone()),
        ..VmConfig::default()
    });
    let mut monitor_calls = 0;
    shutdown_vm
        .run_with_monitor(|vm| {
            monitor_calls += 1;
            vm.request_shutdown();
            vm.cpu_mut().state.rflags &= !(1 << 9);
            Ok(true)
        })
        .expect("guest shutdown");
    assert_eq!(monitor_calls, 1);

    let mut reboot_vm = Vm::with_config(VmConfig {
        memory_size: 8 * 1024 * 1024,
        kernel_path: Some(kernel_path.clone()),
        ..VmConfig::default()
    });
    let mut reboot_calls = 0;
    reboot_vm
        .run_with_monitor(|vm| {
            reboot_calls += 1;
            if reboot_calls == 1 {
                vm.request_reboot();
            } else {
                vm.request_shutdown();
            }
            vm.cpu_mut().state.rflags &= !(1 << 9);
            Ok(true)
        })
        .expect("guest reboot and shutdown");
    assert!(reboot_calls >= 2);
    remove_fixture(&kernel_path);
}

#[test]
fn minimal_synos_boot_reports_scheduler_capabilities_ipc_and_synfs_io() {
    let kernel_path = temp_file("synos-bootstrap", &serial_kernel());
    let report = synos_vm::run_synos_integration(
        VmConfig {
            memory_size: 8 * 1024 * 1024,
            kernel_path: Some(kernel_path.clone()),
            boot_args: "console=serial0".to_string(),
            smp_cores: 1,
            ..VmConfig::default()
        },
        256,
    )
    .expect("minimal SynOS integration");
    assert!(report.kernel_booted);
    assert!(report.paging_ready);
    assert!(report.scheduler_ready);
    assert!(report.capabilities_ready);
    assert!(report.ipc_ready);
    assert!(report.boot_output.contains("SynOS kernel bootstrap"));
    assert!(report.boot_output.contains("synos> "));
    assert!(report.vm.halted);

    let mut fs = SynFs::<SYNFS_SYSTEM_BLOCKS>::new();
    fs.create_directory("/state", true).expect("SynFS mount");
    fs.write("/state/boot", b"ready").expect("SynFS write");
    let mut contents = [0u8; 5];
    let result = fs.read("/state/boot", &mut contents).expect("SynFS read");
    assert_eq!(result.bytes_read, 5);
    assert_eq!(&contents, b"ready");

    let disk_path = temp_file("system-disk", &[]);
    remove_fixture(&disk_path);
    let install = SystemDiskInstall::new(kernel_path.clone())
        .with_boot_args("console=serial0")
        .with_machine_identity("test-machine")
        .with_setting("shell.default", "/state");
    SystemDiskProvisioner::provision(&disk_path, &install).expect("provision SynOS disk");
    let artifacts = SystemDiskProvisioner::load_boot_artifacts(&disk_path).expect("load SynOS disk");
    assert_eq!(artifacts.kernel, serial_kernel());
    assert_eq!(artifacts.setting("shell.default"), Some("/state"));
    assert!(!artifacts.system_volume.is_empty());
    remove_fixture(&disk_path);
    remove_fixture(&kernel_path);
}

#[test]
fn bios_and_uefi_boot_one_and_two_cpu_configs_have_serial_evidence() {
    let kernel_path = temp_file("firmware-boot", &serial_kernel());
    for firmware in [FirmwareMode::Bios, FirmwareMode::Uefi] {
        for smp_cores in [1, 2] {
            let mut vm = Vm::with_config(VmConfig {
                memory_size: 32 * 1024 * 1024,
                kernel_path: Some(kernel_path.clone()),
                boot_args: "console=serial0".to_string(),
                smp_cores,
                firmware,
                ..VmConfig::default()
            });
            let report = vm.run_for_steps(256).expect("bounded firmware boot");
            assert!(report.steps > 0);
            assert!(report.halted);
            assert_eq!(vm.config().smp_cores, smp_cores);
            let output = vm
                .serial()
                .map(|serial| String::from_utf8_lossy(serial.borrow().output()).into_owned())
                .unwrap_or_default();
            assert!(output.contains("SynOS kernel bootstrap"));
            if firmware == FirmwareMode::Uefi {
                assert_eq!(vm.uefi().map(|uefi| uefi.state), Some(UefiState::Initialized));
            }
        }
    }
    remove_fixture(&kernel_path);
}
