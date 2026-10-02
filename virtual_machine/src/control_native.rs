//! Owned Rust strings and host adapters for the C monitor protocol.
use super::{MonitorCommand, MonitorPermission, MonitorPermissions, MonitorRequestError,
    MonitorRequestFrame, MonitorTopic, MONITOR_AUTH_DOMAIN};
use std::ffi::c_void;
use std::path::PathBuf;

#[repr(C)]
pub(super) struct Buffer { bytes: [u8; 4096], length: usize }
impl Buffer {
    pub fn new() -> Self { Self { bytes: [0; 4096], length: 0 } }
    pub fn push(&mut self, bytes: &[u8]) -> MonitorRequestFrame {
        let mut length = 0;
        let code = unsafe { ghostos_monitor_push(self, bytes.as_ptr(), bytes.len(), &mut length) };
        self.frame_result(code, length)
    }
    pub fn end_of_stream(&self) -> MonitorRequestFrame {
        let mut length = 0;
        let code = unsafe { ghostos_monitor_frame(self, true, &mut length) };
        self.frame_result(code, length)
    }
    fn frame_result(&self, code: u32, length: usize) -> MonitorRequestFrame {
        let (code, message) = match code {
            0 => return MonitorRequestFrame::Pending,
            1 => return MonitorRequestFrame::Complete(String::from_utf8(self.bytes[..length].to_vec()).expect("C checked UTF-8")),
            2 => ("request-too-large", "monitor request exceeds the 4096 byte limit"),
            3 => ("partial-command", "monitor connection closed before a complete command line"),
            4 => ("multiple-commands", "monitor connection accepts exactly one command"),
            5 => ("invalid-encoding", "monitor command is not valid UTF-8"),
            _ => unreachable!("C monitor framing result"),
        };
        MonitorRequestFrame::Rejected { code, message: message.to_string() }
    }
}

#[repr(C)]
#[derive(Default)]
struct Command { kind: u32, topic: u32, sensitive: bool, path_start: usize, path_length: usize }
impl Command {
    fn into_command(self, input: &str) -> MonitorCommand {
        match self.kind {
            0 => MonitorCommand::Help,
            1 => MonitorCommand::Info {
                topic: match self.topic { 0 => MonitorTopic::Status, 1 => MonitorTopic::Devices,
                    2 => MonitorTopic::Disks, 3 => MonitorTopic::Snapshots, 4 => MonitorTopic::Migration,
                    5 => MonitorTopic::Registers, _ => unreachable!("C monitor topic") },
                disclose_sensitive: self.sensitive,
            },
            2 => MonitorCommand::SaveSnapshot(PathBuf::from(&input[self.path_start..self.path_start + self.path_length])),
            3 => MonitorCommand::Quit,
            _ => unreachable!("C monitor command kind"),
        }
    }
}

fn parse_error(code: u32) -> String {
    match code {
        1 => "monitor command exceeds the 2048 byte limit",
        2 => "save needs a snapshot PATH",
        3 => "unknown monitor command",
        _ => unreachable!("Rust supplied a valid UTF-8 command"),
    }.to_string()
}

pub(super) fn parse(input: &str) -> Result<MonitorCommand, String> {
    let mut command = Command::default();
    let code = unsafe { ghostos_monitor_parse(input.as_ptr(), input.len(), &mut command) };
    if code == 0 { Ok(command.into_command(input)) } else { Err(parse_error(code)) }
}

pub(super) fn permissions(input: &str) -> Result<MonitorPermissions, String> {
    let mut bits = 0;
    let mut start = 0;
    let mut length = 0;
    let code = unsafe { ghostos_monitor_permissions(input.as_ptr(), input.len(), &mut bits, &mut start, &mut length) };
    match code {
        0 => Ok(MonitorPermissions(bits)),
        1 => Err("monitor permission list cannot be empty".to_string()),
        2 => Err(format!("invalid monitor permission `{}`; use status, device, disk, migration, save, quit, sensitive, or all", &input[start..start + length])),
        _ => unreachable!("Rust supplied valid UTF-8 permissions"),
    }
}

fn render(mut operation: impl FnMut(*mut u8, usize) -> usize) -> String {
    let length = operation(std::ptr::null_mut(), 0);
    let mut output = vec![0; length];
    assert_eq!(operation(output.as_mut_ptr(), output.len()), length);
    String::from_utf8(output).expect("C monitor responses preserve UTF-8")
}

