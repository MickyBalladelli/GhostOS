//! Opt-in QEMU end-to-end coverage for the interactive local login flow.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::net::UnixStream;

use ghostos_vm::devices::{SystemDiskInstall, SystemDiskProvisioner};

const SERIAL_TIMEOUT: Duration = Duration::from_secs(15);

#[test]
#[ignore = "requires GHOSTOS_RUN_QEMU_TESTS=1, QEMU, and GHOSTOS_QEMU_IMAGE"]
#[cfg(unix)]
fn qemu_interactive_login_flow() {
    if std::env::var_os("GHOSTOS_RUN_QEMU_TESTS").is_none() {
        return
    }

    let source_image = qemu_image();
    assert!(source_image.is_file(), "missing QEMU image: {}", source_image.display());
    let writable_image = temporary_path("login-image");
    fs::copy(&source_image, &writable_image).expect("copy QEMU image for login workflow");
    let system_disk = temporary_path("login-system-disk");
    let kernel_payload = temporary_path("login-kernel");
    provision_login_system_disk(&system_disk, &kernel_payload);

    let mut session = QemuLoginSession::start(&writable_image, &system_disk, &kernel_payload);
    let result = drive_login_workflow(&mut session);
    let log = session.finish("qemu-interactive-login");
    result.unwrap_or_else(|error| panic!("login workflow failed: {error}; serial output: {log:?}"));
}

fn provision_login_system_disk(system_disk: &Path, kernel_payload: &Path) {
    fs::write(kernel_payload, b"GhostOS login E2E kernel payload")
        .expect("write login system-disk kernel payload");
    let service_image = kernel_build_output("ghostos-service.bin");
    let shell_image = kernel_build_output("ghostos-shell.bin");
    let login_image = kernel_build_output("ghostos-login.bin");
    let mut install = SystemDiskInstall::new(kernel_payload)
        .with_boot_args("console=serial0")
        .with_machine_identity("qemu-login-e2e")
        .with_network_identity("qemu-login-e2e");
    for role in 1..=14u8 {
        let image = if role == 9 {
            &shell_image
        } else if role == 14 {
            &login_image
        } else {
            &service_image
        };
        install = install.with_service_package(role, image.clone());
    }
    SystemDiskProvisioner::provision(system_disk, &install)
        .expect("provision login system disk");
}

fn kernel_build_output(name: &str) -> PathBuf {
    let build_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target");
    for profile in ["debug", "release"] {
        let build_dir = build_root.join(profile).join("build");
        let Ok(entries) = fs::read_dir(&build_dir) else {
            continue
        };
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().starts_with("ghostos-kernel-") {
                continue
            }
            let output = entry.path().join("out").join(name);
            if output.is_file() {
                return output
            }
        }
    }
    panic!("could not find kernel build output {name:?}")
}

#[cfg(unix)]
fn drive_login_workflow(session: &mut QemuLoginSession) -> Result<(), String> {
    session.wait_for("No administrator account exists.")?;
    session.wait_for("GhostOS first-run setup mode")?;
    session.wait_for("Administrator username: ")?;
    session.send_text("admin\n")?;
    session.wait_for("Credential type [PASSKEY/TPM/SSH] (PASSKEY): ")?;
    session.send_text("passkey\n")?;
    session.wait_for("Public credential material: ")?;
    session.send_text("fixture\n")?;
    session.wait_for("Create this administrator account? [y/N]: ")?;
    let setup_start = session.serial_len();
    session.send_text("y\n")?;
    session.wait_for_after("Administrator account committed.", setup_start)?;

    complete_login(session, "first login", setup_start)?;
    assert_whoami(session, "first login")?;

    let logout_start = session.serial_len();
    session.send_text("logout\n")?;
    session.wait_for_after("GHOSTOS\x1b[90m::\x1b[31mLOCKED", logout_start)?;
    complete_login(session, "normal login", logout_start)?;
    assert_whoami(session, "normal login")?;

    let logout_start = session.serial_len();
    session.send_text("logout\n")?;
    session.wait_for_after("GHOSTOS\x1b[90m::\x1b[31mLOCKED", logout_start)?;
    complete_login(session, "relogin", logout_start)?;
    assert_whoami(session, "relogin")?;
    exercise_login_failures_and_lockout(session)?;
    Ok(())
}

