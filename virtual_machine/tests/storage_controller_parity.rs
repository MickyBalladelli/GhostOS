//! Guest-visible parity checks for the three block-device models.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use synos_vm::devices::{Ahci, Device, Nvme, PortDevice, VirtioBlk};
use synos_vm::{DiskImage, Mmu};

const SECTOR_SIZE: usize = 512;
const SECTORS: u64 = 8;
const BUFFER: u64 = 0x6000;
const PAYLOAD: [u8; SECTOR_SIZE] = [0xA5; SECTOR_SIZE];

#[derive(Debug, PartialEq, Eq)]
struct BlockReport {
    capacity: u64,
    read_after_write: Vec<u8>,
    flush_ok: bool,
    failure_observed: bool,
}

fn image(name: &str) -> (PathBuf, DiskImage) {
    let path = std::env::temp_dir().join(format!(
        "synos-storage-parity-{name}-{}.img",
        std::process::id()
    ));
    let mut file = File::create(&path).expect("create parity image");
    file.set_len(SECTORS * SECTOR_SIZE as u64)
        .expect("size parity image");
    file.flush().expect("flush parity image");
    (path.clone(), DiskImage::open(path).expect("open parity image"))
}

fn ahci_write(ahci: &mut Ahci, _mmu: &mut Mmu, offset: u64, value: u32) {
    Device::write(ahci, offset, value as u64, 4).expect("AHCI register write");
}

fn ahci_setup(ahci: &mut Ahci, mmu: &mut Mmu) {
    ahci_write(ahci, mmu, 0x100, 0x1000);
    ahci_write(ahci, mmu, 0x108, 0x2000);
    ahci_write(ahci, mmu, 0x114, 0xFFFF_FFFF);
    ahci_write(ahci, mmu, 0x04, (1 << 31) | (1 << 1));
}

fn ahci_issue(
    ahci: &mut Ahci,
    mmu: &mut Mmu,
    command: u8,
    lba: u64,
    sector_count: u16,
    buffer: u64,
) {
    let mut header = [0u8; 32];
    header[0..4].copy_from_slice(&1u32.to_le_bytes());
    header[8..16].copy_from_slice(&0x3000u64.to_le_bytes());
    mmu.write_phys(0x1000, &header).expect("write AHCI header");

    let mut fis = [0u8; 64];
    fis[0] = 0x27;
    fis[2] = command;
    fis[4..8].copy_from_slice(&(lba as u32).to_le_bytes());
    fis[8..12].copy_from_slice(&((lba >> 24) as u32).to_le_bytes());
    fis[12..14].copy_from_slice(&sector_count.to_le_bytes());
    mmu.write_phys(0x3000, &fis).expect("write AHCI FIS");

    let mut prd = [0u8; 16];
    prd[0..8].copy_from_slice(&buffer.to_le_bytes());
    prd[12..16].copy_from_slice(&((SECTOR_SIZE as u32 - 1)).to_le_bytes());
    mmu.write_phys(0x3080, &prd).expect("write AHCI PRD");

    ahci_write(ahci, mmu, 0x138, 1);
    ahci.poll_dma(mmu);
}

fn run_ahci() -> (PathBuf, BlockReport) {
    let (path, disk) = image("ahci");
    let mut ahci = Ahci::new();
    ahci.attach_disk(disk);
    let mut mmu = Mmu::new(0x20_000);
    ahci_setup(&mut ahci, &mut mmu);

    ahci_issue(&mut ahci, &mut mmu, 0xEC, 0, 1, 0x5000);
    let mut capacity = 0u64;
    for word in 0..4 {
        let bytes = mmu
            .read_phys(0x5000 + 200 + word * 2, 2)
            .expect("read AHCI IDENTIFY capacity");
        capacity |= (u16::from_le_bytes([bytes[0], bytes[1]]) as u64) << (word * 16);
    }

    mmu.write_phys(BUFFER, &PAYLOAD).expect("write AHCI source buffer");
    ahci_issue(&mut ahci, &mut mmu, 0x35, 1, 1, BUFFER);
    mmu.write_phys(BUFFER, &[0; SECTOR_SIZE])
        .expect("clear AHCI read buffer");
    ahci_issue(&mut ahci, &mut mmu, 0x25, 1, 1, BUFFER);
    let read_after_write = mmu.read_phys(BUFFER, SECTOR_SIZE).expect("read AHCI result");

    ahci_issue(&mut ahci, &mut mmu, 0xEA, 0, 0, BUFFER);
    let flush_ok = Device::read(&ahci, 0x120, 4)
        .expect("read AHCI flush status")
        & 1
        == 0;
    ahci_issue(&mut ahci, &mut mmu, 0x25, SECTORS, 1, BUFFER);
    let failure_observed =
        Device::read(&ahci, 0x120, 4).expect("read AHCI task file") & 1 != 0;

    (
        path,
        BlockReport {
            capacity,
            read_after_write,
            flush_ok,
            failure_observed,
        },
    )
}

