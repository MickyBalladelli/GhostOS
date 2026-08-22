//! Opt-in QEMU coverage. Keep this separate from deterministic VM tests.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::net::UnixStream;

const TIMEOUT: Duration = Duration::from_secs(10);

fn qemu_binary() -> String {
    std::env::var("GHOSTOS_QEMU_BIN").unwrap_or_else(|_| "qemu-system-x86_64".to_string())
}

fn qemu_image(firmware: &str) -> PathBuf {
    let variable = if firmware == "uefi" {
        "GHOSTOS_QEMU_UEFI_IMAGE"
    } else {
        "GHOSTOS_QEMU_IMAGE"
    };
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../build/bios/ghostos-bios.img"))
}

fn run_qemu(firmware: &str, cpus: usize) -> String {
    run_qemu_with_images(firmware, cpus, &[])
}

fn run_qemu_with_images(firmware: &str, cpus: usize, extra_images: &[PathBuf]) -> String {
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
    for extra in extra_images {
        command.arg("-drive").arg(format!(
            "file={},format=raw,if=ide,readonly=on",
            extra.display()
        ));
    }
    if let Some(accel) = std::env::var_os("GHOSTOS_QEMU_ACCEL") {
        command.arg("-accel").arg(accel);
    }
    if firmware == "uefi" {
        let firmware_path = std::env::var_os("GHOSTOS_QEMU_UEFI_FIRMWARE")
            .map(PathBuf::from)
            .expect("set GHOSTOS_QEMU_UEFI_FIRMWARE for UEFI smoke tests");
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
    let log_dir = std::env::var_os("GHOSTOS_QEMU_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../build/qemu-matrix-59-11"));
    fs::create_dir_all(&log_dir).expect("create QEMU log directory");
    let log_path = log_dir.join(format!("{firmware}-{cpus}cpu.log"));
    fs::write(log_path, &log).expect("save QEMU log");
    log
}

fn assert_boot(log: &str, firmware: &str, cpus: usize) {
    assert!(
        log.contains("GhostOS kernel bootstrap"),
        "{firmware} QEMU boot failed with {cpus} CPUs; log: {log:?}"
    );
    assert!(!log.contains("KERNEL PANIC"), "guest panic; log: {log:?}");
}

#[test]
#[ignore = "requires GHOSTOS_RUN_QEMU_TESTS=1, QEMU, and GHOSTOS_QEMU_IMAGE"]
fn qemu_bios_one_cpu_and_smp() {
    if std::env::var_os("GHOSTOS_RUN_QEMU_TESTS").is_none() {
        return
    }
    assert_boot(&run_qemu("bios", 1), "bios", 1);
    assert_boot(&run_qemu("bios", 2), "bios", 2);
}

#[test]
#[ignore = "requires GHOSTOS_RUN_QEMU_TESTS=1, QEMU, GHOSTOS_QEMU_UEFI_IMAGE, and GHOSTOS_QEMU_UEFI_FIRMWARE"]
fn qemu_uefi_one_cpu_and_smp() {
    if std::env::var_os("GHOSTOS_RUN_QEMU_TESTS").is_none() {
        return
    }
    assert_boot(&run_qemu("uefi", 1), "uefi", 1);
    assert_boot(&run_qemu("uefi", 2), "uefi", 2);
}

#[test]
#[ignore = "requires GHOSTOS_RUN_QEMU_TESTS=1, QEMU, and GHOSTOS_QEMU_IMAGE"]
fn qemu_attached_disk_boot() {
    if std::env::var_os("GHOSTOS_RUN_QEMU_TESTS").is_none() {
        return
    }
    let path = temporary_path("attached-disk");
    let file = fs::File::create(&path).expect("create attached disk");
    file.set_len(1024 * 1024).expect("size attached disk");
    let log = run_qemu_with_images("bios", 1, std::slice::from_ref(&path));
    let _ = fs::remove_file(path);
    assert_boot(&log, "bios", 1);
}