#[cfg(unix)]
fn exercise_login_failures_and_lockout(session: &mut QemuLoginSession) -> Result<(), String> {
    let first_failure_start = session.serial_len();
    session.send_text("logout\n")?;
    session.wait_for_after("GHOSTOS\x1b[90m::\x1b[31mLOCKED", first_failure_start)?;
    submit_bad_credential(session, first_failure_start)?;

    let rate_limited_start = session.serial_len();
    session.wait_for_after("login: ", rate_limited_start)?;
    session.send_text("admin\npasskey\n")?;
    let rate_limited_log = session.wait_for_after("Login failed", rate_limited_start)?;
    if rate_limited_log[rate_limited_start..].contains("challenge: ") {
        return Err("rate-limited login unexpectedly received a challenge".to_string())
    }

    let mut next_attempt_start = session.serial_len();
    for delay_ms in [1_100, 2_100, 4_100, 8_100] {
        session.wait_for_after("login: ", next_attempt_start)?;
        std::thread::sleep(Duration::from_millis(delay_ms));
        submit_bad_credential(session, next_attempt_start)?;
        next_attempt_start = session.serial_len();
    }
    session.wait_for_after("Login temporarily locked after repeated failures.", next_attempt_start)?;
    Ok(())
}

#[cfg(unix)]
fn submit_bad_credential(session: &mut QemuLoginSession, search_from: usize) -> Result<(), String> {
    session.wait_for_after("login: ", search_from)?;
    session.send_text("admin\npasskey\n")?;
    session.wait_for_after("passkey: ", search_from)?;
    session.send_text("00\n")?;
    session.wait_for_after("Login failed", search_from)?;
    Ok(())
}

#[cfg(unix)]
fn complete_login(
    session: &mut QemuLoginSession,
    label: &str,
    search_from: usize,
) -> Result<(), String> {
    session.wait_for_after("login: ", search_from)?;
    session.send_text("admin\npasskey\n")?;
    let log = session.wait_for_after("passkey: ", search_from)?;
    let challenge = challenge_from_log(&log).ok_or_else(|| {
        format!("{label}: login challenge was not present in serial output")
    })?;
    session.send_text(&format!("{}\n", assertion_for_challenge(&challenge)))?;
    session.wait_for("Login accepted.")?;
    Ok(())
}

#[cfg(unix)]
fn assert_whoami(session: &mut QemuLoginSession, label: &str) -> Result<(), String> {
    let search_from = session.serial_len();
    session.send_text("whoami\n")?;
    let log = session.wait_for_after("$ whoami\nadmin\n", search_from)?;
    if !log.contains("$ whoami\nadmin\n") {
        return Err(format!("{label}: WHOAMI did not return admin"));
    }
    Ok(())
}

fn challenge_from_log(log: &str) -> Option<Vec<u8>> {
    let marker = "challenge: ";
    let start = log.rfind(marker)? + marker.len();
    let end = log[start..].find('\n').map_or(log.len(), |offset| start + offset);
    let encoded = log[start..end].trim();
    if encoded.len() != 64 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None
    }
    let mut challenge = Vec::with_capacity(32);
    for pair in encoded.as_bytes().chunks_exact(2) {
        challenge.push((hex_value(pair[0])? << 4) | hex_value(pair[1])?);
    }
    Some(challenge)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn assertion_for_challenge(challenge: &[u8]) -> String {
    assert_eq!(challenge.len(), 32);
    let mut assertion = Vec::with_capacity(85);
    assertion.extend_from_slice(b"SYPA");
    assertion.extend_from_slice(&[1, 0, 0, 0]);
    assertion.extend_from_slice(challenge);
    assertion.extend_from_slice(&(37u16).to_le_bytes());
    assertion.extend_from_slice(&(1u16).to_le_bytes());
    assertion.extend_from_slice(&(1u16).to_le_bytes());
    assertion.extend_from_slice(&[0; 32]);
    assertion.push(0x05);
    assertion.extend_from_slice(&[0; 4]);
    assertion.push(0);
    assertion.push(0);
    assertion
        .iter()
        .flat_map(|byte| [hex_digit(byte >> 4), hex_digit(byte & 0x0f)])
        .collect()
}

fn hex_digit(value: u8) -> char {
    b"0123456789abcdef"[value as usize] as char
}

fn qemu_binary() -> String {
    std::env::var("GHOSTOS_QEMU_BIN").unwrap_or_else(|_| "qemu-system-x86_64".to_string())
}

fn qemu_image() -> PathBuf {
    std::env::var_os("GHOSTOS_QEMU_IMAGE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../build/bios/ghostos-bios.img"))
}

fn temporary_path(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("ghostos-qemu-{name}-{}-{stamp}", std::process::id()))
}

