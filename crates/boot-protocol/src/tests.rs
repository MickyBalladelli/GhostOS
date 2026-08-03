use super::{
    BOOT_INFO_MAGIC, BOOT_INFO_VERSION, BootInfo, BootMethod, MAX_MEMORY_REGIONS, MemoryKind,
    MemoryRegion,
};

fn region(start: u64, length: u64) -> MemoryRegion {
    MemoryRegion {
        start,
        length,
        kind: MemoryKind::Usable,
        attributes: 0,
    }
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
        assert!(info.push_region(region(index as u64 * 0x1000, 0x1000)));
    }

    assert!(!info.push_region(region(0, 0x1000)));
    assert_eq!(info.regions().len(), MAX_MEMORY_REGIONS);
}

#[test]
fn memory_region_end_saturates() {
    assert_eq!(region(u64::MAX - 1, 8).end(), u64::MAX);
}

#[test]
fn property_region_capacity_is_stable_for_generated_counts() {
    use synos_test_support::property::{run_assert, Config};

    run_assert("boot-protocol.region-capacity", Config::new(0x59_3, 128), |_, _, entropy| {
        let count = (entropy.next_u64() as usize) % (MAX_MEMORY_REGIONS * 2 + 1);
        let mut info = BootInfo::empty(BootMethod::Bios);
        for index in 0..count {
            let accepted = info.push_region(region(index as u64 * 0x1000, 0x1000));
            if accepted != (index < MAX_MEMORY_REGIONS) {
                return false;
            }
        }
        info.regions().len() == count.min(MAX_MEMORY_REGIONS) && info.is_valid()
    })
    .expect("generated region counts preserve capacity invariants");
}
