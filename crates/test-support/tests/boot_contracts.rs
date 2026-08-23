const BIOS_STAGE1: &str = include_str!("../../../boot/bios/stage1.S");
const BIOS_STAGE2: &str = include_str!("../../../boot/bios/stage2.S");
const BIOS_BUILDER: &str = include_str!("../../../scripts/build-bios-image.sh");
const UEFI_MAIN: &str = include_str!("../../../boot/uefi/src/main.rs");
const UEFI_CHAINLOAD: &str = include_str!("../../../boot/uefi/src/chainload.rs");
const X86_PAGING: &str = include_str!("../../../kernel/src/arch/x86_64.rs");
const POWER_PATHS: &str = include_str!("../../../kernel/src/power.rs");

#[test]
fn bios_stage_one_has_a_valid_boot_sector_and_bounded_edd_read() {
    assert!(BIOS_STAGE1.contains(".org 510"));
    assert!(BIOS_STAGE1.contains(".word 0xaa55"));
    assert!(BIOS_STAGE1.contains("movb $0x42, %ah"));
    assert!(BIOS_STAGE1.contains(".word STAGE2_SECTORS"));
    assert!(BIOS_STAGE1.contains(".quad 1"));
}

#[test]
fn bios_stage_two_checks_kernel_size_chunks_reads_and_handoff() {
    assert!(BIOS_STAGE2.contains(".set MAX_KERNEL_SECTORS, 1196"));
    assert!(BIOS_STAGE2.contains(".if KERNEL_SECTORS > MAX_KERNEL_SECTORS"));
    assert!(BIOS_STAGE2.contains("cmpw $127, %ax"));
    assert!(BIOS_STAGE2.contains("movq $BOOT_INFO, %rdi"));
    assert!(BIOS_STAGE2.contains("jmp *%rax"));
    assert!(BIOS_BUILDER.contains("max_kernel_sectors=1196"));
    assert!(BIOS_BUILDER.contains("kernel exceeds its BIOS staging-memory limit"));
    assert!(BIOS_BUILDER.contains("stage2 exceeds its 16-sector reservation"));
}

#[test]
fn uefi_loader_contract_covers_map_handoff_framebuffer_and_chainload_failures() {
    assert!(UEFI_MAIN.contains("const MEMORY_MAP_CAPACITY: usize = 32 * 1024"));
    assert!(UEFI_MAIN.contains("ExitBootServices failed"));
    assert!(UEFI_MAIN.contains("Memory map changed. Retrying handoff"));
    assert!(UEFI_MAIN.contains("FRAMEBUFFER_PIXEL_RGB"));
    assert!(UEFI_MAIN.contains("FRAMEBUFFER_PIXEL_BGR"));
    assert!(UEFI_MAIN.contains("fn memory_kind(efi_type: u32)"));
    assert!(UEFI_CHAINLOAD.contains("DEVICE_PATH_BUFFER_SIZE"));
    assert!(UEFI_CHAINLOAD.contains("node_length > u16::MAX"));
    assert!(UEFI_CHAINLOAD.contains("unload_image"));
}

#[test]
fn x86_paging_contract_covers_root_creation_permissions_and_large_pages() {
    assert!(X86_PAGING.contains("pub const TABLE_FRAME_COUNT: usize = 6"));
    assert!(X86_PAGING.contains("const HUGE_PAGE: u64 = 1 << 7"));
    assert!(X86_PAGING.contains("supports_one_gib_pages"));
    assert!(X86_PAGING.contains("mov cr3"));
}

#[test]
fn kernel_failure_paths_report_panic_fault_and_power_recovery_actions() {
    assert!(POWER_PATHS.contains("pub fn shutdown(platform: Option<&AcpiPlatform>) -> !"));
    assert!(POWER_PATHS.contains("PowerState::SoftOff"));
    assert!(POWER_PATHS.contains("PowerState::Reboot"));
    assert!(POWER_PATHS.contains("VM_SOFT_OFF_VALUE"));
    assert!(POWER_PATHS.contains("VM_REBOOT_VALUE"));
    assert!(include_str!("../../../kernel/src/lib.rs").contains("KERNEL PANIC"));
    assert!(include_str!("../../../kernel/src/arch/x86_64.rs").contains("vector == 14"));
}
