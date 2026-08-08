//! Deterministic coverage for TODO 10.5.
//!
//! QEMU-facing checks stay in `test_environments.rs`; this target exercises
//! the same contracts without a host executable, wall clock, or TTY.

use std::cell::RefCell;
use std::fs::File;
use std::path::PathBuf;
use std::rc::Rc;

use synos_vm::net::NetError;
use synos_vm::{
    ascii_to_scancodes, translate_input_bytes, DiskController, DiskSpec, FirmwareMode,
    LoopbackHub, LoopbackPort, MacAddress, NetBackend, PacketQueue, SnapshotChain,
    GuestInputMode, SnapshotError, Vm, VmConfig,
};

fn frame(destination: MacAddress, source: MacAddress) -> Vec<u8> {
    let mut packet = vec![0u8; 60];
    packet[..6].copy_from_slice(&destination.to_bytes());
    packet[6..12].copy_from_slice(&source.to_bytes());
    packet[12..14].copy_from_slice(&0x88B5u16.to_be_bytes());
    packet[14..].fill(0xA5);
    packet
}

fn image_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "synos-vm-10-5-{label}-{}-{}.img",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ))
}

#[test]
fn snapshot_serialization_restore_chain_and_failures() {
    let mut vm = Vm::with_config(VmConfig {
        memory_size: 4 * 1024 * 1024,
        firmware: FirmwareMode::Bios,
        ..VmConfig::default()
    });
    vm.mmu_mut().write_byte(0x2000, 0x11).expect("base memory");
    vm.cpu_mut().set_rip(0x2000);
    let base = vm.snapshot();

    vm.mmu_mut().write_byte(0x2000, 0x22).expect("first change");
    vm.cpu_mut().state.halted = true;
    vm.apic()
        .borrow_mut()
        .signal(0x40, synos_vm::ApicTrigger::Edge);
    let first = vm.snapshot();
    let diff = base.diff(&first).expect("snapshot diff");
    assert_eq!(diff.apply_to(&base).expect("apply diff"), first);

    vm.mmu_mut().write_byte(0x3000, 0x33).expect("second change");
    let second = vm.snapshot();
    let mut chain = SnapshotChain::new(base.clone());
    let first_id = chain.checkpoint(first.clone()).expect("first checkpoint");
    let second_id = chain.checkpoint(second.clone()).expect("second checkpoint");
    assert_eq!(first_id, 1);
    assert_eq!(second_id, 2);
    assert_eq!(chain.len(), 3);
    assert_eq!(chain.snapshot(first_id).expect("first snapshot"), first);
    assert_eq!(chain.snapshot(second_id).expect("second snapshot"), second);
    assert!(matches!(chain.snapshot(99), Err(SnapshotError::MissingSnapshot(99))));

    vm.mmu_mut().write_byte(0x2000, 0xFF).expect("mutate memory");
    vm.cpu_mut().set_rip(0x9000);
    vm.apic()
        .borrow_mut()
        .signal(0x41, synos_vm::ApicTrigger::Edge);
    vm.restore_snapshot(&base).expect("restore base");
    assert_eq!(vm.mmu().read_byte(0x2000).expect("restored memory"), 0x11);
    assert_eq!(vm.cpu().rip(), 0x2000);
    assert!(!vm.cpu().state.halted);
    assert_eq!(vm.apic().borrow_mut().pending_vector(), None);

    let bytes = base.to_bytes();
    let mut corrupted = bytes.clone();
    corrupted[0] ^= 1;
    assert!(matches!(
        synos_vm::VmSnapshot::from_bytes(&corrupted),
        Err(SnapshotError::InvalidFormat)
    ));
    let mut wrong_version = bytes;
    wrong_version[8..12].copy_from_slice(&3u32.to_le_bytes());
    assert!(matches!(
        synos_vm::VmSnapshot::from_bytes(&wrong_version),
        Err(SnapshotError::VersionMismatch(3))
    ));

    let mut invalid = diff;
    invalid.changed_pages.push(synos_vm::snapshot::SnapshotPage {
        page: u64::MAX,
        data: vec![1],
    });
    assert!(matches!(
        invalid.apply_to(&base),
        Err(SnapshotError::InvalidState)
    ));

    let mut other_vm = Vm::with_config(VmConfig {
        memory_size: 8 * 1024 * 1024,
        ..VmConfig::default()
    });
    assert!(matches!(
        base.restore_into(&mut other_vm),
        Err(SnapshotError::IncompatibleMemory { .. })
    ));
}