fn nvme_write(nvme: &mut Nvme, offset: u64, value: u64, size: u8) {
    Device::write(nvme, offset, value, size).expect("NVMe register write");
}

fn nvme_admin_command(
    nvme: &mut Nvme,
    mmu: &mut Mmu,
    slot: u64,
    opcode: u8,
    cid: u16,
    prp1: u64,
    cdw10: u32,
    cdw12: u32,
    tail: u32,
) {
    let mut command = [0u8; 64];
    command[0] = opcode;
    command[2..4].copy_from_slice(&cid.to_le_bytes());
    command[4..8].copy_from_slice(&1u32.to_le_bytes());
    command[8..16].copy_from_slice(&prp1.to_le_bytes());
    command[40..44].copy_from_slice(&cdw10.to_le_bytes());
    command[48..52].copy_from_slice(&cdw12.to_le_bytes());
    mmu.write_phys(0x1000 + slot * 64, &command)
        .expect("write NVMe admin command");
    nvme_write(nvme, 0x1000, tail as u64, 4);
    nvme.poll_dma(mmu);
}

fn nvme_setup(nvme: &mut Nvme, mmu: &mut Mmu) {
    nvme_write(nvme, 0x24, 0x0003_0003, 4);
    nvme_write(nvme, 0x28, 0x1000, 8);
    nvme_write(nvme, 0x30, 0x2000, 8);
    nvme_write(nvme, 0x14, 1, 4);

    nvme_admin_command(nvme, mmu, 0, 0x05, 1, 0x3000, 1 | (3 << 16), 0, 1);
    nvme_admin_command(nvme, mmu, 1, 0x01, 2, 0x4000, 1 | (3 << 16), 1, 2);
}

fn nvme_io_command(
    nvme: &mut Nvme,
    mmu: &mut Mmu,
    slot: u64,
    opcode: u8,
    lba: u64,
    buffer: u64,
    tail: u32,
) -> u16 {
    let mut command = [0u8; 64];
    command[0] = opcode;
    command[2..4].copy_from_slice(&(slot as u16).to_le_bytes());
    command[4..8].copy_from_slice(&1u32.to_le_bytes());
    command[8..16].copy_from_slice(&buffer.to_le_bytes());
    command[40..44].copy_from_slice(&(lba as u32).to_le_bytes());
    command[44..48].copy_from_slice(&((lba >> 32) as u32).to_le_bytes());
    mmu.write_phys(0x4000 + slot * 64, &command)
        .expect("write NVMe I/O command");
    nvme_write(nvme, 0x1008, tail as u64, 4);
    nvme.poll_dma(mmu);
    let completion = mmu
        .read_phys(0x3000 + slot * 16 + 10, 2)
        .expect("read NVMe completion");
    u16::from_le_bytes([completion[0], completion[1]]) >> 1
}

fn run_nvme() -> (PathBuf, BlockReport) {
    let (path, disk) = image("nvme");
    let mut nvme = Nvme::new();
    nvme.attach_namespace(disk);
    let mut mmu = Mmu::new(0x20_000);
    nvme_setup(&mut nvme, &mut mmu);

    nvme_admin_command(&mut nvme, &mut mmu, 2, 0x06, 3, 0x7000, 0, 0, 3);
    let capacity = u64::from_le_bytes(
        mmu.read_phys(0x7000, 8)
            .expect("read NVMe IDENTIFY capacity")
            .try_into()
            .expect("capacity bytes"),
    );

    mmu.write_phys(BUFFER, &PAYLOAD).expect("write NVMe source buffer");
    assert_eq!(nvme_io_command(&mut nvme, &mut mmu, 0, 0x01, 1, BUFFER, 1), 0);
    mmu.write_phys(BUFFER, &[0; SECTOR_SIZE])
        .expect("clear NVMe read buffer");
    assert_eq!(nvme_io_command(&mut nvme, &mut mmu, 1, 0x02, 1, BUFFER, 2), 0);
    let read_after_write = mmu.read_phys(BUFFER, SECTOR_SIZE).expect("read NVMe result");

    let flush_ok = nvme_io_command(&mut nvme, &mut mmu, 2, 0x00, 0, 0, 3) == 0;
    let failure_observed = nvme_io_command(&mut nvme, &mut mmu, 3, 0x02, SECTORS, BUFFER, 0) != 0;

    (
        path,
        BlockReport {
            capacity,
            read_after_write,
            flush_ok,
            failure_observed,
        },
    )
}

