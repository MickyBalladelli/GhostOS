use synos_boot_protocol::{BootInfo, BootMethod, MemoryKind, MemoryRegion};

fn main() {
    let mut info = BootInfo::empty(BootMethod::Bios);
    assert!(info.push_region(MemoryRegion {
        start: 0x10_0000,
        length: 0x20_0000,
        kind: MemoryKind::Usable,
        attributes: 0,
    }));
    assert!(info.is_valid());
    println!("boot info valid: {:?}, {} region", info.method, info.regions().len());
}