pub(super) fn json_string(input: &str) -> String {
    render(|output, capacity| unsafe { ghostos_monitor_json_string(input.as_ptr(), input.len(), output, capacity) })
}
pub(super) fn envelope(command: &str, data: &str) -> String {
    render(|output, capacity| unsafe { ghostos_monitor_envelope(command.as_ptr(), command.len(), data.as_ptr(), data.len(), output, capacity) })
}
pub(super) fn failure(command: Option<&str>, code: &str, message: &str) -> String {
    let name = command.unwrap_or("");
    render(|output, capacity| unsafe { ghostos_monitor_failure(command.is_some(), name.as_ptr(), name.len(),
        code.as_ptr(), code.len(), message.as_ptr(), message.len(), output, capacity) })
}
pub(super) fn action(command: &str, action: &str) -> String {
    render(|output, capacity| unsafe { ghostos_monitor_action(command.as_ptr(), command.len(), action.as_ptr(), action.len(), output, capacity) })
}
pub(super) fn help() -> String { render(|output, capacity| unsafe { ghostos_monitor_help(output, capacity) }) }

pub(super) fn response_fits(length: usize) -> bool { unsafe { ghostos_monitor_response_fits(length) } }

#[repr(C)]
struct AuthIo {
    now: unsafe extern "C" fn(*mut c_void, *mut u64) -> bool,
    verify: unsafe extern "C" fn(*mut c_void, u64, *const u8, *const u8, usize, *const u8) -> bool,
    context: *mut c_void,
}
#[repr(C)]
#[derive(Default)]
struct AuthResult { command_start: usize, command_length: usize, command: Command, parse_error: u32, missing_permission: u8 }
struct AuthContext { key: ghostos_vm::SnapshotAuthKey }

unsafe extern "C" fn now(_: *mut c_void, output: *mut u64) -> bool {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(now) => { unsafe { *output = now.as_secs(); } true },
        Err(_) => false,
    }
}
unsafe extern "C" fn verify(raw: *mut c_void, timestamp: u64, nonce: *const u8,
    command: *const u8, length: usize, tag: *const u8) -> bool {
    let context = unsafe { &*raw.cast::<AuthContext>() };
    // C supplies fixed-size nonce/tag arrays and a borrowed command slice;
    // SnapshotAuthKey delegates HMAC and constant-time comparison to C.
    let nonce = unsafe { std::slice::from_raw_parts(nonce, 32) };
    let command = unsafe { std::slice::from_raw_parts(command, length) };
    let tag = unsafe { &*tag.cast::<[u8; 32]>() };
    context.key.verify_parts(&[MONITOR_AUTH_DOMAIN, &timestamp.to_le_bytes(), nonce, command], tag).is_ok()
}

