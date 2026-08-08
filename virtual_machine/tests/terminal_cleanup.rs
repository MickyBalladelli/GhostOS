//! Terminal I/O and exit-cleanup coverage.

use std::io::{self, Cursor, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use synos_vm::{
    serial_resize_sequence, TerminalInput, TerminalResize, TerminalSession,
    TerminalTranscriptEvent,
};

#[derive(Clone, Default)]
struct FakeOutput {
    bytes: Arc<Mutex<Vec<u8>>>,
    flushes: Arc<AtomicUsize>,
}

impl Write for FakeOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn poll_until_input(session: &TerminalSession) -> TerminalInput {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let input = session.poll().expect("fake terminal poll");
        if !input.bytes.is_empty() {
            return input;
        }
        assert!(Instant::now() < deadline, "fake input reader did not produce bytes");
        std::thread::yield_now();
    }
}

#[test]
fn fake_terminal_input_output_are_deterministic() {
    let output = FakeOutput::default();
    let output_state = output.clone();
    let session = TerminalSession::new_with_io(
        Cursor::new(vec![b'h', b'i', 0x7F, 0x1B, b'[', b'A']),
        output,
    );

    let input = poll_until_input(&session);
    let expected = [b'h', b'i', 0x08, 0x1B, b'[', b'A'];
    assert!(input.bytes.starts_with(&expected));
    assert!(input.bytes[expected.len()..].is_empty() || input.bytes[expected.len()..] == [0x04]);
    session
        .write_output(b"guest output")
        .expect("write fake terminal output");
    session.flush_output().expect("flush fake terminal output");
    assert_eq!(output_state.flushes.load(Ordering::Relaxed), 1);
    assert_eq!(&*output_state.bytes.lock().unwrap(), b"guest output");
}

#[test]
fn terminal_transcript_replays_policy_events_without_host_state() {
    let session = TerminalSession::new_with_io(
        Cursor::new(vec![b'a', 0x7F, 0x1B, b'[', b'D']),
        FakeOutput::default(),
    );
    let input = poll_until_input(&session);
    let expected = [b'a', 0x08, 0x1B, b'[', b'D'];
    assert!(input.bytes.starts_with(&expected));
    assert!(input.bytes[expected.len()..].is_empty() || input.bytes[expected.len()..] == [0x04]);

    let transcript = session.transcript();
    assert!(matches!(
        transcript.events().first(),
        Some(TerminalTranscriptEvent::Input { raw, bytes })
            if raw == &[b'a', 0x7F, 0x1B, b'[', b'D']
                && bytes == &[b'a', 0x08, 0x1B, b'[', b'D']
    ));
    assert_eq!(transcript.replay()[0].bytes, expected);
    assert_eq!(
        serial_resize_sequence(TerminalResize { rows: 24, columns: 80 }),
        b"\x1b[8;24;80t"
    );
}

#[test]
fn fake_terminal_eof_becomes_ctrl_d() {
    let session = TerminalSession::new_with_io(Cursor::new(Vec::<u8>::new()), FakeOutput::default());
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let input = session.poll().expect("fake EOF poll");
        if input.bytes.contains(&0x04) {
            assert_eq!(input.bytes, vec![0x04]);
            break;
        }
        assert!(Instant::now() < deadline, "fake input reader did not produce EOF");
        std::thread::yield_now();
    }
}

#[cfg(all(unix, any(target_os = "linux", target_os = "macos")))]
mod unix_pty {
    use super::*;
    use std::ffi::CStr;
    use std::fs::{File, OpenOptions};
    use std::mem::MaybeUninit;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::ptr;

    static PTY_TEST_LOCK: Mutex<()> = Mutex::new(());

    unsafe extern "C" {
        fn openpty(
            master: *mut libc::c_int,
            slave: *mut libc::c_int,
            name: *mut libc::c_char,
            termp: *mut libc::termios,
            winp: *mut libc::winsize,
        ) -> libc::c_int;
    }

