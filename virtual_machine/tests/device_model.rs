//! Deterministic device-model contract tests.

use std::cell::RefCell;
use std::fs::File;
use std::io::Write;
use std::rc::Rc;

use ghostos_vm::devices::{Ahci, ApicTrigger, LocalApic, Nvme, PortDevice, VirtioBlk};
use ghostos_vm::net::{LoopbackHub, LoopbackPort};
use ghostos_vm::{
    DiskImage, MacAddress, Mmu, NetBackend, PacketQueue,
};

const VIRTIO_QUEUE_PFN: u16 = 0x08;
const VIRTIO_QUEUE_NOTIFY: u16 = 0x10;
const VIRTIO_STATUS: u16 = 0x12;
const VIRTIO_ISR_STATUS: u16 = 0x13;
const VIRTIO_DRIVER_OK: u32 = 1 << 2;
const VIRTIO_QUEUE_BASE: u64 = 0x1000;
const VIRTIO_DESC_SIZE: u64 = 16;
const VIRTIO_QUEUE_SIZE: u64 = 128;

fn frame(destination: MacAddress, source: MacAddress, marker: u8) -> Vec<u8> {
    let mut packet = vec![0u8; 60];
    packet[0..6].copy_from_slice(&destination.0);
    packet[6..12].copy_from_slice(&source.0);
    packet[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    packet[14] = marker;
    packet
}

fn virtio_used_base() -> u64 {
    let avail_base = VIRTIO_QUEUE_BASE + VIRTIO_QUEUE_SIZE * VIRTIO_DESC_SIZE;
    (avail_base + 4 + VIRTIO_QUEUE_SIZE * 2 + 3) & !3
}

fn submit_malformed_virtio_requests(blk: &mut VirtioBlk, mmu: &mut Mmu) {
    let avail_base = VIRTIO_QUEUE_BASE + VIRTIO_QUEUE_SIZE * VIRTIO_DESC_SIZE;
    mmu.write_phys(avail_base + 2, &2u16.to_le_bytes())
        .expect("publish two available requests");
    mmu.write_phys(avail_base + 4, &0u16.to_le_bytes())
        .expect("publish first request");
    mmu.write_phys(avail_base + 6, &1u16.to_le_bytes())
        .expect("publish second request");
    PortDevice::write(blk, VIRTIO_QUEUE_PFN, 1, 4).expect("enable virtio queue");
    PortDevice::write(blk, VIRTIO_STATUS, VIRTIO_DRIVER_OK as u64, 4)
        .expect("set virtio driver status");
    PortDevice::write(blk, VIRTIO_QUEUE_NOTIFY, 0, 4).expect("notify virtio queue");
}

#[test]
fn device_ordering_and_queue_backpressure_are_fifo_and_bounded() {
    let mut queue = PacketQueue::new(4, 5);
    assert!(queue.push(vec![1, 2]));
    assert!(queue.push(vec![3, 4]));
    assert!(!queue.push(vec![5, 6]));
    assert_eq!(queue.bytes(), 4);
    assert_eq!(queue.pop(), Some(vec![1, 2]));
    assert_eq!(queue.pop(), Some(vec![3, 4]));
    assert!(queue.is_empty());

    let hub = Rc::new(RefCell::new(LoopbackHub::new()));
    let source_mac = MacAddress::ghostos_default(1);
    let destination_mac = MacAddress::ghostos_default(2);
    let mut tx = LoopbackPort::new(hub.clone(), 0, source_mac);
    let mut rx = LoopbackPort::new(hub.clone(), 1, destination_mac);
    tx.transmit(&frame(destination_mac, source_mac, 1))
        .expect("transmit first frame");
    tx.transmit(&frame(destination_mac, source_mac, 2))
        .expect("transmit second frame");
    assert_eq!(hub.borrow().queued_packets(1), 2);
    assert_eq!(rx.receive().expect("receive first").unwrap()[14], 1);
    assert_eq!(rx.receive().expect("receive second").unwrap()[14], 2);
    assert!(rx.receive().expect("receive empty").is_none());
}

#[test]
fn device_interrupts_coalesce_and_preserve_completion_order() {
    let apic = Rc::new(RefCell::new(LocalApic::new(0)));
    let mut blk = VirtioBlk::new();
    blk.attach_apic(apic.clone());
    blk.set_irq_vector(0x45);
    let mut mmu = Mmu::new(0x20_000);

    submit_malformed_virtio_requests(&mut blk, &mut mmu);
    blk.poll(&mut mmu);

    let used_base = virtio_used_base();
    assert_eq!(mmu.read_phys(used_base + 4, 2).expect("first used head"), [0, 0]);
    assert_eq!(mmu.read_phys(used_base + 12, 2).expect("second used head"), [1, 0]);
    assert_eq!(
        PortDevice::read(&mut blk, VIRTIO_ISR_STATUS, 1).expect("read ISR"),
        1
    );
    assert_eq!(
        PortDevice::read(&mut blk, VIRTIO_ISR_STATUS, 1).expect("clear ISR"),
        0
    );
    assert_eq!(apic.borrow_mut().pending_vector(), Some(0x45));

    apic.borrow_mut().signal(0x45, ApicTrigger::Edge);
    assert_eq!(apic.borrow_mut().pending_vector(), Some(0x45));
}

#[test]
fn reset_during_io_drops_pending_work_and_stale_completion() {
    let apic = Rc::new(RefCell::new(LocalApic::new(0)));
    let mut blk = VirtioBlk::new();
    blk.attach_apic(apic.clone());
    blk.set_irq_vector(0x46);
    let mut mmu = Mmu::new(0x20_000);
    submit_malformed_virtio_requests(&mut blk, &mut mmu);
    assert!(blk.has_pending());

    blk.reset();
    assert!(!blk.has_pending());
    blk.poll(&mut mmu);

    assert_eq!(
        mmu.read_phys(virtio_used_base() + 2, 2)
            .expect("used index after reset"),
        [0, 0]
    );
    assert_eq!(apic.borrow_mut().pending_vector(), None);
}

#[test]
fn hot_removal_detaches_storage_without_leaving_guest_capacity() {
    let ahci_path = std::env::temp_dir().join(format!(
        "ghostos-device-ahci-{}-{}.img",
        std::process::id(),
        1
    ));
    let nvme_path = std::env::temp_dir().join(format!(
        "ghostos-device-nvme-{}-{}.img",
        std::process::id(),
        1
    ));
    let mut ahci_file = File::create(&ahci_path).expect("create AHCI image");
    ahci_file.set_len(4096).expect("size AHCI image");
    ahci_file.flush().expect("flush AHCI image");
    let mut nvme_file = File::create(&nvme_path).expect("create NVMe image");
    nvme_file.set_len(4096).expect("size NVMe image");
    nvme_file.flush().expect("flush NVMe image");

    let mut ahci = Ahci::new();
    ahci.attach_disk(DiskImage::open(&ahci_path).expect("open AHCI image"));
    assert_eq!(ahci.sector_count(), Some(8));
    drop(ahci.detach_disk().expect("remove AHCI disk"));
    assert_eq!(ahci.sector_count(), None);

    let mut nvme = Nvme::new();
    nvme.attach_namespace(DiskImage::open(&nvme_path).expect("open NVMe image"));
    assert!(nvme.detach_namespace().is_some());
    assert!(!nvme.has_pending());

    let _ = std::fs::remove_file(ahci_path);
    let _ = std::fs::remove_file(nvme_path);
}
