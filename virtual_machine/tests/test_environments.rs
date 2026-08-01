//! Environment checks for the virtual machine project.
//!
//! The network and storage checks run without external tools. QEMU checks are
//! opt-in because they need a locally built BIOS image and a QEMU executable.

use std::cell::RefCell;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use synos_vm::{DiskImage, LoopbackHub, LoopbackPort, MacAddress, NetBackend};

const STORAGE_SECTORS: u64 = 256;
const QEMU_BOOT_TIMEOUT: Duration = Duration::from_secs(5);

fn temporary_path(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("synos-vm-{name}-{}-{now}.img", std::process::id()))
}

fn create_raw_image(path: &Path, sectors: u64) {
    let file = File::create(path).expect("create temporary disk image");
    file.set_len(sectors * 512)
        .expect("size temporary disk image");
}

#[test]
fn network_connectivity_loopback() {
    let hub = Rc::new(RefCell::new(LoopbackHub::new()));
    let left_mac = MacAddress::synos_default(0x10);
    let right_mac = MacAddress::synos_default(0x11);
    let mut left = LoopbackPort::new(hub.clone(), 0, left_mac);
    let mut right = LoopbackPort::new(hub, 1, right_mac);

    let mut frame = vec![0u8; 60];
    frame[..6].copy_from_slice(&right_mac.to_bytes());
    frame[6..12].copy_from_slice(&left_mac.to_bytes());
    frame[12..14].copy_from_slice(&0x88B5u16.to_be_bytes());
    frame[14..].fill(0x5A);

    left.transmit(&frame).expect("loopback transmit");
    let received = right
        .receive()
        .expect("loopback receive")
        .expect("loopback packet");

    assert_eq!(received.len(), 60);
    assert_eq!(&received[..14], &frame[..14]);
    assert_eq!(&received[14..], &frame[14..]);
    assert!(left.link_up());
    assert!(right.link_up());
}

#[test]
fn storage_io_performance_and_integrity() {
    let path = temporary_path("storage");
    create_raw_image(&path, STORAGE_SECTORS);

    let mut image = DiskImage::open(&path).expect("open temporary disk image");
    let started = Instant::now();
    for lba in 0..STORAGE_SECTORS {
        let mut sector = [0u8; 512];
        sector.fill((lba % 251) as u8);
        image
            .write_sector(lba, &sector)
            .expect("write storage sector");
    }
    let write_elapsed = started.elapsed();

    let started = Instant::now();
    for lba in 0..STORAGE_SECTORS {
        let mut sector = [0u8; 512];
        image
            .read_sector(lba, &mut sector)
            .expect("read storage sector");
        assert!(sector.iter().all(|byte| *byte == (lba % 251) as u8));
    }
    let read_elapsed = started.elapsed();
    let bytes = STORAGE_SECTORS * 512;

    assert_eq!(image.sector_count(), STORAGE_SECTORS);
    assert!(write_elapsed.as_nanos() > 0);
    assert!(read_elapsed.as_nanos() > 0);

    eprintln!(
        "storage: wrote {bytes} bytes in {:?} ({:.2} MiB/s), read in {:?} ({:.2} MiB/s)",
        write_elapsed,
        throughput_mib(bytes, write_elapsed),
        read_elapsed,
        throughput_mib(bytes, read_elapsed),
    );

    drop(image);
    let _ = fs::remove_file(path);
}

fn throughput_mib(bytes: u64, elapsed: Duration) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64().max(f64::MIN_POSITIVE)
}

#[test]
#[ignore = "requires SYNOS_QEMU_IMAGE and a local QEMU installation"]
fn qemu_integration_boot() {
    let Some(output) = run_qemu_boot(1) else {
        return;
    };
    assert_boot_output(&output, 1);
}

#[test]
#[ignore = "requires SYNOS_QEMU_IMAGE and a local QEMU installation"]
fn qemu_smp_boot() {
    let Some(output) = run_qemu_boot(2) else {
        return;
    };
    assert_boot_output(&output, 2);
}

#[test]
#[ignore = "requires SYNOS_QEMU_IMAGE and a local QEMU installation"]
fn qemu_root_filesystem_io() {
    let Some(output) = run_qemu_boot(1) else {
        return;
    };
    assert_boot_output(&output, 1);
    assert!(
        output.contains("SynFS root mounted")
            && output.contains("application I/O validated"),
        "QEMU did not validate root filesystem I/O; serial output was: {output:?}"
    );
}

fn qemu_image() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SYNOS_QEMU_IMAGE") {
        return Some(PathBuf::from(path));
    }

    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent()?;
    Some(repository.join("build/bios/synos-bios.img"))
}

fn run_qemu_boot(vcpus: usize) -> Option<String> {
    if std::env::var_os("SYNOS_RUN_QEMU_TESTS").is_none() {
        eprintln!("QEMU test skipped: set SYNOS_RUN_QEMU_TESTS=1 to enable");
        return None;
    }

    let qemu = std::env::var_os("SYNOS_QEMU_BIN").unwrap_or_else(|| "qemu-system-x86_64".into());
    let image = qemu_image().expect("locate repository root");
    if !image.is_file() {
        panic!("QEMU test image does not exist: {}", image.display());
    }

    let serial_path = temporary_path(&format!("qemu-{vcpus}"));
    let mut child = Command::new(&qemu)
        .args([
            "-machine",
            "q35",
            "-cpu",
            "max",
            "-m",
            "128M",
            "-display",
            "none",
            "-monitor",
            "none",
            "-no-reboot",
            "-no-shutdown",
            "-serial",
        ])
        .arg(format!("file:{}", serial_path.display()))
        .arg("-smp")
        .arg(vcpus.to_string())
        .arg("-drive")
        .arg(format!(
            "file={},format=raw,if=ide,readonly=on",
            image.display()
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap_or_else(|error| panic!("start QEMU `{}`: {error}", qemu.to_string_lossy()));

    wait_for_qemu(&mut child);
    let output = fs::read_to_string(&serial_path).unwrap_or_default();
    let _ = fs::remove_file(serial_path);
    Some(output)
}

fn wait_for_qemu(child: &mut Child) {
    let deadline = Instant::now() + QEMU_BOOT_TIMEOUT;
    loop {
        if child.try_wait().expect("poll QEMU").is_some() {
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn assert_boot_output(output: &str, vcpus: usize) {
    assert!(
        output.contains("SynOS kernel bootstrap"),
        "QEMU did not boot SynOS with {vcpus} vCPUs; serial output was: {output:?}"
    );
    assert!(
        !output.contains("KERNEL PANIC"),
        "SynOS panicked with {vcpus} vCPUs; serial output was: {output:?}"
    );
}