    fn open_pty() -> (File, File, File) {
        let mut master = -1;
        let mut slave = -1;
        let mut name = [0 as libc::c_char; 128];
        let result = unsafe {
            openpty(
                &mut master,
                &mut slave,
                name.as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        assert_eq!(result, 0, "openpty failed: {}", io::Error::last_os_error());
        let slave_path = unsafe { CStr::from_ptr(name.as_ptr()) }
            .to_str()
            .expect("PTY path is UTF-8")
            .to_string();
        let probe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(slave_path)
            .expect("open PTY probe");
        (
            unsafe { File::from_raw_fd(master) },
            unsafe { File::from_raw_fd(slave) },
            probe,
        )
    }

    fn termios(file: &File, phase: &str) -> libc::termios {
        let mut value = MaybeUninit::uninit();
        let fd = file.as_raw_fd();
        let result = unsafe { libc::tcgetattr(fd, value.as_mut_ptr()) };
        assert_eq!(
            result,
            0,
            "tcgetattr failed during {phase} on fd {fd}, isatty={}, errno={}",
            unsafe { libc::isatty(fd) },
            io::Error::last_os_error()
        );
        unsafe { value.assume_init() }
    }

    fn termios_bytes(value: &libc::termios) -> Vec<u8> {
        unsafe {
            std::slice::from_raw_parts(
                value as *const libc::termios as *const u8,
                std::mem::size_of::<libc::termios>(),
            )
            .to_vec()
        }
    }

    fn set_nonblocking(file: &File) {
        let fd = file.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        assert!(flags >= 0, "fcntl get flags failed: {}", io::Error::last_os_error());
        let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        assert!(result >= 0, "fcntl set flags failed: {}", io::Error::last_os_error());
    }

    fn wait_for_ready(master: &mut File) {
        set_nonblocking(master);
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut output = Vec::new();
        let mut buffer = [0u8; 256];
        loop {
            match std::io::Read::read(master, &mut buffer) {
                Ok(count) => output.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("read child PTY output: {error}"),
            }
            if output.windows(b"TERMINAL_READY".len()).any(|window| window == b"TERMINAL_READY") {
                return;
            }
            assert!(Instant::now() < deadline, "terminal child did not become ready; output={output:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for_raw_mode(slave: &File, original: &[u8]) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if termios_bytes(&termios(slave, "raw-mode wait")) != original {
                return;
            }
            assert!(Instant::now() < deadline, "terminal child did not enter raw mode");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn spawn_child(action: &str, slave: &File) -> Child {
        let stdin = slave.try_clone().expect("clone PTY stdin");
        let stdout = slave.try_clone().expect("clone PTY stdout");
        let stderr = slave.try_clone().expect("clone PTY stderr");
        let mut command = Command::new(std::env::current_exe().expect("test executable"));
        command
            .arg("--exact")
            .arg("terminal_child_entrypoint")
            .arg("--nocapture")
            .env("SYNOS_TERMINAL_CHILD", action)
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        unsafe {
            command.pre_exec(|| {
                if libc::login_tty(0) < 0 {
                    return Err(io::Error::last_os_error())
                }
                Ok(())
            });
        }
        command.spawn().expect("spawn terminal child")
    }

    fn exercise_child(action: &str, cleanup_with_guard: bool) {
        let (mut master, slave, probe) = open_pty();
        let original = termios_bytes(&termios(&probe, "original"));
        let mut child = spawn_child(action, &slave);
        wait_for_ready(&mut master);
        wait_for_raw_mode(&probe, &original);

        if cleanup_with_guard {
            let mut cleanup = synos_test_support::CleanupGuard::new();
            let pid = child.id() as libc::pid_t;
            cleanup.defer("terminal-child", move || {
                let result = unsafe { libc::kill(pid, libc::SIGTERM) };
                if result < 0 && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
                    return Err(io::Error::last_os_error().to_string())
                }
                child
                    .wait()
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            });
            cleanup.cleanup().expect("terminal child cleanup");
        } else {
            let status = child.wait().expect("wait terminal child");
            if action == "success" || action == "close" {
                assert!(status.success(), "terminal child failed: {status}");
            } else {
                assert!(!status.success(), "terminal child unexpectedly succeeded");
            }
        }

        assert_eq!(
            termios_bytes(&termios(&probe, "restored")),
            original,
            "PTY settings leaked after {action}"
        );
        drop(master);
    }

    #[test]
    fn terminal_restores_pty_after_success_signal_panic_and_input_error() {
        let _lock = PTY_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for action in ["success", "signal", "panic", "close"] {
            exercise_child(action, false);
        }
    }

    #[test]
    fn terminal_cleanup_harness_reaps_child_and_restores_pty() {
        let _lock = PTY_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        exercise_child("hold", true);
    }
}

#[test]
fn terminal_child_entrypoint() {
    let Some(action) = std::env::var_os("SYNOS_TERMINAL_CHILD") else {
        return;
    };
    let action = action.to_string_lossy();
    let session = TerminalSession::new(Some(true)).expect("child terminal session");
    print!("TERMINAL_READY\n");
    std::io::stdout().flush().expect("child terminal flush");

    match action.as_ref() {
        "success" => drop(session),
        "signal" => {
            #[cfg(unix)]
            unsafe {
                libc::raise(libc::SIGTERM);
            }
            #[cfg(not(unix))]
            drop(session);
        }
        "panic" => panic!("terminal cleanup panic"),
        "close" => {
            unsafe {
                libc::close(0);
            }
            loop {
                match session.poll() {
                    Ok(input) if input.bytes.contains(&0x04) => break,
                    Ok(_) => std::thread::sleep(Duration::from_millis(5)),
                    Err(_) => break,
                }
            }
        }
        "hold" => loop {
            std::thread::sleep(Duration::from_secs(1));
        },
        other => panic!("unknown terminal child action {other}"),
    }
}
