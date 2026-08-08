#[test]
fn snapshot_inventory_covers_vm_fields_and_required_state_families() {
    let inventory = include_str!("../docs/SNAPSHOT_STATE_INVENTORY.md");

    for status in ["Serialized", "Rebuilt", "Intentionally excluded"] {
        assert!(inventory.contains(status), "missing state status {status}");
    }

    for field in [
        "cpu",
        "mmu",
        "interrupt_controller",
        "ports",
        "serial",
        "power_state",
        "power_notifications",
        "guest_power_notifications",
        "guest_agent",
        "pv_clock",
        "memory_hotplug",
        "ps2",
        "pci",
        "apic",
        "pit",
        "hpet",
        "ahci",
        "nvme",
        "e1000",
        "virtio_net",
        "virtio_blk",
        "virtio_console",
        "virtio_rng",
        "persistence",
        "display",
        "bios",
        "execution",
        "hardware_acceleration",
        "disk_manager",
        "booted_system_disk",
        "config",
        "initialized",
    ] {
        assert!(
            inventory.contains(&format!("`{field}`")),
            "VM field {field} missing from snapshot inventory"
        );
    }

    for state_family in [
        "Device queues",
        "Serial/input buffers",
        "Timers",
        "RNG",
        "Disks",
        "Network topology",
        "Guest agent",
        "Hotplug",
        "Acceleration",
        "Firmware",
    ] {
        assert!(
            inventory.contains(state_family),
            "state family {state_family} missing from snapshot inventory"
        );
    }
}