#[cfg(unix)]
struct QemuLoginSession {
    child: Child,
    qmp: UnixStream,
    serial: PathBuf,
    socket: PathBuf,
    image: PathBuf,
    system_disk: PathBuf,
    kernel_payload: PathBuf,
}

#[cfg(unix)]
impl QemuLoginSession {
    fn start(image: &Path, system_disk: &Path, kernel_payload: &Path) -> Self {
        let serial = temporary_path("login-serial");
        let socket = temporary_path("login-qmp");
        let mut command = Command::new(qemu_binary());
        command
            .args([
                "-machine", "q35", "-cpu", "max", "-m", "128M", "-display", "none",
                "-monitor", "none", "-no-reboot", "-no-shutdown", "-qmp",
            ])
            .arg(format!("unix:{},server=on,wait=off", socket.display()))
            .arg("-serial")
            .arg(format!("file:{}", serial.display()))
            .arg("-drive")
            .arg(format!("file={},format=raw,if=ide,index=0", image.display()))
            .arg("-drive")
            .arg(format!(
                "file={},format=raw,if=ide,index=1",
                system_disk.display()
            ));
        if let Some(accel) = std::env::var_os("GHOSTOS_QEMU_ACCEL") {
            command.arg("-accel").arg(accel);
        }
        let child = command.spawn().expect("start QEMU login session");
        let deadline = Instant::now() + SERIAL_TIMEOUT;
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
        qmp.set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set QMP read timeout");
        qmp_line(&mut qmp);
        assert_qmp_ok(&qmp_exec(&mut qmp, "{\"execute\":\"qmp_capabilities\"}"));
        Self {
            child,
            qmp,
            serial,
            socket,
            image: image.to_path_buf(),
            system_disk: system_disk.to_path_buf(),
            kernel_payload: kernel_payload.to_path_buf(),
        }
    }

    fn wait_for(&self, marker: &str) -> Result<String, String> {
        self.wait_for_after(marker, 0)
    }

    fn serial_len(&self) -> usize {
        fs::metadata(&self.serial)
            .map(|metadata| metadata.len() as usize)
            .unwrap_or(0)
    }

    fn wait_for_after(&self, marker: &str, search_from: usize) -> Result<String, String> {
        let deadline = Instant::now() + SERIAL_TIMEOUT;
        loop {
            let log = fs::read_to_string(&self.serial).unwrap_or_default();
            if search_from <= log.len() && log[search_from..].contains(marker) {
                return Ok(log)
            }
            if Instant::now() >= deadline {
                return Err(format!("timed out waiting for {marker:?}"))
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn send_text(&mut self, text: &str) -> Result<(), String> {
        for byte in text.bytes() {
            let key = match byte {
                b'\n' | b'\r' => "ret".to_string(),
                b'a'..=b'z' | b'0'..=b'9' => (byte as char).to_string(),
                b' ' => "spc".to_string(),
                _ => return Err(format!("unsupported QEMU key byte 0x{byte:02x}")),
            };
            let command = format!(
                "{{\"execute\":\"human-monitor-command\",\"arguments\":{{\"command-line\":\"sendkey {key}\"}}}}"
            );
            assert_qmp_ok(&qmp_exec(&mut self.qmp, &command));
        }
        Ok(())
    }

    fn finish(&mut self, label: &str) -> String {
        let _ = qmp_exec(&mut self.qmp, "{\"execute\":\"quit\"}");
        let _ = self.child.wait();
        let log = fs::read_to_string(&self.serial).unwrap_or_default();
        if let Some(directory) = std::env::var_os("GHOSTOS_QEMU_LOG_DIR") {
            let directory = PathBuf::from(directory);
            fs::create_dir_all(&directory).expect("create QEMU evidence directory");
            fs::write(directory.join(format!("{label}.log")), &log)
                .expect("write QEMU login evidence");
        }
        log
    }
}

#[cfg(unix)]
impl Drop for QemuLoginSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.serial);
        let _ = fs::remove_file(&self.socket);
        let _ = fs::remove_file(&self.image);
        let _ = fs::remove_file(&self.system_disk);
        let _ = fs::remove_file(&self.kernel_payload);
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
            return String::from_utf8_lossy(&bytes).into_owned()
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

#[cfg(not(unix))]
struct QemuLoginSession;

#[cfg(not(unix))]
impl QemuLoginSession {
    fn start(_image: &Path) -> Self {
        panic!("QEMU login workflow requires Unix QMP sockets")
    }
}