#[test]
fn terminal_translation_and_ps2_fallback_are_stable() {
    let input = [b'h', b'i', b'\r', 0x7F, b'\t', 0x03, 0x04, 0x1B, b'[', b'A'];
    assert_eq!(
        translate_input_bytes(&input),
        vec![b'h', b'i', b'\r', 0x08, b'\t', 0x03, 0x04, 0x1B, b'[', b'A']
    );
    assert_eq!(GuestInputMode::Serial, GuestInputMode::Serial);
    assert_eq!(
        ascii_to_scancodes(0x03),
        vec![0x1D, 0x2E, 0xAE, 0x9D]
    );
    assert_eq!(ascii_to_scancodes(b'\n'), vec![0x1C, 0x9C]);
    assert!(ascii_to_scancodes(0).is_empty());
}

#[test]
fn loopback_routes_isolates_and_applies_deterministic_faults() {
    let hub = Rc::new(RefCell::new(LoopbackHub::new()));
    let left_mac = MacAddress::synos_default(0x70);
    let right_mac = MacAddress::synos_default(0x71);
    let mut left = LoopbackPort::new(hub.clone(), 0, left_mac);
    let mut right = LoopbackPort::new(hub.clone(), 1, right_mac);

    left.transmit(&frame(right_mac, left_mac)).expect("route frame");
    assert_eq!(right.receive().expect("receive frame"), Some(frame(right_mac, left_mac)));
    left.transmit(&frame(left_mac, right_mac)).expect("isolation frame");
    assert_eq!(right.receive().expect("filtered frame"), None);

    for _ in 0..256 {
        left.transmit(&frame(right_mac, left_mac)).expect("fill queue");
    }
    assert_eq!(hub.borrow().queued_packets(1), 256);
    assert_eq!(left.transmit(&frame(right_mac, left_mac)), Err(NetError::QueueFull));

    hub.borrow_mut().set_link_up(false);
    assert!(!hub.borrow().link_up());
    assert_eq!(left.transmit(&frame(right_mac, left_mac)), Err(NetError::LinkDown));
    assert_eq!(right.receive(), Err(NetError::LinkDown));
    hub.borrow_mut().set_link_up(true);
    hub.borrow_mut().clear();
    assert_eq!(hub.borrow().queued_packets(1), 0);

    let mut packets = PacketQueue::new(2, 8);
    assert!(packets.push(vec![1, 2, 3, 4]));
    assert!(packets.push(vec![5, 6, 7, 8]));
    assert!(!packets.push(vec![9]));
    assert_eq!(packets.bytes(), 8);
    packets.clear();
    assert!(packets.is_empty());
}

#[test]
fn storage_and_network_devices_attach_as_a_boot_matrix() {
    let controllers = [
        ("ahci", DiskController::Ahci),
        ("nvme", DiskController::Nvme),
        ("virtio", DiskController::VirtioBlk),
    ];
    let mut specs = Vec::new();
    for (label, controller) in controllers {
        let path = image_path(label);
        let file = File::create(&path).expect("create disk image");
        file.set_len(16 * 1024 * 1024).expect("size disk image");
        specs.push(DiskSpec::new(label, path).with_controller(controller));
    }

    let mut vm = Vm::try_with_config(VmConfig {
        memory_size: 4 * 1024 * 1024,
        disks: specs,
        ..VmConfig::default()
    })
    .expect("attach storage matrix");
    assert_eq!(vm.disks().len(), 3);
    assert!(vm.serial().is_some());
    vm.queue_serial_input(b"help\n");
    assert!(vm.serial().expect("serial device").borrow().input_pending());
    vm.close_disks().expect("close storage matrix");
}
