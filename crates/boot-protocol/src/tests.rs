use super::{
    BOOT_INFO_MAGIC, BOOT_INFO_VERSION, BootInfo, BootMethod, FramebufferInfo,
    MAX_MEMORY_REGIONS, MemoryKind, MemoryRegion, FRAMEBUFFER_PIXEL_BGR, FRAMEBUFFER_PIXEL_RGB,
};

fn region(start: u64, length: u64) -> MemoryRegion {
    MemoryRegion {
        start,
        length,
        kind: MemoryKind::Usable,
        attributes: 0,
    }
}

fn valid_boot_info() -> BootInfo {
    let mut info = BootInfo::empty(BootMethod::Uefi);
    assert!(info.push_region(region(0x1000, 0x2000)));
    info
}

#[test]
fn empty_boot_info_has_valid_contract() {
    let info = BootInfo::empty(BootMethod::Uefi);

    assert_eq!(info.magic, BOOT_INFO_MAGIC);
    assert_eq!(info.version, BOOT_INFO_VERSION);
    assert!(info.is_valid());
    assert!(info.regions().is_empty());
}

#[test]
fn memory_regions_keep_insertion_order() {
    let mut info = BootInfo::empty(BootMethod::Bios);

    assert!(info.push_region(region(0x1000, 0x2000)));
    assert!(info.push_region(region(0x8000, 0x1000)));

    assert_eq!(info.regions().len(), 2);
    assert_eq!(info.regions()[0].start, 0x1000);
    assert_eq!(info.regions()[1].start, 0x8000);
}

#[test]
fn memory_region_capacity_is_bounded() {
    let mut info = BootInfo::empty(BootMethod::Bios);

    for index in 0..MAX_MEMORY_REGIONS {
        assert!(info.push_region(region((index as u64 + 1) * 0x1000, 0x1000)));
    }

    assert!(!info.push_region(region(0, 0x1000)));
    assert_eq!(info.regions().len(), MAX_MEMORY_REGIONS);
}

#[test]
fn memory_region_end_saturates() {
    assert_eq!(region(u64::MAX - 1, 8).end(), u64::MAX);
}

#[test]
fn raw_enums_reject_unknown_values() {
    assert_eq!(BootMethod::from_raw(1), Some(BootMethod::Bios));
    assert_eq!(BootMethod::from_raw(2), Some(BootMethod::Uefi));
    assert_eq!(BootMethod::from_raw(0), None);
    assert_eq!(BootMethod::from_raw(u32::MAX), None);

    assert_eq!(MemoryKind::from_raw(MemoryKind::Framebuffer as u32), Some(MemoryKind::Framebuffer));
    assert_eq!(MemoryKind::from_raw(0), None);
    assert_eq!(MemoryKind::from_raw(8), None);
}

#[test]
fn boot_info_rejects_malformed_and_overlapping_regions() {
    let mut info = BootInfo::empty(BootMethod::Bios);
    assert!(!info.push_region(MemoryRegion {
        start: 0x1000,
        length: 0,
        kind: MemoryKind::Usable,
        attributes: 0,
    }));
    assert!(info.push_region(region(0x1000, 0x2000)));
    assert!(!info.push_region(region(0x2000, 0x1000)));

    let mut malformed = BootInfo::empty(BootMethod::Uefi);
    malformed.memory_region_count = 1;
    malformed.memory_regions[0] = MemoryRegion {
        start: u64::MAX - 1,
        length: 8,
        kind: MemoryKind::Reserved,
        attributes: 0,
    };
    assert!(!malformed.is_valid());
}

#[test]
fn framebuffer_validation_checks_format_geometry_and_size() {
    let mut info = BootInfo::empty(BootMethod::Uefi);
    assert!(info.is_valid());

    info.framebuffer = FramebufferInfo {
        address: 0x1000,
        size: 16 * 4,
        width: 4,
        height: 4,
        stride: 4,
        pixel_format: FRAMEBUFFER_PIXEL_RGB,
    };
    assert!(info.is_valid());

    info.framebuffer.pixel_format = 99;
    assert!(!info.is_valid());
    info.framebuffer.pixel_format = FRAMEBUFFER_PIXEL_BGR;
    info.framebuffer.size = 1;
    assert!(!info.is_valid());
}

#[test]
fn boot_info_layout_is_aligned_for_handoff() {
    assert_eq!(core::mem::align_of::<BootInfo>(), 16);
    assert_eq!(core::mem::size_of::<BootInfo>() % 16, 0);
}

#[test]
fn property_region_capacity_is_stable_for_generated_counts() {
    use synos_test_support::property::{run_assert, Config};

    run_assert("boot-protocol.region-capacity", Config::new(0x59_3, 128), |_, _, entropy| {
        let count = (entropy.next_u64() as usize) % (MAX_MEMORY_REGIONS * 2 + 1);
        let mut info = BootInfo::empty(BootMethod::Bios);
        for index in 0..count {
            let accepted = info.push_region(region((index as u64 + 1) * 0x1000, 0x1000));
            if accepted != (index < MAX_MEMORY_REGIONS) {
                return false;
            }
        }
        info.regions().len() == count.min(MAX_MEMORY_REGIONS) && info.is_valid()
    })
    .expect("generated region counts preserve capacity invariants");
}

#[test]
fn boot_recovery_rejects_replay_rollback_downgrade_and_untrusted_roots() {
    let current = valid_boot_info();
    assert!(current.is_valid());

    let mut replayed = valid_boot_info();
    replayed.memory_regions[1] = replayed.memory_regions[0];
    replayed.memory_region_count = 2;
    assert!(!replayed.is_valid());

    let mut rollback = valid_boot_info();
    rollback.version = BOOT_INFO_VERSION.saturating_sub(1);
    assert!(!rollback.is_valid());

    let mut downgrade = valid_boot_info();
    downgrade.version = BOOT_INFO_VERSION + 1;
    assert!(!downgrade.is_valid());

    let mut untrusted = valid_boot_info();
    untrusted.magic ^= 1;
    assert!(!untrusted.is_valid());

    let recovered = BootInfo::empty(BootMethod::Bios);
    assert!(recovered.is_valid());
}
