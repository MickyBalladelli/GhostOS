use ghostos_boot_protocol::{
    BOOT_INFO_MAGIC, BOOT_INFO_VERSION, BootInfo, BootMethod, FramebufferInfo, MemoryKind,
    MemoryRegion, FRAMEBUFFER_PIXEL_RGB,
};

#[test]
fn boot_handoff_rejects_bad_magic_version_and_memory_order() {
    let mut info = BootInfo::empty(BootMethod::Uefi);
    assert!(info.is_valid());
    info.memory_region_count = 1;
    info.memory_regions[0] = MemoryRegion { start: 0x1000, length: 0x1000, kind: MemoryKind::Usable, attributes: 0 };
    assert!(info.is_valid());
    info.magic = BOOT_INFO_MAGIC ^ 1;
    assert!(!info.is_valid());
    info.magic = BOOT_INFO_MAGIC;
    info.version = BOOT_INFO_VERSION + 1;
    assert!(!info.is_valid());
    info.version = BOOT_INFO_VERSION;
    info.memory_regions[0].start = 0;
    assert!(!info.is_valid());
}

#[test]
fn boot_framebuffer_contract_rejects_downgrade_geometry() {
    let mut info = BootInfo::empty(BootMethod::Bios);
    info.framebuffer = FramebufferInfo {
        address: 0x1000,
        size: 4096,
        width: 32,
        height: 32,
        stride: 32,
        pixel_format: FRAMEBUFFER_PIXEL_RGB,
    };
    assert!(info.is_valid());
    info.framebuffer.stride = 31;
    assert!(!info.is_valid());
}