pub(super) struct Nonces { raw: *mut c_void }
// C only mutates the cache during an exclusive authenticate borrow. No native
// pointers or references are exposed, and the cache owns no thread-bound data.
unsafe impl Send for Nonces {}
unsafe impl Sync for Nonces {}
impl Nonces {
    pub fn new() -> Self {
        let raw = unsafe { ghostos_monitor_nonces_new() };
        if raw.is_null() { std::alloc::handle_alloc_error(std::alloc::Layout::new::<[u8; 32784]>()); }
        Self { raw }
    }
    pub fn authenticate(&mut self, key: ghostos_vm::SnapshotAuthKey,
        permissions: MonitorPermissions, input: &str) -> Result<MonitorCommand, MonitorRequestError> {
        let mut context = AuthContext { key };
        let io = AuthIo { now, verify, context: (&mut context as *mut AuthContext).cast() };
        let mut output = AuthResult::default();
        let code = unsafe { ghostos_monitor_authenticate(self.raw, permissions.0, input.as_ptr(), input.len(), &io, &mut output) };
        if code == 0 {
            let command = &input[output.command_start..output.command_start + output.command_length];
            return Ok(output.command.into_command(command));
        }
        if code == 16 {
            return Err(MonitorRequestError { code: "invalid-command",
                command: Some(input[output.command_start..output.command_start + output.command_length].to_string()),
                message: parse_error(output.parse_error) });
        }
        if code == 17 || code == 18 {
            let command = &input[output.command_start..output.command_start + output.command_length];
            let parsed = output.command.into_command(command);
            let permission = match output.missing_permission { 1 => MonitorPermission::Status, 2 => MonitorPermission::Device,
                4 => MonitorPermission::Disk, 8 => MonitorPermission::Migration, 16 => MonitorPermission::Save,
                32 => MonitorPermission::Quit, _ if code == 18 => MonitorPermission::Sensitive,
                _ => unreachable!("C monitor permission") };
            return Err(MonitorRequestError { code: "permission-denied", command: Some(parsed.name().to_string()),
                message: format!("monitor client lacks `{}` permission", permission.name()) });
        }
        let (code, message) = match code {
            1 => ("authentication-required", "monitor request needs authentication"),
            2 => ("authentication-required", "monitor request must start with `auth`"),
            3 => ("invalid-authentication", "authenticated request needs a timestamp"),
            4 => ("invalid-authentication", "authenticated request needs a nonce"),
            5 => ("invalid-authentication", "authenticated request needs a tag and command"),
            6 => ("invalid-authentication", "authenticated request needs a command"),
            7 => ("invalid-authentication", "monitor timestamp is invalid"),
            8 => ("authentication-failed", "host clock is before the Unix epoch"),
            9 => ("authentication-expired", "monitor request timestamp is outside the five-minute window"),
            10 => ("invalid-authentication", "monitor nonce must be 64 hexadecimal characters"),
            11 => ("invalid-authentication", "monitor nonce is not hexadecimal"),
            12 => ("invalid-authentication", "monitor authentication tag must be 64 hexadecimal characters"),
            13 => ("invalid-authentication", "monitor authentication tag is not hexadecimal"),
            14 => ("authentication-replay", "monitor nonce was already used"),
            15 => ("authentication-failed", "monitor authentication failed"),
            _ => unreachable!("C monitor authentication result"),
        };
        Err(MonitorRequestError { code, command: None, message: message.to_string() })
    }
}
impl Drop for Nonces { fn drop(&mut self) { unsafe { ghostos_monitor_nonces_free(self.raw); } } }

unsafe extern "C" {
    fn ghostos_monitor_push(state: *mut Buffer, bytes: *const u8, length: usize, command_length: *mut usize) -> u32;
    fn ghostos_monitor_frame(state: *const Buffer, eof: bool, command_length: *mut usize) -> u32;
    fn ghostos_monitor_parse(input: *const u8, length: usize, command: *mut Command) -> u32;
    fn ghostos_monitor_permissions(input: *const u8, length: usize, permissions: *mut u8, start: *mut usize, size: *mut usize) -> u32;
    fn ghostos_monitor_json_string(input: *const u8, length: usize, output: *mut u8, capacity: usize) -> usize;
    fn ghostos_monitor_envelope(command: *const u8, command_length: usize, data: *const u8, data_length: usize, output: *mut u8, capacity: usize) -> usize;
    fn ghostos_monitor_failure(has_command: bool, command: *const u8, command_length: usize,
        code: *const u8, code_length: usize, message: *const u8, message_length: usize, output: *mut u8, capacity: usize) -> usize;
    fn ghostos_monitor_action(command: *const u8, command_length: usize, action: *const u8, action_length: usize, output: *mut u8, capacity: usize) -> usize;
    fn ghostos_monitor_help(output: *mut u8, capacity: usize) -> usize;
    fn ghostos_monitor_response_fits(length: usize) -> bool;
    fn ghostos_monitor_nonces_new() -> *mut c_void;
    fn ghostos_monitor_nonces_free(state: *mut c_void);
    fn ghostos_monitor_authenticate(state: *mut c_void, permissions: u8, input: *const u8, length: usize,
        io: *const AuthIo, output: *mut AuthResult) -> u32;
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(super::MAX_MONITOR_REQUEST_BYTES == 4096);
    assert!(super::MAX_MONITOR_COMMAND_BYTES == 2048);
    assert!(std::mem::size_of::<Buffer>() == 4104);
    assert!(std::mem::size_of::<Command>() == 32);
    assert!(std::mem::size_of::<AuthResult>() == 56);
};