#[cfg(unix)]
#[test]
#[ignore = "requires GHOSTOS_RUN_QEMU_TESTS=1 and QEMU"]
fn qemu_reboot_and_shutdown_lifecycle() {
    if std::env::var_os("GHOSTOS_RUN_QEMU_TESTS").is_none() {
        return
    }
    let mut run = QmpRun::start("bios", 1, None);
    assert_qmp_ok(&run.command("system_reset"));
    assert_qmp_ok(&run.command("quit"));
    let log = run.finish("bios-reboot-shutdown");
    assert_boot(&log, "bios", 1);
}

#[cfg(unix)]
#[test]
#[ignore = "requires GHOSTOS_RUN_QEMU_TESTS=1 and QEMU"]
fn qemu_terminal_wakeup() {
    if std::env::var_os("GHOSTOS_RUN_QEMU_TESTS").is_none() {
        return
    }
    let mut run = QmpRun::start("bios", 1, None);
    assert_qmp_ok(&run.send_key("enter"));
    assert_qmp_ok(&run.send_key("a"));
    assert_qmp_ok(&run.command("quit"));
    let log = run.finish("bios-terminal-wakeup");
    assert_boot(&log, "bios", 1);
}

#[cfg(unix)]
#[test]
#[ignore = "requires GHOSTOS_RUN_QEMU_TESTS=1, QEMU, and qemu-img"]
fn qemu_snapshot_restore() {
    if std::env::var_os("GHOSTOS_RUN_QEMU_TESTS").is_none() {
        return
    }
    let qemu_img = std::env::var_os("GHOSTOS_QEMU_IMG_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("qemu-img"));
    let available = Command::new(&qemu_img)
        .arg("--version")
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !available {
        eprintln!("QEMU snapshot evidence skipped: qemu-img is unavailable");
        return
    }
    snapshot_restore_with(&qemu_img.to_string_lossy());
}

#[cfg(unix)]
fn snapshot_restore_with(qemu_img: &str) {
    let base = qemu_image("bios");
    let overlay = temporary_path("snapshot-overlay");
    let output = Command::new(qemu_img)
        .args(["create", "-f", "qcow2", "-F", "raw", "-b"])
        .arg(&base)
        .arg(&overlay)
        .output()
        .expect("start qemu-img");
    assert!(output.status.success(), "qemu-img failed: {:?}", output);
    let mut run = QmpRun::start_with_image("bios", 1, overlay.clone(), false);
    assert_qmp_ok(&run.human("savevm ghostos-evidence"));
    assert_qmp_ok(&run.human("loadvm ghostos-evidence"));
    assert_qmp_ok(&run.command("quit"));
    let log = run.finish("bios-snapshot-restore");
    let _ = fs::remove_file(overlay);
    assert_boot(&log, "bios", 1);
}

#[test]
#[ignore = "requires QEMU"]
fn qemu_failure_cleanup() {
    if std::env::var_os("GHOSTOS_RUN_QEMU_TESTS").is_none() {
        return
    }
    if !Command::new(qemu_binary())
        .arg("--version")
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
    {
        return
    }
    let missing = temporary_path("missing-image");
    let output = Command::new(qemu_binary())
        .args(["-display", "none", "-no-reboot", "-no-shutdown", "-drive"])
        .arg(format!("file={},format=raw,if=ide,readonly=on", missing.display()))
        .output()
        .expect("start QEMU failure case");
    assert!(!output.status.success(), "QEMU accepted a missing image");
    assert!(!missing.exists(), "failure case created the missing image");
}

fn temporary_path(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("ghostos-qemu-{name}-{}-{stamp}", std::process::id()))
}

#[cfg(unix)]
struct QmpRun {
    child: std::process::Child,
    qmp: UnixStream,
    serial: PathBuf,
    socket: PathBuf,
}

#[cfg(unix)]
impl QmpRun {
    fn start(firmware: &str, cpus: usize, extra: Option<PathBuf>) -> Self {
        let image = qemu_image(firmware);
        let base_image = extra.is_none();
        Self::start_with_image(firmware, cpus, extra.unwrap_or(image), base_image)
    }