fn virtio_write(blk: &mut VirtioBlk, port: u16, value: u64) {
    PortDevice::write(blk, port, value, 4).expect("Virtio register write");
}

fn virtio_descriptor(mmu: &mut Mmu, index: u16, addr: u64, len: u32, flags: u16, next: u16) {
    let mut descriptor = [0u8; 16];
    descriptor[0..8].copy_from_slice(&addr.to_le_bytes());
    descriptor[8..12].copy_from_slice(&len.to_le_bytes());
    descriptor[12..14].copy_from_slice(&flags.to_le_bytes());
    descriptor[14..16].copy_from_slice(&next.to_le_bytes());
    mmu.write_phys(0x1000 + index as u64 * 16, &descriptor)
        .expect("write Virtio descriptor");
}

fn virtio_submit(
    blk: &mut VirtioBlk,
    mmu: &mut Mmu,
    tail: u16,
    request_type: u32,
    sector: u64,
    data_write: bool,
) -> u8 {
    let mut header = [0u8; 16];
    header[0..4].copy_from_slice(&request_type.to_le_bytes());
    header[8..16].copy_from_slice(&sector.to_le_bytes());
    mmu.write_phys(0x3000, &header).expect("write Virtio header");
    virtio_descriptor(mmu, 0, 0x3000, 16, 1, 1);
    virtio_descriptor(
        mmu,
        1,
        BUFFER,
        SECTOR_SIZE as u32,
        1 | if data_write { 0 } else { 2 },
        2,
    );
    virtio_descriptor(mmu, 2, 0x5000, 1, 2, 0);
    mmu.write_phys(0x5000, &[0xFF]).expect("clear Virtio status");
    let avail_base = 0x1800;
    mmu.write_phys(avail_base + 4 + (tail as u64 - 1) * 2, &0u16.to_le_bytes())
        .expect("publish Virtio head");
    mmu.write_phys(avail_base + 2, &tail.to_le_bytes())
        .expect("publish Virtio index");
    virtio_write(blk, 0x10, 0);
    blk.poll(mmu);
    mmu.read_phys(0x5000, 1).expect("read Virtio status")[0]
}

fn run_virtio() -> (PathBuf, BlockReport) {
    let (path, disk) = image("virtio");
    let mut blk = VirtioBlk::new();
    blk.attach_disk(disk);
    let mut mmu = Mmu::new(0x20_000);
    virtio_write(&mut blk, 0x08, 1);
    virtio_write(&mut blk, 0x12, 4);
    let capacity = PortDevice::read(&mut blk, 0x14, 8).expect("read Virtio capacity");

    mmu.write_phys(BUFFER, &PAYLOAD)
        .expect("write Virtio source buffer");
    assert_eq!(virtio_submit(&mut blk, &mut mmu, 1, 1, 1, true), 0);
    mmu.write_phys(BUFFER, &[0; SECTOR_SIZE])
        .expect("clear Virtio read buffer");
    assert_eq!(virtio_submit(&mut blk, &mut mmu, 2, 0, 1, false), 0);
    let read_after_write = mmu.read_phys(BUFFER, SECTOR_SIZE).expect("read Virtio result");
    let flush_ok = virtio_submit(&mut blk, &mut mmu, 3, 4, 0, true) == 0;
    let failure_observed = virtio_submit(&mut blk, &mut mmu, 4, 0, SECTORS, false) != 0;

    (
        path,
        BlockReport {
            capacity,
            read_after_write,
            flush_ok,
            failure_observed,
        },
    )
}

#[test]
fn ahci_nvme_and_virtio_have_matching_block_contracts() {
    let (ahci_path, ahci) = run_ahci();
    let (nvme_path, nvme) = run_nvme();
    let (virtio_path, virtio) = run_virtio();

    assert_eq!(ahci, nvme, "AHCI and NVMe behavior differ");
    assert_eq!(nvme, virtio, "NVMe and Virtio behavior differ");

    let _ = std::fs::remove_file(ahci_path);
    let _ = std::fs::remove_file(nvme_path);
    let _ = std::fs::remove_file(virtio_path);
}
