//! Opt-in QEMU coverage. Keep this separate from deterministic VM tests.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(10);

fn qemu_binary() -> String {
    std::env::var("SYNOS_QEMU_BIN").unwrap_or_else(|_| "qemu-system-x86_64".to_string())
}

fn qemu_image(firmware: &str) -> PathBuf {
    let variable = if firmware == "uefi" {
        "SYNOS_QEMU_UEFI_IMAGE"
    } else {
        "SYNOS_QEMU_IMAGE"
    };
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../build/bios/synos-bios.img"))
}

fn run_qemu(firmware: &str, cpus: usize) -> String {
    let image = qemu_image(firmware);
    assert!(image.is_file(), "missing QEMU image: {}", image.display());
    let mut command = Command::new(qemu_binary());
    command
        .args([
            "-machine", "q35", "-cpu", "max", "-m", "128M", "-display", "none",
            "-monitor", "none", "-no-reboot", "-no-shutdown", "-serial", "stdio",
        ])
        .arg("-smp")
        .arg(cpus.to_string())
        .arg("-drive")
        .arg(format!("file={},format=raw,if=ide,readonly=on", image.display()))
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());
    if firmware == "uefi" {
        let firmware_path = std::env::var_os("SYNOS_QEMU_UEFI_FIRMWARE")
            .map(PathBuf::from)
            .expect("set SYNOS_QEMU_UEFI_FIRMWARE for UEFI smoke tests");
        assert!(
            firmware_path.is_file(),
            "missing UEFI firmware: {}",
            firmware_path.display()
        );
        command.arg("-bios").arg(firmware_path);
    }
    let mut child = command
        .spawn()
        .expect("start QEMU");
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if child.try_wait().expect("poll QEMU").is_some() {
            break
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output().expect("collect QEMU output");
    let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    let log_dir = std::env::var_os("SYNOS_QEMU_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../build/qemu-matrix-59-11"));
    fs::create_dir_all(&log_dir).expect("create QEMU log directory");
    let log_path = log_dir.join(format!("{firmware}-{cpus}cpu.log"));
    fs::write(log_path, &log).expect("save QEMU log");
    log
}

fn assert_boot(log: &str, firmware: &str, cpus: usize) {
    assert!(
        log.contains("SynOS kernel bootstrap"),
        "{firmware} QEMU boot failed with {cpus} CPUs; log: {log:?}"
    );
    assert!(!log.contains("KERNEL PANIC"), "guest panic; log: {log:?}");
}

#[test]
#[ignore = "requires SYNOS_RUN_QEMU_TESTS=1, QEMU, and SYNOS_QEMU_IMAGE"]
fn qemu_bios_one_cpu_and_smp() {
    if std::env::var_os("SYNOS_RUN_QEMU_TESTS").is_none() {
        return
    }
    assert_boot(&run_qemu("bios", 1), "bios", 1);
    assert_boot(&run_qemu("bios", 2), "bios", 2);
}

#[test]
#[ignore = "requires SYNOS_RUN_QEMU_TESTS=1, QEMU, SYNOS_QEMU_UEFI_IMAGE, and SYNOS_QEMU_UEFI_FIRMWARE"]
fn qemu_uefi_one_cpu_and_smp() {
    if std::env::var_os("SYNOS_RUN_QEMU_TESTS").is_none() {
        return
    }
    assert_boot(&run_qemu("uefi", 1), "uefi", 1);
    assert_boot(&run_qemu("uefi", 2), "uefi", 2);
}