    fn start_with_image(firmware: &str, cpus: usize, image: PathBuf, base_image: bool) -> Self {
        let serial = temporary_path("qmp-serial");
        let socket = temporary_path("qmp");
        let mut command = Command::new(qemu_binary());
        command
            .args([
                "-machine", "q35", "-cpu", "max", "-m", "128M", "-display", "none",
                "-monitor", "none", "-no-reboot", "-no-shutdown", "-qmp",
            ])
            .arg(format!("unix:{},server=on,wait=off", socket.display()))
            .arg("-serial")
            .arg(format!("file:{}", serial.display()))
            .arg("-smp")
            .arg(cpus.to_string())
            .arg("-drive")
            .arg(format!(
                "file={},format={},if=ide,readonly={}",
                image.display(),
                if base_image { "raw" } else { "qcow2" },
                if base_image { "on" } else { "off" }
            ));
        if let Some(accel) = std::env::var_os("GHOSTOS_QEMU_ACCEL") {
            command.arg("-accel").arg(accel);
        }
        if firmware == "uefi" {
            let firmware_path = std::env::var_os("GHOSTOS_QEMU_UEFI_FIRMWARE")
                .expect("set GHOSTOS_QEMU_UEFI_FIRMWARE for UEFI smoke tests");
            command.arg("-bios").arg(firmware_path);
        }
        let child = command.spawn().expect("start QEMU QMP session");
        let deadline = Instant::now() + TIMEOUT;
        let mut qmp = loop {
            match UnixStream::connect(&socket) {
                Ok(stream) => break stream,
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => panic!("connect QMP socket: {error}"),
            }
        };
        qmp.set_read_timeout(Some(Duration::from_secs(2))).expect("QMP read timeout");
        let _ = qmp_line(&mut qmp);
        assert_qmp_ok(&qmp_exec(&mut qmp, "{\"execute\":\"qmp_capabilities\"}"));
        Self { child, qmp, serial, socket }
    }

    fn command(&mut self, command: &str) -> String {
        qmp_exec(&mut self.qmp, &format!("{{\"execute\":\"{command}\"}}"))
    }

    fn human(&mut self, command: &str) -> String {
        let escaped = command.replace('\\', "\\\\").replace('"', "\\\"");
        qmp_exec(
            &mut self.qmp,
            &format!("{{\"execute\":\"human-monitor-command\",\"arguments\":{{\"command-line\":\"{escaped}\"}}}}"),
        )
    }

    fn send_key(&mut self, key: &str) -> String {
        qmp_exec(
            &mut self.qmp,
            &format!("{{\"execute\":\"send-key\",\"arguments\":{{\"keys\":[{{\"type\":\"qcode\",\"data\":\"{key}\"}}]}}}}"),
        )
    }

    fn finish(mut self, label: &str) -> String {
        let _ = self.child.wait();
        let log = fs::read_to_string(&self.serial).unwrap_or_default();
        if let Some(directory) = std::env::var_os("GHOSTOS_QEMU_LOG_DIR") {
            let directory = PathBuf::from(directory);
            fs::create_dir_all(&directory).expect("create QEMU evidence directory");
            fs::write(directory.join(format!("{label}.log")), &log).expect("write QEMU log");
        }
        let _ = fs::remove_file(&self.serial);
        let _ = fs::remove_file(&self.socket);
        log
    }
}

#[cfg(unix)]
impl Drop for QmpRun {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.serial);
        let _ = fs::remove_file(&self.socket);
    }
}

#[cfg(unix)]
fn qmp_line(stream: &mut UnixStream) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).expect("read QMP response");
        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            return String::from_utf8_lossy(&bytes).into_owned();
        }
    }
}

#[cfg(unix)]
fn qmp_exec(stream: &mut UnixStream, command: &str) -> String {
    stream.write_all(command.as_bytes()).expect("write QMP command");
    stream.write_all(b"\r\n").expect("finish QMP command");
    loop {
        let response = qmp_line(stream);
        if response.contains("\"return\"") || response.contains("\"error\"") {
            return response
        }
    }
}

#[cfg(unix)]
fn assert_qmp_ok(response: &str) {
    assert!(!response.contains("\"error\""), "QMP command failed: {response}");
}
